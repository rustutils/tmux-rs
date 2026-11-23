//! calloop-backed event loop implementation.
//!
//! Provides safe RAII wrappers (`TimerHandle`, `SignalHandle`, `IoHandle`,
//! `defer`) and the core `event_init` / `event_loop` dispatch.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::{c_int, c_short};
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::time::Duration;

// ---------------------------------------------------------------------------
// Safe timer API
// ---------------------------------------------------------------------------

/// RAII handle for a registered timer.  Dropping it deregisters the timer
/// from the event loop automatically.
pub struct TimerHandle {
    id: u64,
}

impl Drop for TimerHandle {
    fn drop(&mut self) {
        deregister(self.id);
    }
}

// ---------------------------------------------------------------------------
// Safe I/O event API
// ---------------------------------------------------------------------------

/// RAII handle for a registered I/O event.  Dropping it deregisters the
/// fd from the event loop.
pub struct IoHandle {
    id: u64,
}

impl Drop for IoHandle {
    fn drop(&mut self) {
        deregister(self.id);
    }
}

/// Register an I/O event that calls `callback` when the fd is readable
/// and/or writable.  The callback receives `(fd, fired_events)`.
///
/// `events` should be a combination of `EV_READ` and `EV_WRITE`.
/// `EV_PERSIST` is always implied — the registration stays active until
/// dropped.
///
/// Returns `Some(IoHandle)` on success.  Drop the handle to deregister.
pub fn io_register(
    fd: c_int,
    events: c_short,
    callback: Box<dyn Fn(c_int, c_short)>,
) -> Option<IoHandle> {
    if fd < 0 {
        return None;
    }
    let interest = match (
        (events & super::super::EV_READ) != 0,
        (events & super::super::EV_WRITE) != 0,
    ) {
        (true, true) => Interest::BOTH,
        (true, false) => Interest::READ,
        (false, true) => Interest::WRITE,
        _ => return None,
    };

    // Dup the fd so calloop gets its own epoll registration.
    // SAFETY: dup only reads the fd number. A bad fd makes it fail.
    let dup_fd = unsafe { libc::dup(fd) };
    if dup_fd < 0 {
        return None;
    }
    // SAFETY: dup returned a new fd that nothing else owns.
    let owned_fd = unsafe { OwnedFd::from_raw_fd(dup_fd) };
    let generic = Generic::new(owned_fd, interest, Mode::Level);

    with_registry(|base| {
        let id = base.alloc_id();
        match base
            .handle
            .insert_source(generic, move |readiness, _, data| {
                let mut fired: c_short = 0;
                if readiness.readable {
                    fired |= super::super::EV_READ;
                }
                if readiness.writable {
                    fired |= super::super::EV_WRITE;
                }
                if fired != 0 {
                    data.ready.push(ReadyEvent {
                        id,
                        fd,
                        events: fired,
                    });
                }
                Ok(PostAction::Continue)
            }) {
            Ok(token) => {
                base.registrations.insert(id, Registration { token });
                base.io_callbacks.insert(id, callback);
                Some(IoHandle { id })
            }
            Err(_) => {
                // Non-epolable fd (regular file, etc.) — treat as always-ready.
                // The callback fires on every event_loop iteration.
                base.fallback_io.insert(
                    id,
                    (
                        fd,
                        events & (super::super::EV_READ | super::super::EV_WRITE),
                    ),
                );
                base.io_callbacks.insert(id, callback);
                Some(IoHandle { id })
            }
        }
    })
    .flatten()
}

// ---------------------------------------------------------------------------
// Safe deferred callback API
// ---------------------------------------------------------------------------

