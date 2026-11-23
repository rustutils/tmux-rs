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

//! Hook monitors (set-hook -B), the monitor part of hooks.c in 3.8.
//!
//! A hook option `@name` can watch a format with a [`MonitorSet`] of one
//! item. Once a second the monitor expands the format in its session (a
//! global hook uses the first session by name) and, when the value
//! changes, fires the event `@name` with the value, the last value and
//! the target. Its own sink, [`Sink::HookMonitor`], then runs the hook
//! from the tree it is in, expanding the command first. Other sinks see
//! the event too (wait-for -E), the plain hooks sink skips it.
//!
//! The monitor is held by the option entry ([`HookMonitor`] is the
//! monitor type of `COptions`), so it goes away with the entry. Its timer
//! and sink find it again through a registry from its number to the tree
//! and option name, kept as IDs rather than pointers.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::Duration;

use crate::events::{
    Payload, PayloadValue, Sink, SinkId, events_add_sink, events_fire, events_remove_sink, payload_pane,
    payload_session, payload_target, payload_window,
};
use crate::hooks::hooks_insert_event;
use crate::monitor::{
    MonitorChange, MonitorFlags, MonitorKey, MonitorSet, MonitorTarget, monitor_items,
    monitor_now, monitor_observe,
};
use crate::options_::*;
use crate::*;

/// The tree a hook option is in, by IDs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookTree {
    Server,
    GlobalSession,
    GlobalWindow,
    Session(SessionId),
    Window(WindowId),
    Pane(PaneId),
}

impl HookTree {
    /// The tree, or null if its entity is gone.
    ///
    /// # Safety
    /// The registries must hold valid entities.
    unsafe fn options(self) -> *mut options {
        unsafe {
            match self {
                Self::Server => GLOBAL_OPTIONS,
                Self::GlobalSession => GLOBAL_S_OPTIONS,
                Self::GlobalWindow => GLOBAL_W_OPTIONS,
                Self::Session(id) => session_from_id(id).map_or(null_mut(), |s| (*s).options),
                Self::Window(id) => window_from_id(id).map_or(null_mut(), |w| (*w).options),
                Self::Pane(id) => pane_from_id(id).map_or(null_mut(), |wp| (*wp).options),
            }
        }
    }
}

thread_local! {
    static NEXT_ID: Cell<u32> = const { Cell::new(1) };
    /// Each monitor's tree and option name, by number.
    static MONITORS: RefCell<BTreeMap<u32, (HookTree, String)>> = const { RefCell::new(BTreeMap::new()) };
}

/// The monitor of a hook option, held by the option entry. Dropping it
/// stops the timer and removes the sink and the registry entry.
pub struct HookMonitor {
    id: u32,
    set: MonitorSet,
    /// The session watched, None for a global hook.
    session: Option<SessionId>,
    sink: SinkId,
    timer: Option<TimerHandle>,
}

impl Drop for HookMonitor {
    fn drop(&mut self) {
        events_remove_sink(self.sink);
        MONITORS.with_borrow_mut(|m| m.remove(&self.id));
    }
}

/// The option entry of monitor `id`, if it still has that monitor.
///
/// # Safety
/// The registries must hold valid entities.
unsafe fn hooks_monitor_entry(id: u32) -> Option<*mut options_entry> {
    unsafe {
        let (tree, name) = MONITORS.with_borrow(|m| m.get(&id).cloned())?;
        let oo = tree.options();
        if oo.is_null() {
            return None;
        }
        let o = options_get_only(oo, &name);
        let monitor = o.as_ref()?.monitor()?;
        (monitor.id == id).then_some(o)
    }
}

/// The session a monitor watches: its own while it is alive, the first by
/// name for a global hook. Null for none.
unsafe fn hooks_monitor_session(session: Option<SessionId>) -> *mut session {
    unsafe {
        match session {
            Some(id) => sessions_iter().find(|&s| (*s).id == id.0).unwrap_or(null_mut()),
            None => sessions_iter().next().unwrap_or(null_mut()),
        }
    }
}

/// The timer: checks the monitor's value and fires an event for a change.
unsafe fn hooks_monitor_tick(id: u32) {
    unsafe {
        let Some(o) = hooks_monitor_entry(id) else {
            return;
        };
        let Some(hm) = (*o).monitor_mut() else {
            return;
        };
        hm.timer = timer_add(Duration::from_secs(1), Box::new(move || hooks_monitor_tick(id)));
        let s = hooks_monitor_session(hm.session);
        if s.is_null() {
            return;
        }
        let items = monitor_items(&hm.set);

        let observed = monitor_observe(&items, null_mut(), s);

        // Expanding formats ran other code, look the monitor up again.
        let Some(o) = hooks_monitor_entry(id) else {
            return;
        };
        let Some(hm) = (*o).monitor_mut() else {
            return;
        };
        let changes = hm.set.check(observed, monitor_now());
        for change in &changes {
            hooks_monitor_report(id, s, change);
        }
    }
}

