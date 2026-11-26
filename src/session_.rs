// Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
//
// Permission to use, copy, modify, and distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
// IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
// OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
use std::time::Duration;

use crate::*;
use crate::sort::SortCriteria;
use crate::options_::*;
use crate::global::with_links;

pub static mut SESSIONS: sessions = BTreeMap::new();

/// Central registry owning all session allocations. `Box<session>` provides a
/// stable heap address, so `*mut session` pointers derived from it remain valid
/// for the lifetime of the registry entry.
pub static mut SESSION_REGISTRY: BTreeMap<SessionId, Box<session>> = BTreeMap::new();

pub static NEXT_SESSION_ID: AtomicU32 = AtomicU32::new(0);

/// Iterate over all **alive** sessions as `*mut session` pointers.
///
/// Uses the `SESSIONS` name index (not `SESSION_REGISTRY`), because
/// destroyed sessions remain in the registry until their reference count
/// drains. Callers expect to only see live, usable sessions.
#[inline]
pub unsafe fn sessions_iter() -> impl Iterator<Item = *mut session> {
    unsafe {
        (*(&raw mut SESSIONS))
            .values()
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
    }
}

/// Look up a session by ID in the global registry.
pub unsafe fn session_from_id(id: SessionId) -> Option<*mut session> {
    unsafe {
        (*(&raw mut SESSION_REGISTRY))
            .get_mut(&id)
            .map(|b| &mut **b as *mut session)
    }
}

/// Look up a session by ID and return a shared reference, suitable for
/// read-only access. See `client_ref` for the rationale and aliasing
/// caveats. The same convention applies.
#[expect(dead_code, reason = "Phase 2.4 hook; used opportunistically going forward")]
pub unsafe fn session_ref(id: SessionId) -> Option<&'static session> {
    unsafe {
        (*(&raw const SESSION_REGISTRY))
            .get(&id)
            .map(|b| &**b as &session)
    }
}

/// Check whether a session is still alive (not yet destroyed).
///
/// Uses the `SESSIONS` name index, not `SESSION_REGISTRY`, because a destroyed
/// session remains in the registry until its reference count reaches zero and
/// `session_free` runs.
pub unsafe fn session_alive(s: *mut session) -> bool {
    unsafe { (*(&raw mut SESSIONS)).values().any(|&s_ptr| s_ptr == s) }
}

/// Find session by name.
pub unsafe fn session_find(name: &str) -> *mut session {
    unsafe { (*(&raw mut SESSIONS)).get(name).copied().unwrap_or(null_mut()) }
}

/// Find session by id parsed from a string.
pub unsafe fn session_find_by_id_str(s: &str) -> *mut session {
    unsafe {
        if !s.starts_with('$') {
            return null_mut();
        }

        let Ok(id) = strtonum_(&s[1..], 0, u32::MAX) else {
            return null_mut();
        };
        transmute_ptr(session_find_by_id(id))
    }
}

/// Find session by id. O(log n) via `SESSION_REGISTRY`.
pub unsafe fn session_find_by_id(id: u32) -> Option<NonNull<session>> {
    unsafe { session_from_id(SessionId(id)).and_then(NonNull::new) }
}

impl session {
    /// The session's ID.
    pub fn sid(&self) -> SessionId {
        SessionId(self.id)
    }

    /// The current winlink. None only while the session is built or
    /// destroyed.
    pub fn curw(&self) -> Option<Winlink> {
        let sid = self.sid();
        with_links(|l| l.current(sid))
    }

    /// The ID of the current winlink.
    pub fn curw_id(&self) -> Option<WinlinkId> {
        self.curw().map(|wl| wl.id)
    }

    /// The winlinks in index order.
    pub fn windows(&self) -> Vec<Winlink> {
        let sid = self.sid();
        with_links(|l| l.windows(sid))
    }

    /// The last window stack, most recent first.
    pub fn lastw(&self) -> Vec<Winlink> {
        let sid = self.sid();
        with_links(|l| l.last_stack(sid))
    }