/// Defer a callback to fire on the next event loop iteration.
///
/// Used for resource cleanup where the callback needs to run *after* the
/// current call stack unwinds — e.g. `server_client_free` must not run
/// while we're still inside a callback that references the client.
///
/// The callback is `FnOnce` — it fires exactly once, then is dropped.
/// Unlike `timer_add`, this returns no handle: the caller cannot cancel
/// the deferred work.
pub fn defer(callback: Box<dyn FnOnce()>) {
    // If the loop is not initialised the callback is dropped, after the
    // registry borrow has ended.
    let _ = with_registry(move |base| {
        let id = base.alloc_id();

        let timer = Timer::immediate();
        let res = base.handle.insert_source(timer, move |_, (), data| {
            data.ready.push(ReadyEvent {
                id,
                fd: -1,
                events: super::super::EV_TIMEOUT,
            });
            TimeoutAction::Drop
        });
        if let Ok(token) = res {
            base.registrations.insert(id, Registration { token });
            base.deferred_callbacks.insert(id, Some(callback));
            None
        } else {
            Some(callback)
        }
    });
}

// ---------------------------------------------------------------------------
// Safe signal API
// ---------------------------------------------------------------------------

/// RAII handle for a registered signal.  Dropping it deregisters the signal
/// handler from the event loop automatically.
pub struct SignalHandle {
    id: u64,
    /// On non-Linux platforms, holds the signal-hook registration ID so we
    /// can unregister the signal handler on drop.
    #[cfg(not(target_os = "linux"))]
    sig_id: signal_hook::SigId,
}

impl Drop for SignalHandle {
    fn drop(&mut self) {
        deregister(self.id);
        #[cfg(not(target_os = "linux"))]
        signal_hook::low_level::unregister(self.sig_id);
    }
}

/// Register a signal handler that calls `callback` when `signum` is delivered.
///
/// Returns `Some(SignalHandle)` on success.  Drop the handle to deregister.
///
/// On Linux this uses `signalfd(2)` via calloop's built-in `Signals` source.
/// On other platforms (macOS, OpenBSD) it uses a self-pipe with `signal-hook`.
#[cfg(target_os = "linux")]
pub fn signal_register(signum: c_int, callback: Box<dyn Fn()>) -> Option<SignalHandle> {
    let signal = signal_from_number(signum)?;
    let signals = Signals::new(&[signal]).ok()?;
    // Moved into the registry only on success, otherwise dropped after the
    // registry borrow has ended.
    let mut callback = Some(callback);
    with_registry(|base| {
        let id = base.alloc_id();
        let token = base
            .handle
            .insert_source(signals, move |_, (), data| {
                data.ready.push(ReadyEvent {
                    id,
                    fd: signum,
                    events: super::super::EV_SIGNAL,
                });
            })
            .ok()?;

        base.registrations.insert(id, Registration { token });
        base.timer_callbacks.insert(id, callback.take()?);
        Some(SignalHandle { id })
    })
    .flatten()
}

/// Self-pipe signal registration for non-Linux platforms.
#[cfg(not(target_os = "linux"))]
pub fn signal_register(signum: c_int, callback: Box<dyn Fn()>) -> Option<SignalHandle> {
    // Create a non-blocking pipe.
    let (read_fd, write_fd) = pipe_nonblock().ok()?;

    // Register signal-hook to write a byte on signal delivery.
    let sig_id = signal_hook::low_level::pipe::register(signum, write_fd).ok()?;

    // Register the read end with calloop.
    let generic = Generic::new(read_fd, Interest::READ, Mode::Level);
    let mut callback = Some(callback);
    with_registry(|base| {
        let id = base.alloc_id();
        let token = base
            .handle
            .insert_source(generic, move |_, fd, data| {
                // Drain the pipe so it doesn't keep firing.
                let mut buf = [0u8; 64];
                loop {
                    match rustix::io::read(&mut *fd, &mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {}
                    }
                }
                data.ready.push(ReadyEvent {
                    id,
                    fd: signum,
                    events: super::super::EV_SIGNAL,
                });
                Ok(PostAction::Continue)
            })
            .ok()?;

        base.registrations.insert(id, Registration { token });
        base.timer_callbacks.insert(id, callback.take()?);
        Some(SignalHandle { id, sig_id })
    })
    .flatten()
}