/// Fires the event for a change, with the target and payload of
/// `hooks_monitor_cb`.
unsafe fn hooks_monitor_report(id: u32, s: *mut session, change: &MonitorChange) {
    unsafe {
        let mut wl: *mut winlink = null_mut();
        let mut wp: *mut window_pane = null_mut();
        match change.key {
            MonitorKey::Session => {}
            MonitorKey::Pane { pane, index } => {
                wp = window_pane_find_by_id(pane);
                wl = winlink_find_by_index(&raw mut (*s).windows, index);
            }
            MonitorKey::Window { index, .. } => {
                wl = winlink_find_by_index(&raw mut (*s).windows, index);
            }
        }

        let mut fs: cmd_find_state = zeroed();
        cmd_find_clear_state(&raw mut fs, cmd_find_flags::empty());
        if !wl.is_null() && !wp.is_null() && winlink_window(wl) == window_pane_window(wp) {
            cmd_find_from_winlink_pane(&raw mut fs, wl, wp, cmd_find_flags::empty());
        } else if !wl.is_null() {
            cmd_find_from_winlink(&raw mut fs, wl, cmd_find_flags::empty());
        } else if !wp.is_null() {
            cmd_find_from_pane(&raw mut fs, wp, cmd_find_flags::empty());
        } else {
            cmd_find_from_session(&raw mut fs, s, cmd_find_flags::empty());
        }

        let mut payload = Payload::new();
        payload.set_uint("_hooks_monitor", id);
        payload.set_target(payload_target(&raw const fs));
        payload.set_string("value", change.value.as_str());
        payload.set_string("last", change.last.as_deref().unwrap_or(""));
        payload.set("session", payload_session(s));
        if !wl.is_null() {
            payload.set("window", payload_window(winlink_window(wl)));
            payload.set_int("window_index", (*wl).idx);
        }
        if !wp.is_null() {
            payload.set("pane", payload_pane(wp));
            if wl.is_null() {
                payload.set("window", payload_window(window_pane_window(wp)));
            }
        }
        events_fire(&change.name, payload, null_mut());
    }
}

/// The monitor's sink: runs the hook for the monitor's own events, from
/// the monitor's tree, with the command expanded.
///
/// # Safety
/// The payload's entities must be valid or gone from their registries.
pub unsafe fn hooks_monitor_event(id: u32, name: &str, payload: &Payload) {
    unsafe {
        if payload.get("_hooks_monitor") != Some(&PayloadValue::Uint(id)) {
            return;
        }
        let Some(tree) = MONITORS.with_borrow(|m| m.get(&id).map(|(tree, _)| *tree)) else {
            return;
        };
        let oo = tree.options();
        if oo.is_null() {
            return;
        }
        hooks_insert_event(cmdq_running(null_mut()), name, payload, oo, true);
    }
}

/// Adds a monitor to hook option `name` in `oo`, replacing one it has. A
/// missing option is set to an empty string.
///
/// # Safety
/// `oo` must be a valid tree, and `tree` must name it.
pub unsafe fn hooks_monitor_add(
    oo: *mut options,
    tree: HookTree,
    name: &str,
    target: MonitorTarget,
    format: &str,
    flags: MonitorFlags,
    session: Option<SessionId>,
) {
    unsafe {
        hooks_monitor_remove(oo, name);
        let mut o = options_get_only(oo, name);
        if o.is_null() {
            o = options_set_string_bytes(oo, name, b"", false);
        }

        let id = NEXT_ID.get();
        NEXT_ID.set(id.wrapping_add(1).max(1));
        MONITORS.with_borrow_mut(|m| m.insert(id, (tree, name.to_string())));

        let mut set = MonitorSet::new();
        set.add(name, target, format, flags);
        let monitor = HookMonitor {
            id,
            set,
            session,
            sink: events_add_sink(name, Sink::HookMonitor(id)),
            timer: timer_add(Duration::from_secs(1), Box::new(move || hooks_monitor_tick(id))),
        };
        (*o).set_monitor(Some(monitor));
    }
}

/// Removes the monitor of hook option `name` in `oo`, keeping the option.
///
/// # Safety
/// `oo` must be a valid tree.
pub unsafe fn hooks_monitor_remove(oo: *mut options, name: &str) {
    unsafe {
        let o = options_get_only(oo, name);
        if !o.is_null() {
            drop((*o).set_monitor(None));
        }
    }
}

/// The monitor's target and format, if the option has a monitor.
///
/// # Safety
/// `o` must be a valid entry.
pub unsafe fn hooks_monitor_get(o: *mut options_entry) -> Option<(MonitorTarget, String)> {
    unsafe {
        let monitor = (*o).monitor()?;
        let (_, target, format) = monitor.set.items().next()?;
        Some((target, format.to_string()))
    }
}

/// The monitor as set-hook -B takes it: `name:target:format`.
///
/// # Safety
/// `o` must be a valid entry.
pub unsafe fn hooks_monitor_to_string(o: *mut options_entry) -> Option<String> {
    unsafe {
        let (target, format) = hooks_monitor_get(o)?;
        Some(format!("{}:{}:{}", (*o).name(), monitor_target_string(target), format))
    }
}

/// A monitor target as written in `name:target:format`.
pub fn monitor_target_string(target: MonitorTarget) -> String {
    match target {
        MonitorTarget::Session => String::new(),
        MonitorTarget::Pane(id) => format!("%{id}"),
        MonitorTarget::AllPanes => "%*".to_string(),
        MonitorTarget::Window(id) => format!("@{id}"),
        MonitorTarget::AllWindows => "@*".to_string(),
    }
}

/// How often the monitor reported a change, and when it last did (0 for
/// never). Zero for an option without a monitor.
///
/// # Safety
/// `o` must be a valid entry.
pub unsafe fn hooks_monitor_fired(o: *mut options_entry) -> (u32, i64) {
    unsafe {
        let Some(monitor) = (*o).monitor() else {
            return (0, 0);
        };
        let name = (*o).name();
        (monitor.set.fire_count(name), monitor.set.fire_time(name))
    }
}