    unsafe fn create(
        prefix: *const u8,
        name: Option<&str>,
        cwd: *const u8,
        env: Environ,
        oo: *mut options,
        tio: *mut termios,
    ) -> Box<Self> {
        unsafe {
            let mut s: Box<session> = Box::new(MaybeUninit::<session>::zeroed().assume_init_read());
            s.flags = 0;

            // xcalloc'd zero bytes are NOT a guaranteed-valid Option<PathBuf>::None,
            // so initialize via ptr::write before any read.
            std::ptr::write(
                &raw mut s.cwd,
                Some(PathBuf::from(
                    std::ffi::CStr::from_ptr(cwd.cast())
                        .to_string_lossy()
                        .into_owned(),
                )),
            );

            std::ptr::write(&raw mut s.lock_timer, None);

            std::ptr::write(&raw mut s.environ, Rc::new(env));
            s.options = oo;

            status_update_cache(s.as_mut());

            std::ptr::write(&raw mut s.tio,
                if !tio.is_null() { Some(Box::new(*tio)) } else { None });


            if let Some(name) = name {
                s.name = name.to_string().into();
                s.id = NEXT_SESSION_ID.fetch_add(1, atomic::Ordering::Relaxed);
            } else {
                loop {
                    s.id = NEXT_SESSION_ID.fetch_add(1, atomic::Ordering::Relaxed);
                    s.name = if !prefix.is_null() {
                        format!("{}-{}", _s(prefix), s.id).into()
                    } else {
                        format!("{}", s.id).into()
                    };

                    if !(*(&raw mut SESSIONS)).contains_key(&*s.name) {
                        break;
                    }
                }
            }
            (*(&raw mut SESSIONS)).insert(s.name.to_string(), s.as_mut());

            log_debug!("new session {} ${}", s.name, s.id);

            if libc::gettimeofday(&raw mut s.creation_time, null_mut()) != 0 {
                fatal("gettimeofday failed");
            }
            session_update_activity(s.as_mut(), &raw mut s.creation_time);

            s
        }
    }
}

/// Create a new session.
///
/// Allocates the session in `SESSION_REGISTRY`. The `Box` in the registry
/// provides a stable heap address, so the returned `*mut session` is valid
/// for the lifetime of the registry entry.
pub unsafe fn session_create(
    prefix: *const u8,
    name: Option<&str>,
    cwd: *const u8,
    env: Environ,
    oo: *mut options,
    tio: *mut termios,
) -> *mut session {
    unsafe {
        let boxed = session::create(prefix, name, cwd, env, oo, tio);
        let id = SessionId(boxed.id);
        let s = (*(&raw mut SESSION_REGISTRY)).entry(id).or_insert(boxed);
        &mut **s as *mut session
    }
}

/// Free a destroyed session, from the event loop after the destroy, so
/// pointers the code that destroyed it still holds stay valid until it
/// returns. Nothing keeps a session across the loop: everything else
/// holds its ID, and a stale ID finds nothing.
///
/// Removes the session from `SESSION_REGISTRY`, which drops the `Box<session>`
/// and deallocates the memory.
unsafe fn session_free_deferred(sid: SessionId) {
    unsafe {
        let Some(s) = session_from_id(sid) else { return };

        log_debug!("session {} freed", (*s).name);

        options_free((*s).options);
        (*s).name = Cow::Borrowed("");
        // Drop the Box from the registry. Box drop runs Drop on all fields,
        // including `cwd: Option<PathBuf>` and the environment.
        let _ = (*(&raw mut SESSION_REGISTRY)).remove(&sid);
    }
}

/// Destroy a session.
pub unsafe fn session_destroy(s: *mut session, notify: i32, from: *const u8) {
    let __func__ = c!("session_destroy");
    unsafe {
        log_debug!("session {} destroyed ({})", (*s).name, _s(from));

        // Destroyed already. C checks for a current window, which a
        // session whose first window failed has as a dangling pointer, and
        // here does not.
        if !session_alive(s) {
            return;
        }

        (*(&raw mut SESSIONS)).remove(&*(*s).name);
        if notify != 0 {
            events_fire_session("session-closed", s);
        }

        (*s).tio = None; // Box<termios> dropped

        // Drop the timer handle to deregister from the event loop.
        (*s).lock_timer = None;

        session_group_remove(s);

        // Each window is unlinked while it is still linked, as in C.
        for wl in (*s).windows() {
            events_fire_winlink("window-unlinked", wl);
            winlink_remove(wl.id);
        }
        let sid = (*s).sid();
        with_links(|l| l.remove_session(sid));

        defer(Box::new(move || session_free_deferred(sid)));
    }
}