// ---------------------------------------------------------------------------
// Safe timer API (continued)
// ---------------------------------------------------------------------------

/// Register a one-shot timer that fires after `duration` and calls `callback`.
///
/// Returns `Some(TimerHandle)` on success.  Drop the handle to cancel.
pub fn timer_add(duration: Duration, callback: Box<dyn Fn()>) -> Option<TimerHandle> {
    let timer = Timer::from_duration(duration);
    // Moved into the registry only on success, otherwise dropped after the
    // registry borrow has ended.
    let mut callback = Some(callback);
    with_registry(|base| {
        let id = base.alloc_id();
        let token = base
            .handle
            .insert_source(timer, move |_, (), data| {
                data.ready.push(ReadyEvent {
                    id,
                    fd: -1,
                    events: super::super::EV_TIMEOUT,
                });
                TimeoutAction::Drop
            })
            .ok()?;

        base.registrations.insert(id, Registration { token });
        base.timer_callbacks.insert(id, callback.take()?);
        Some(TimerHandle { id })
    })
    .flatten()
}

use calloop::generic::Generic;
#[cfg(target_os = "linux")]
use calloop::signals::{Signal, Signals};
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, Interest, LoopHandle, Mode, PostAction, RegistrationToken};
use tmux_log::log_debug;

use super::super::{EV_READ, EV_SIGNAL, EV_TIMEOUT, EV_WRITE, EVLOOP_NONBLOCK, event_log_cb};
use super::event_base;

/// A ready event notification — queued when a calloop source fires.
struct ReadyEvent {
    id: u64,
    fd: c_int,
    events: c_short,
}

/// Registration entry tracked per event id.
struct Registration {
    token: RegistrationToken,
}

/// Shared state passed to calloop callbacks and the dispatch loop.
struct LoopData {
    /// Events ready to be dispatched after calloop returns.
    ready: Vec<ReadyEvent>,
}

/// Registrations and callbacks of the event loop.
///
/// Kept apart from the calloop `EventLoop` itself, so callbacks can
/// register and drop events while the loop is borrowed for dispatch.
struct Registry {
    handle: LoopHandle<'static, LoopData>,
    registrations: HashMap<u64, Registration>,
    /// Non-epolable fds (regular files, etc.) that are always ready.
    /// Maps event id → (fd, events).  The actual callback is in
    /// `io_callbacks`; these fire on every `event_loop` iteration.
    fallback_io: HashMap<u64, (c_int, c_short)>,
    /// Closures for safe timer API (`timer_add`).  Keyed by event id.
    timer_callbacks: HashMap<u64, Box<dyn Fn()>>,
    /// Closures for safe I/O API (`io_register`).  Keyed by event id.
    /// Takes `(fd, fired_events)`.
    io_callbacks: HashMap<u64, Box<dyn Fn(c_int, c_short)>>,
    /// One-shot closures for `defer()`.  Option so dispatch can `.take()`
    /// and move out the `FnOnce`.  Keyed by event id.
    deferred_callbacks: HashMap<u64, Option<Box<dyn FnOnce()>>>,
    next_id: u64,
}

