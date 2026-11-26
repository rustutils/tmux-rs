// Copyright (c) 2026 Nicholas Marriott <nicholas.marriott@gmail.com>
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

//! Events: something happened, and everyone who asked is told.
//!
//! Code that changes state fires an event with a name (`window-renamed`)
//! and a [`Payload`]. The event goes to every sink registered under that
//! name, at once and in the order they were added:
//!
//! - control mode notifications (`%window-renamed`), in
//!   `control_notify.rs`,
//! - hooks, which queue the commands of the hook option, in hooks.rs,
//! - `wait-for -E` waiters, in `cmd_wait_for.rs`.
//!
//! [`EventBus`] in [`bus`] decides which sinks get an event and [`Payload`]
//! in [`payload`] holds what it carries. Both are safe Rust. This file is
//! the adapter: it builds payloads from entities, reaches the bus through
//! the bridge in global.rs and calls the sinks. It stays unsafe until the
//! entities and the sinks are converted.
//!
//! This replaces notify.c, which queued a callback for each notification
//! and ran hooks and control notifications later from the command queue.
//! Events are delivered when they fire, as in tmux 3.8.

pub mod bus;
pub mod payload;

pub use bus::{EventBus, Sink, SinkId};
pub use payload::{Payload, PayloadValue, Target};

use crate::global::with_events;
use crate::*;

/// Registers a sink for events called `name`.
pub fn events_add_sink(name: &str, sink: Sink) -> SinkId {
    with_events(|bus| bus.add(name, sink))
}

/// Removes a sink. Safe to call from a sink while an event is delivered.
pub fn events_remove_sink(id: SinkId) {
    with_events(|bus| bus.remove(id));
}

/// Whether a sink is registered for `name`.
pub fn events_has_sink(name: &str, sink: Sink) -> bool {
    with_events(|bus| bus.has(name, sink))
}

/// A client as a payload value, by ID with its name.
///
/// # Safety
/// `c` must be a valid client.
pub unsafe fn payload_client(c: *mut client) -> PayloadValue {
    unsafe {
        PayloadValue::Client {
            id: (*c).id,
            name: cstr_to_string_lossy((*c).name),
        }
    }
}

/// A session as a payload value, by ID with its name.
///
/// # Safety
/// `s` must be a valid session.
pub unsafe fn payload_session(s: *mut session) -> PayloadValue {
    unsafe {
        PayloadValue::Session {
            id: SessionId((*s).id),
            name: (*s).name.to_string(),
        }
    }
}

/// A window as a payload value, by ID with its name.
///
/// # Safety
/// `w` must be a valid window.
pub unsafe fn payload_window(w: *mut window) -> PayloadValue {
    unsafe {
        PayloadValue::Window {
            id: WindowId((*w).id),
            name: (*w).name.as_deref().unwrap_or("").to_string(),
        }
    }
}

/// A pane as a payload value.
///
/// # Safety
/// `wp` must be a valid pane.
pub unsafe fn payload_pane(wp: *mut window_pane) -> PayloadValue {
    unsafe { PayloadValue::Pane(PaneId((*wp).id)) }
}

/// The payload target for a command target, as C's
/// `event_payload_set_target`: the winlink becomes its index, and its
/// session and window fill in what the target lacks.
///
/// # Safety
/// `fs` must be a valid find state whose winlink, if any, is valid.
pub unsafe fn payload_target(fs: *const cmd_find_state) -> Target {
    unsafe {
        let mut t = Target {
            session: (*fs).s,
            window: (*fs).w,
            pane: (*fs).wp,
            index: None,
        };
        if let Some(wl) = (*fs).wl.and_then(winlink_get) {
            t.index = Some(wl.idx);
            if t.session.is_none() {
                t.session = Some(wl.session);
            }
            if t.window.is_none() {
                t.window = Some(wl.window);
            }
        }
        t
    }
}