/// Update activity time.
pub unsafe fn session_update_activity(s: *mut session, from: *mut timeval) {
    unsafe {
        if from.is_null() {
            libc::gettimeofday(&raw mut (*s).activity_time, null_mut());
        } else {
            memcpy__(&raw mut (*s).activity_time, from);
        }

        log_debug!(
            "session ${} {} activity {}.{:06}",
            (*s).id,
            (*s).name,
            (*s).activity_time.tv_sec,
            (*s).activity_time.tv_usec,
        );

        // Cancel any existing lock timer.
        (*s).lock_timer = None;

        if (*s).attached != 0 {
            let lock_after = options_get_number_((*s).options, "lock-after-time");
            if lock_after != 0 {
                let sid = SessionId((*s).id);
                (*s).lock_timer = timer_add(
                    Duration::from_secs(lock_after as u64),
                    Box::new(move || {
                        let Some(s) = session_from_id(sid) else { return };
                        if (*s).attached == 0 {
                            return;
                        }
                        log_debug!(
                            "session {} locked, activity time {}",
                            (*s).name,
                            (*s).activity_time.tv_sec,
                        );
                        server_lock_session(s);
                        recalculate_sizes();
                    }),
                );
            }
        }
    }
}

/// The session before or after `s` in `l`, wrapping around. This is `s`
/// itself when it is the only one, and null when `s` is not in `l`.
fn session_neighbour(l: &[*mut session], s: *mut session, forward: bool) -> *mut session {
    let Some(i) = l.iter().position(|&x| x == s) else {
        debug_assert!(false, "session not found in sorted list");
        return null_mut();
    };
    let n = l.len();
    if forward { l[(i + 1) % n] } else { l[(i + n - 1) % n] }
}

/// Find the next session in the order `criteria` sorts them, wrapping
/// around. This is `s` itself when it is the only session.
///
/// # Safety
/// `s` must be null or a valid session.
pub unsafe fn session_next_session(s: *mut session, criteria: &SortCriteria) -> *mut session {
    unsafe {
        if (*(&raw const SESSIONS)).is_empty() || !session_alive(s) {
            return null_mut();
        }
        session_neighbour(&sort_get_sessions(criteria), s, true)
    }
}

/// Find the previous session in the order `criteria` sorts them, wrapping
/// around. This is `s` itself when it is the only session.
///
/// # Safety
/// `s` must be null or a valid session.
pub unsafe fn session_previous_session(s: *mut session, criteria: &SortCriteria) -> *mut session {
    unsafe {
        if (*(&raw const SESSIONS)).is_empty() || !session_alive(s) {
            return null_mut();
        }
        session_neighbour(&sort_get_sessions(criteria), s, false)
    }
}

/// The winlinks of a session by ID, in index order.
pub fn session_windows(s: Option<SessionId>) -> Vec<Winlink> {
    match s {
        Some(s) => with_links(|l| l.windows(s)),
        None => Vec::new(),
    }
}

/// The last window stack of a session by ID, most recent first.
pub fn session_lastw(s: Option<SessionId>) -> Vec<Winlink> {
    match s {
        Some(s) => with_links(|l| l.last_stack(s)),
        None => Vec::new(),
    }
}

/// The current winlink of a session by ID.
pub fn session_curw_id(s: Option<SessionId>) -> Option<WinlinkId> {
    let s = s?;
    with_links(|l| l.current(s)).map(|wl| wl.id)
}

/// Sets a session's current winlink without the effects of
/// `session_set_current`, for a session that has none (a new one).
pub fn session_set_curw_only(s: SessionId, wl: Option<WinlinkId>) {
    with_links(|l| l.set_current_only(s, wl));
}

/// Attach a window to a session.
pub unsafe fn session_attach(
    s: *mut session,
    w: *mut window,
    idx: i32,
) -> Result<WinlinkId, String> {
    unsafe {
        let Some(wl) = winlink_add((*s).sid(), idx, w) else {
            return Err(format!("index in use: {idx}"));
        };
        events_fire_winlink("window-linked", wl);

        session_group_synchronize_from(s);
        Ok(wl.id)
    }
}

/// Detach a window from a session. Returns 1 if the window has not been
/// detached because it is the session's only one: the caller must destroy
/// the session.
pub unsafe fn session_detach(s: *mut session, wl: WinlinkId) -> i32 {
    unsafe {
        let mut events = Vec::new();
        if !with_links(|l| l.detach_prepare(wl, &mut events)) {
            return 1;
        }
        links_apply(events);

        // Fired while the window is still linked, so control clients see
        // %window-close rather than %unlinked-window-close.
        if let Some(wl) = winlink_get(wl) {
            events_fire_winlink("window-unlinked", wl);
        }
        winlink_remove(wl);

        session_group_synchronize_from(s);
        0
    }
}

/// Return if session has window.
///
/// `s` may be null (e.g. when called with `client_get_session()` for an
/// unattached client). In that case the answer is always `false`.
pub unsafe fn session_has(s: *mut session, w: &window) -> bool {
    if s.is_null() {
        return false;
    }
    let sid = unsafe { (*s).sid() };
    let wid = WindowId(w.id);
    with_links(|l| l.has(sid, wid))
}