impl Registry {
    fn new(handle: LoopHandle<'static, LoopData>) -> Self {
        Self {
            handle,
            registrations: HashMap::new(),
            fallback_io: HashMap::new(),
            timer_callbacks: HashMap::new(),
            io_callbacks: HashMap::new(),
            deferred_callbacks: HashMap::new(),
            next_id: 1,
        }
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

/// A callback taken out of the registry, dropped once the borrow has ended.
enum Removed {
    Timer(Box<dyn Fn()>),
    Io(Box<dyn Fn(c_int, c_short)>),
    Deferred(Box<dyn FnOnce()>),
}

thread_local! {
    /// The calloop loop. Borrowed only while calloop polls, when no tmux
    /// code runs (calloop's own callbacks only queue ready events).
    static LOOP: RefCell<Option<EventLoop<'static, LoopData>>> = const { RefCell::new(None) };

    /// Registrations and callbacks. Borrowed only inside [`with_registry`],
    /// which never calls tmux code or drops a callback while it holds the
    /// borrow. Dropping a callback can drop the handles it captured, and
    /// their Drop borrows the registry again.
    static REGISTRY: RefCell<Option<Registry>> = const { RefCell::new(None) };

    /// Event IDs deleted during a dispatch cycle. Checked before firing each
    /// callback so we skip events whose underlying resource was freed by an
    /// earlier callback in the same iteration (e.g. SIGCHLD destroying a pane
    /// whose read event is also ready).
    static CANCELLED: RefCell<HashSet<u64>> = RefCell::new(HashSet::new());
}

/// Runs `f` on the registry. Returns `None` if the loop is not initialised
/// or the thread is shutting down.
///
/// `f` must not call tmux code, and must hand back any callback it removes
/// instead of dropping it, so nothing can borrow the registry again while
/// `f` runs.
fn with_registry<R>(f: impl FnOnce(&mut Registry) -> R) -> Option<R> {
    let mut f = Some(f);
    let result = REGISTRY
        .try_with(|cell| {
            let mut registry = cell.borrow_mut();
            let base = registry.as_mut()?;
            f.take().map(|f| f(base))
        })
        .ok()
        .flatten();
    // If `f` did not run, it (and any callback it captured) is dropped here,
    // after the borrow has ended.
    drop(f);
    result
}

/// Removes event `id` and marks it cancelled for the current dispatch cycle.
fn deregister(id: u64) {
    let removed = with_registry(|base| {
        if let Some(reg) = base.registrations.remove(&id) {
            base.handle.remove(reg.token);
        }
        base.fallback_io.remove(&id);
        let timer = base.timer_callbacks.remove(&id).map(Removed::Timer);
        let io = base.io_callbacks.remove(&id).map(Removed::Io);
        let deferred = base
            .deferred_callbacks
            .remove(&id)
            .flatten()
            .map(Removed::Deferred);
        (timer, io, deferred)
    });
    // Mark cancelled so any already-queued ReadyEvent for this id is
    // skipped in the current dispatch cycle.
    let _ = CANCELLED.try_with(|cell| cell.borrow_mut().insert(id));
    // The callbacks are dropped here, after the borrow has ended.
    drop(removed);
}

/// Creates a fresh loop and registry, replacing any previous ones.
///
/// The previous ones are leaked on purpose: after fork, dropping them
/// would double-free calloop's internal signal state.
fn install_loop() -> io::Result<()> {
    let event_loop: EventLoop<LoopData> = EventLoop::try_new()?;
    let registry = Registry::new(event_loop.handle());
    let old_loop = LOOP.with(|cell| cell.borrow_mut().replace(event_loop));
    let old_registry = REGISTRY.with(|cell| cell.borrow_mut().replace(registry));
    std::mem::forget(old_loop);
    std::mem::forget(old_registry);
    Ok(())
}

/// The value handed out as `*mut event_base`. Callers only pass it on and
/// compare it with null, it is never dereferenced.
fn base_marker() -> *mut event_base {
    std::ptr::NonNull::<event_base>::dangling().as_ptr()
}

#[cfg(target_os = "linux")]
fn signal_from_number(signum: c_int) -> Option<Signal> {
    match signum {
        libc::SIGCHLD => Some(Signal::SIGCHLD),
        libc::SIGHUP => Some(Signal::SIGHUP),
        libc::SIGINT => Some(Signal::SIGINT),
        libc::SIGTERM => Some(Signal::SIGTERM),
        libc::SIGUSR1 => Some(Signal::SIGUSR1),
        libc::SIGUSR2 => Some(Signal::SIGUSR2),
        libc::SIGWINCH => Some(Signal::SIGWINCH),
        libc::SIGCONT => Some(Signal::SIGCONT),
        libc::SIGTSTP => Some(Signal::SIGTSTP),
        libc::SIGPIPE => Some(Signal::SIGPIPE),
        libc::SIGQUIT => Some(Signal::SIGQUIT),
        _ => {
            log_debug!("event_calloop: unsupported signal {signum}");
            None
        }
    }
}

/// Create a non-blocking pipe, returning `(read, write)` as `OwnedFd`s.
#[cfg(not(target_os = "linux"))]
fn pipe_nonblock() -> io::Result<(OwnedFd, OwnedFd)> {
    use std::os::fd::AsRawFd;
    let (read_fd, write_fd) =
        rustix::pipe::pipe().map_err(|e| io::Error::from_raw_os_error(e.raw_os_error()))?;
    // Set non-blocking and close-on-exec on both ends.
    for fd in [read_fd.as_raw_fd(), write_fd.as_raw_fd()] {
        // SAFETY: both fds were just created and are owned here.
        unsafe {
            libc::fcntl(fd, libc::F_SETFL, libc::O_NONBLOCK);
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    Ok((read_fd, write_fd))
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Creates the event loop for this thread.
pub unsafe fn event_init() -> *mut event_base {
    match install_loop() {
        Ok(()) => {
            log_debug!("event_init: calloop pid={}", std::process::id());
            base_marker()
        }
        Err(e) => {
            log_debug!("event_init: calloop failed: {e}");
            std::ptr::null_mut()
        }
    }
}

/// Replaces the event loop after fork. The old loop's epoll fd is an
/// inherited copy and does not work correctly in the child.
pub unsafe fn event_reinit(_base: *mut event_base) -> c_int {
    match install_loop() {
        Ok(()) => 0,
        Err(_) => -1,
    }
}

/// Waits for events once and runs their callbacks.
///
/// No borrow of the loop or the registry is held while a callback runs,
/// so callbacks may register new events and drop any handle, including
/// their own.
pub unsafe fn event_loop(flags: c_int) -> c_int {
    let mut data = LoopData { ready: Vec::new() };

    // Fallback I/O events (non-epolable fds like regular files) are always
    // ready, so they fire on every iteration.
    let Some(has_fallback) = with_registry(|base| {
        for (&id, &(fd, events)) in &base.fallback_io {
            data.ready.push(ReadyEvent { id, fd, events });
        }
        !base.fallback_io.is_empty()
    }) else {
        return -1;
    };

    let timeout = if (flags & EVLOOP_NONBLOCK) != 0 {
        Some(Duration::ZERO)
    } else if has_fallback {
        // data.ready is never empty when fallback events exist. Still poll
        // epoll for real I/O and signals, but do not block forever.
        Some(Duration::from_millis(1))
    } else {
        // EVLOOP_ONCE: block until at least one event fires.
        None
    };
    LOOP.with(|cell| {
        if let Some(event_loop) = cell.borrow_mut().as_mut() {
            let _ = event_loop.dispatch(timeout, &mut data);
        }
    });

    // Clear the cancelled set before dispatching.
    CANCELLED.with(|cell| cell.borrow_mut().clear());

    // Dispatch all ready events, skipping any that were cancelled by an
    // earlier callback in this cycle (e.g. SIGCHLD handler destroying a
    // pane whose read event is also in the ready list).
    for ready in data.ready {
        if is_cancelled(ready.id) {
            continue;
        }
        // Take the callback out of the registry while it runs, so it can
        // drop its own handle or register new events. Put it back
        // afterwards unless its handle was dropped meanwhile.
        let Some(cb) = with_registry(|base| {
            if let Some(cb) = base.io_callbacks.remove(&ready.id) {
                Some(Removed::Io(cb))
            } else if let Some(cb) = base.timer_callbacks.remove(&ready.id) {
                Some(Removed::Timer(cb))
            } else if let Some(cb) = base.deferred_callbacks.remove(&ready.id).flatten() {
                // One-shot: the registration goes with it.
                base.registrations.remove(&ready.id);
                Some(Removed::Deferred(cb))
            } else {
                None
            }
        })
        .flatten() else {
            continue;
        };

        let cb = match cb {
            Removed::Io(cb) => {
                cb(ready.fd, ready.events);
                Removed::Io(cb)
            }
            Removed::Timer(cb) => {
                cb();
                Removed::Timer(cb)
            }
            Removed::Deferred(cb) => {
                cb();
                continue;
            }
        };
        if is_cancelled(ready.id) {
            // Dropped here, outside the registry borrow.
            drop(cb);
            continue;
        }
        let _ = with_registry(|base| match cb {
            Removed::Io(cb) => {
                base.io_callbacks.insert(ready.id, cb);
            }
            Removed::Timer(cb) => {
                base.timer_callbacks.insert(ready.id, cb);
            }
            Removed::Deferred(_) => unreachable!(),
        });
    }

    0
}

/// Whether event `id` was deregistered during the current dispatch cycle.
fn is_cancelled(id: u64) -> bool {
    CANCELLED
        .try_with(|cell| cell.borrow().contains(&id))
        .unwrap_or(true)
}

static VERSION_STR: &[u8] = b"calloop-event/1.0\0";
static METHOD_STR: &[u8] = b"calloop\0";

pub fn event_get_version() -> *const u8 {
    VERSION_STR.as_ptr()
}

pub fn event_get_method() -> *const u8 {
    METHOD_STR.as_ptr()
}

pub fn event_set_log_callback(_cb: event_log_cb) {
    // TODO: wire up to calloop's logging or a global callback.
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    use super::*;

    /// A timer callback may drop its own handle and register new timers
    /// while it runs. Both used to free or move the running closure,
    /// because dispatch called it through a reference into the map.
    #[test]
    fn timer_callback_can_drop_its_own_handle() {
        unsafe { event_init() };
        let slot: Rc<RefCell<Option<TimerHandle>>> = Rc::new(RefCell::new(None));
        let fired = Rc::new(Cell::new(0));
        let extra: Rc<RefCell<Vec<TimerHandle>>> = Rc::new(RefCell::new(Vec::new()));

        let (s, f, e) = (slot.clone(), fired.clone(), extra.clone());
        let handle = timer_add(
            Duration::from_millis(1),
            Box::new(move || {
                f.set(f.get() + 1);
                // Drop our own handle while running.
                s.borrow_mut().take();
                // Register enough timers to make the map reallocate.
                for _ in 0..256 {
                    if let Some(h) = timer_add(Duration::from_secs(3600), Box::new(|| {})) {
                        e.borrow_mut().push(h);
                    }
                }
            }),
        );
        *slot.borrow_mut() = handle;

        unsafe { event_loop(crate::EVLOOP_ONCE) };
        assert_eq!(fired.get(), 1);
        assert!(slot.borrow().is_none());
        assert_eq!(extra.borrow().len(), 256);
    }

    /// A timer callback may register an fd, and that fd's callback fires
    /// in a later iteration.
    #[test]
    fn timer_callback_can_register_an_fd() {
        unsafe { event_init() };
        let (read_fd, write_fd) = pipe_pair();
        let io_fired = Rc::new(Cell::new(0));
        let io_handle: Rc<RefCell<Option<IoHandle>>> = Rc::new(RefCell::new(None));

        let (f, h) = (io_fired.clone(), io_handle.clone());
        let _timer = timer_add(
            Duration::from_millis(1),
            Box::new(move || {
                let f = f.clone();
                *h.borrow_mut() = io_register(
                    read_fd,
                    crate::EV_READ,
                    Box::new(move |_, _| f.set(f.get() + 1)),
                );
            }),
        );
        unsafe { event_loop(crate::EVLOOP_ONCE) };
        assert!(io_handle.borrow().is_some());

        assert_eq!(unsafe { libc::write(write_fd, b"x".as_ptr().cast(), 1) }, 1);
        unsafe { event_loop(crate::EVLOOP_ONCE) };
        assert_eq!(io_fired.get(), 1);
        close_pair(read_fd, write_fd);
    }

    /// An fd callback may drop its own handle. It does not fire again even
    /// though the fd stays readable.
    #[test]
    fn io_callback_can_drop_its_own_handle() {
        unsafe { event_init() };
        let (read_fd, write_fd) = pipe_pair();
        let slot: Rc<RefCell<Option<IoHandle>>> = Rc::new(RefCell::new(None));
        let fired = Rc::new(Cell::new(0));

        let (s, f) = (slot.clone(), fired.clone());
        *slot.borrow_mut() = io_register(
            read_fd,
            crate::EV_READ,
            Box::new(move |_, _| {
                f.set(f.get() + 1);
                s.borrow_mut().take();
            }),
        );
        assert_eq!(unsafe { libc::write(write_fd, b"x".as_ptr().cast(), 1) }, 1);
        unsafe { event_loop(crate::EVLOOP_ONCE) };
        unsafe { event_loop(crate::EVLOOP_NONBLOCK) };
        assert_eq!(fired.get(), 1);
        assert!(slot.borrow().is_none());
        close_pair(read_fd, write_fd);
    }

    /// Dropping a handle drops its callback, and with it any handles the
    /// callback owns. Their Drop reaches the event loop again, which must
    /// not find it borrowed.
    #[test]
    fn dropping_a_callback_that_owns_handles() {
        unsafe { event_init() };
        let inner_fired = Rc::new(Cell::new(0));
        let f = inner_fired.clone();
        let inner = timer_add(
            Duration::from_millis(1),
            Box::new(move || f.set(f.get() + 1)),
        );
        let outer = timer_add(
            Duration::from_secs(3600),
            Box::new(move || {
                let _ = &inner;
            }),
        );
        drop(outer);
        // The inner timer was dropped with the outer callback, so it
        // never fires.
        unsafe { event_loop(crate::EVLOOP_NONBLOCK) };
        std::thread::sleep(Duration::from_millis(5));
        unsafe { event_loop(crate::EVLOOP_NONBLOCK) };
        assert_eq!(inner_fired.get(), 0);
    }

    /// A deferred callback may register a timer and defer more work.
    #[test]
    fn deferred_callback_can_register_events() {
        unsafe { event_init() };
        let order: Rc<RefCell<Vec<&str>>> = Rc::new(RefCell::new(Vec::new()));
        let timer: Rc<RefCell<Option<TimerHandle>>> = Rc::new(RefCell::new(None));

        let (o, t) = (order.clone(), timer.clone());
        defer(Box::new(move || {
            o.borrow_mut().push("deferred");
            let o2 = o.clone();
            *t.borrow_mut() = timer_add(
                Duration::from_millis(1),
                Box::new(move || o2.borrow_mut().push("timer")),
            );
            let o3 = o.clone();
            defer(Box::new(move || o3.borrow_mut().push("deferred again")));
        }));
        for _ in 0..10 {
            if order.borrow().len() == 3 {
                break;
            }
            unsafe { event_loop(crate::EVLOOP_ONCE) };
        }
        let order = order.borrow();
        assert_eq!(order[0], "deferred");
        assert!(order.contains(&"timer"));
        assert!(order.contains(&"deferred again"));
    }

    fn pipe_pair() -> (c_int, c_int) {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        (fds[0], fds[1])
    }

    fn close_pair(read_fd: c_int, write_fd: c_int) {
        unsafe {
            libc::close(read_fd);
            libc::close(write_fd);
        }
    }
}