/// Turns a payload target back into a command target, as C's
/// `event_payload_get_target`. Parts that are gone are dropped and the
/// target is rebuilt from what is left: the winlink and pane, the pane,
/// the winlink, the session and window, the session, and finally the best
/// session. Returns false if there is nothing to target.
///
/// # Safety
/// `fs` must point to a find state that can be overwritten. Its flags are
/// kept.
pub unsafe fn payload_get_target(payload: &Payload, fs: *mut cmd_find_state) -> bool {
    unsafe {
        let t = payload.target();
        let flags = (*fs).flags;
        let s = t
            .session
            .and_then(|id| session_from_id(id))
            .unwrap_or(null_mut());
        let w = t.window.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        let wp = t.pane.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        let s_alive = !s.is_null() && session_alive(s);

        let mut wl: Option<Winlink> = None;
        if let Some(idx) = t.index
            && s_alive
            && !w.is_null()
        {
            wl = winlink_find_by_index((*s).sid(), idx).filter(|wl| Some(wl.window) == t.window);
        }

        cmd_find_clear_state(fs, flags);
        (*fs).s = if s.is_null() { None } else { t.session };
        (*fs).w = if w.is_null() { None } else { t.window };
        (*fs).wp = if wp.is_null() { None } else { t.pane };
        (*fs).wl = wl.map(|wl| wl.id);
        (*fs).idx = wl.map_or(-1, |wl| wl.idx);
        if cmd_find_valid_state(fs) {
            return true;
        }

        if let Some(wl) = wl
            && !wp.is_null()
            && window_has_pane(&*winlink_window(wl), wp)
        {
            cmd_find_from_winlink_pane(fs, wl, wp, flags);
            if cmd_find_valid_state(fs) {
                return true;
            }
        }
        if !wp.is_null() && cmd_find_from_pane(fs, wp, flags) == 0 && cmd_find_valid_state(fs) {
            return true;
        }
        if let Some(wl) = wl {
            cmd_find_from_winlink(fs, wl, flags);
            if cmd_find_valid_state(fs) {
                return true;
            }
        }
        if s_alive
            && !w.is_null()
            && cmd_find_from_session_window(fs, s, w, flags) == 0
            && cmd_find_valid_state(fs)
        {
            return true;
        }
        if s_alive {
            cmd_find_from_session(fs, s, flags);
            if cmd_find_valid_state(fs) {
                return true;
            }
        }
        if cmd_find_from_nothing(fs, flags) == 0 {
            return true;
        }
        cmd_find_clear_state(fs, flags);
        false
    }
}

/// Delivers an event to its sinks.
///
/// `item` is the command an after-* or command-error hook belongs to,
/// null for everything else. Hooks for it are inserted right after it
/// rather than at the end of the queue. (C passes it in the payload as
/// `_cmdq_item`.)
///
/// # Safety
/// The sinks run unconverted code: every entity the payload names must be
/// valid or gone from its registry, and `item` must be null or valid.
pub unsafe fn events_fire(name: &str, mut payload: Payload, item: *mut cmdq_item) {
    payload.set_string("event", name);
    log_debug!("events_fire: {}: {}", name, payload);

    let sinks = with_events(|bus| bus.begin(name));
    for (id, sink) in sinks {
        if !with_events(|bus| bus.is_live(id)) {
            continue;
        }
        // SAFETY: the caller guarantees the payload's entities and item.
        unsafe {
            match sink {
                Sink::Control => control_notify_event(name, &payload),
                Sink::Hooks => hooks_event(name, &payload, item),
                Sink::WaitFor(waiter) => cmd_wait_for_event(waiter, &payload),
                Sink::HookMonitor(id) => {
                    crate::hook_monitor::hooks_monitor_event(id, name, &payload);
                }
            }
        }
    }
    with_events(EventBus::end);
}

/// Adds the client, session, window, `window_index` and pane items for a
/// command target, the payload C's client events and set-hook -E carry.
///
/// # Safety
/// `fs` must be a valid find state and `c` null or a valid client.
pub unsafe fn payload_add_find_state(
    payload: &mut Payload,
    c: *mut client,
    fs: *const cmd_find_state,
) {
    unsafe {
        if !c.is_null() {
            payload.set("client", payload_client(c));
        }
        if let Some(s) = (*fs).s.and_then(|id| session_from_id(id)) {
            payload.set("session", payload_session(s));
        }
        if let Some(w) = (*fs).w.and_then(|id| window_from_id(id)) {
            payload.set("window", payload_window(w));
        }
        if let Some(wl) = (*fs).wl.and_then(winlink_get) {
            payload.set_int("window_index", wl.idx);
        } else if (*fs).idx != -1 {
            payload.set_int("window_index", (*fs).idx);
        }
        if let Some(wp) = (*fs).wp.and_then(|id| pane_from_id(id)) {
            payload.set("pane", payload_pane(wp));
        }
    }
}