/// Return if a window is linked outside this session (not including
/// session groups). The window must be in this session. Like 3.8 this
/// compares the window's reference count with the group size.
pub unsafe fn session_is_linked(s: *mut session, w: &window) -> bool {
    unsafe {
        match session_group_contains(s) {
            Some(name) => w.references != session_group_count(&name),
            None => w.references != 1,
        }
    }
}

/// Carries out what a change to the links did, in order: window
/// references, window-linked and window-unlinked, and the effects of a
/// new current window.
///
/// # Safety
/// The sessions and windows the events name must be valid or gone from
/// the registries.
pub unsafe fn links_apply(events: Vec<LinkEvent>) {
    unsafe {
        for event in events {
            match event {
                LinkEvent::Linked(wl) => {
                    let w = winlink_window(wl);
                    if !w.is_null() {
                        window_add_ref(w, c!("winlink_set_window"));
                    }
                    events_fire_winlink("window-linked", wl);
                }
                LinkEvent::Unlinked(wl) => events_fire_winlink("window-unlinked", wl),
                LinkEvent::Removed(wl) => {
                    let w = winlink_window(wl);
                    if !w.is_null() {
                        window_remove_ref(w, c!("winlink_remove"));
                    }
                }
                LinkEvent::CurrentChanged(change) => session_current_changed(change),
            }
        }
    }
}

/// Move session to next window.
pub unsafe fn session_next(s: *mut session, alert: bool) -> i32 {
    unsafe {
        let sid = (*s).sid();
        match with_links(|l| l.next_target(sid, alert)) {
            Some(wl) => session_set_current(s, Some(wl)),
            None => -1,
        }
    }
}

/// Move session to previous window.
pub unsafe fn session_previous(s: *mut session, alert: bool) -> i32 {
    unsafe {
        let sid = (*s).sid();
        match with_links(|l| l.previous_target(sid, alert)) {
            Some(wl) => session_set_current(s, Some(wl)),
            None => -1,
        }
    }
}

/// Move session to specific window.
pub unsafe fn session_select(s: *mut session, idx: i32) -> i32 {
    unsafe {
        let wl = winlink_find_by_index((*s).sid(), idx);
        session_set_current(s, wl.map(|wl| wl.id))
    }
}

/// Move session to last used window.
pub unsafe fn session_last(s: *mut session) -> i32 {
    unsafe {
        let sid = (*s).sid();
        match with_links(|l| l.last_target(sid)) {
            crate::links::LastTarget::None => -1,
            crate::links::LastTarget::Current => 1,
            crate::links::LastTarget::Window(wl) => session_set_current(s, Some(wl)),
        }
    }
}

/// Set current winlink to wl. Returns -1 if it is missing, 1 if it is
/// current already.
pub unsafe fn session_set_current(s: *mut session, wl: Option<WinlinkId>) -> i32 {
    unsafe {
        let Some(wl) = wl else {
            return -1;
        };
        let sid = (*s).sid();
        if (*s).curw_id() == Some(wl) {
            return 1;
        }
        match with_links(|l| l.set_current(sid, wl)) {
            Some(change) => {
                session_current_changed(change);
                0
            }
            None => -1,
        }
    }
}

/// The effects of a new current window (the rest of `session_set_current`):
/// focus, the window's alert flags, activity, the offset and
/// session-window-changed.
unsafe fn session_current_changed(change: crate::links::CurrentChange) {
    unsafe {
        let w_cur = winlink_window(change.new);
        if w_cur.is_null() {
            return;
        }
        if options_get_number_(GLOBAL_OPTIONS, "focus-events") != 0 {
            if let Some(old) = change.old {
                let w_old = winlink_window(old);
                if !w_old.is_null() {
                    window_update_focus(w_old);
                }
            }
            window_update_focus(w_cur);
        }
        (*w_cur).flags &= !WINDOW_ALERTFLAGS;
        for sid in change.cleared {
            if let Some(s) = session_from_id(sid) {
                server_status_session(s);
            }
        }
        window_update_activity(NonNull::new_unchecked(w_cur));
        tty_update_window_offset(w_cur);
        if let Some(s) = session_from_id(change.session) {
            session_fire_window_changed(s, change.new, change.old);
        }
    }
}

/// Fires session-window-changed, with the new and old current window.
///
/// # Safety
/// `s` must be valid.
unsafe fn session_fire_window_changed(s: *mut session, wl: Winlink, old: Option<Winlink>) {
    unsafe {
        let mut fs: cmd_find_state = zeroed();
        cmd_find_from_winlink(&raw mut fs, wl, cmd_find_flags::empty());
        let mut payload = crate::events::Payload::new();
        payload.set_target(crate::events::payload_target(&raw const fs));
        payload.set("session", crate::events::payload_session(s));
        let w = winlink_window(wl);
        payload.set("window", crate::events::payload_window(w));
        payload.set("new_window", crate::events::payload_window(w));
        payload.set_int("window_index", wl.idx);
        payload.set_int("new_window_index", wl.idx);
        if let Some(old) = old {
            payload.set("old_window", crate::events::payload_window(winlink_window(old)));
            payload.set_int("old_window_index", old.idx);
        }
        crate::events::events_fire("session-window-changed", payload, null_mut());
    }
}

/// The name of the session group containing a session.
pub unsafe fn session_group_contains(target: *mut session) -> Option<String> {
    if target.is_null() {
        return None;
    }
    let sid = unsafe { (*target).sid() };
    with_links(|l| l.group_of(sid).map(str::to_string))
}

/// Whether a session group exists.
pub fn session_group_find(name: &str) -> bool {
    with_links(|l| l.group_exists(name))
}

/// The sessions of a group, in the order they joined.
pub unsafe fn session_group_sessions(name: &str) -> Vec<*mut session> {
    let ids = with_links(|l| l.group_sessions(name));
    ids.into_iter()
        .filter_map(|id| unsafe { session_from_id(id) })
        .collect()
}

/// Fires a session group event, with the group's name and size.
///
/// # Safety
/// `s` must be valid.
unsafe fn session_group_fire(event: &str, name: &str, s: *mut session) {
    unsafe {
        let mut payload = crate::events::payload_for_session(s);
        payload.set_string("group", name);
        payload.set_uint("group_size", session_group_count(name));
        crate::events::events_fire(event, payload, null_mut());
    }
}

/// Add a session to a session group, creating the group if needed
/// (`session_group_new` and `session_group_add`).
pub unsafe fn session_group_add(name: &str, s: *mut session) {
    unsafe {
        let sid = (*s).sid();
        if with_links(|l| l.group_add(name, sid)) {
            session_group_fire("session-added-to-group", name, s);
        }
    }
}

/// Remove a session from its group and destroy the group if empty.
pub unsafe fn session_group_remove(s: *mut session) {
    unsafe {
        let Some(name) = session_group_contains(s) else {
            return;
        };
        session_group_fire("session-removed-from-group", &name, s);
        let sid = (*s).sid();
        with_links(|l| l.group_remove(sid));
    }
}

/// Count number of sessions in session group.
pub fn session_group_count(name: &str) -> u32 {
    with_links(|l| l.group_sessions(name).len() as u32)
}

/// Count number of clients attached to sessions in session group.
pub unsafe fn session_group_attached_count(name: &str) -> u32 {
    unsafe {
        session_group_sessions(name)
            .iter()
            .map(|&s| (*s).attached)
            .sum()
    }
}

/// Synchronize a session to its session group.
pub unsafe fn session_group_synchronize_to(s: *mut session) {
    unsafe {
        let sid = (*s).sid();
        let mut events = Vec::new();
        with_links(|l| l.sync_to(sid, &mut events));
        links_apply(events);
    }
}

/// Synchronize a session group to a session.
pub unsafe fn session_group_synchronize_from(target: *mut session) {
    unsafe {
        let sid = (*target).sid();
        let mut events = Vec::new();
        with_links(|l| l.sync_from(sid, &mut events));
        links_apply(events);
    }
}

/// Renumber the windows across winlinks attached to a specific session.
/// The winlinks keep their IDs, so the marked pane needs no fixing.
pub unsafe fn session_renumber_windows(s: *mut session) {
    unsafe {
        let base = options_get_number_((*s).options, "base-index") as i32;
        let sid = (*s).sid();
        with_links(|l| l.renumber(sid, base));
    }
}

/// Applies the session's history-limit to the panes of its windows,
/// trimming histories that are over it.
///
/// # Safety
/// `s` must be a valid live session.
pub unsafe fn session_update_history(s: *mut session) {
    unsafe {
        let limit: u32 = options_get_number___(&*(*s).options, "history-limit");
        for wl in (*s).windows() {
            let w = winlink_window(wl);
            for &wp in &(*w).panes {
                let gd = &mut (*wp).base.grid;
                let osize = gd.hsize;
                gd.hlimit = limit;
                gd.collect_history_all();
                if gd.hsize != osize {
                    log_debug!("session_update_history: %{} {} -> {}", (*wp).id, osize, gd.hsize);
                }
            }
        }
    }
}