/// The payload of a client event: the client and where it is.
///
/// # Safety
/// `c` must be a valid client.
pub unsafe fn payload_for_client(c: *mut client) -> Payload {
    unsafe {
        let mut fs: cmd_find_state = zeroed();
        cmd_find_from_client(&raw mut fs, c, cmd_find_flags::empty());
        let mut payload = Payload::new();
        payload.set_target(payload_target(&raw const fs));
        payload_add_find_state(&mut payload, c, &raw const fs);
        payload
    }
}

/// Fires a client event.
///
/// # Safety
/// `c` must be a valid client.
pub unsafe fn events_fire_client(name: &str, c: *mut client) {
    unsafe { events_fire(name, payload_for_client(c), null_mut()) }
}

/// The payload of a session event. A session that is being destroyed
/// has no target.
///
/// # Safety
/// `s` must be a valid session.
pub unsafe fn payload_for_session(s: *mut session) -> Payload {
    unsafe {
        let mut payload = Payload::new();
        if session_alive(s) {
            let mut fs: cmd_find_state = zeroed();
            cmd_find_from_session(&raw mut fs, s, cmd_find_flags::empty());
            payload.set_target(payload_target(&raw const fs));
        }
        payload.set("session", payload_session(s));
        payload
    }
}

/// Fires a session event.
///
/// # Safety
/// `s` must be a valid session.
pub unsafe fn events_fire_session(name: &str, s: *mut session) {
    unsafe { events_fire(name, payload_for_session(s), null_mut()) }
}

/// The payload of a window event.
///
/// # Safety
/// `w` must be a valid window.
pub unsafe fn payload_for_window(w: *mut window) -> Payload {
    unsafe {
        let mut fs: cmd_find_state = zeroed();
        cmd_find_from_window(&raw mut fs, w, cmd_find_flags::empty());
        let mut payload = Payload::new();
        payload.set_target(payload_target(&raw const fs));
        payload.set("window", payload_window(w));
        payload
    }
}

/// Fires a window event.
///
/// # Safety
/// `w` must be a valid window.
pub unsafe fn events_fire_window(name: &str, w: *mut window) {
    unsafe { events_fire(name, payload_for_window(w), null_mut()) }
}

/// The payload of a pane event: the pane and its window.
///
/// # Safety
/// `wp` must be a valid pane in a valid window.
pub unsafe fn payload_for_pane(wp: *mut window_pane) -> Payload {
    unsafe {
        let mut fs: cmd_find_state = zeroed();
        cmd_find_from_pane(&raw mut fs, wp, cmd_find_flags::empty());
        let mut payload = Payload::new();
        payload.set_target(payload_target(&raw const fs));
        payload.set("pane", payload_pane(wp));
        if let Some(w) = (*wp).window.and_then(|id| window_from_id(id)) {
            payload.set("window", payload_window(w));
        }
        payload
    }
}

/// Fires a pane event.
///
/// # Safety
/// `wp` must be a valid pane in a valid window.
pub unsafe fn events_fire_pane(name: &str, wp: *mut window_pane) {
    unsafe { events_fire(name, payload_for_pane(wp), null_mut()) }
}

/// The payload of a winlink event: its session, window and index. The
/// winlink is a copy, so this works for one that was just removed.
///
/// # Safety
/// The winlink's session and window must be valid or gone from the
/// registries.
pub unsafe fn payload_for_winlink(wl: Winlink) -> Payload {
    unsafe {
        let mut fs: cmd_find_state = zeroed();
        cmd_find_from_winlink(&raw mut fs, wl, cmd_find_flags::empty());
        let mut payload = Payload::new();
        let mut target = payload_target(&raw const fs);
        target.index = Some(wl.idx);
        payload.set_target(target);
        if let Some(s) = session_from_id(wl.session) {
            payload.set("session", payload_session(s));
        }
        if let Some(w) = window_from_id(wl.window) {
            payload.set("window", payload_window(w));
        }
        payload.set_int("window_index", wl.idx);
        payload
    }
}

/// Fires a winlink event.
///
/// # Safety
/// The winlink's session and window must be valid or gone from the
/// registries.
pub unsafe fn events_fire_winlink(name: &str, wl: Winlink) {
    unsafe { events_fire(name, payload_for_winlink(wl), null_mut()) }
}
