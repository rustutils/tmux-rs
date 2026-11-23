// Copyright (c) 2012 Nicholas Marriott <nicholas.marriott@gmail.com>
// Copyright (c) 2012 George Nachman <tmux@georgester.com>
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

//! Control mode notifications (`%window-add @1`) for events.
//!
//! The control sink is registered for the events in [`CONTROL_EVENTS`] when
//! the server starts. [`control_notify_event`] gets each such event with
//! its payload and writes the notification to every control client that
//! should see it. The text and the rules for who gets what follow tmux
//! 3.8's control-notify.c.

use crate::events::{Payload, Sink, events_add_sink};
use crate::*;

/// The events control clients are told about.
const CONTROL_EVENTS: [&str; 14] = [
    "pane-mode-changed",
    "window-layout-changed",
    "window-pane-changed",
    "window-unlinked",
    "window-linked",
    "window-renamed",
    "client-session-changed",
    "client-detached",
    "session-renamed",
    "session-created",
    "session-closed",
    "session-window-changed",
    "paste-buffer-changed",
    "paste-buffer-deleted",
];

/// Registers the control sink for its events, at server start.
pub fn control_build_events() {
    for name in CONTROL_EVENTS {
        events_add_sink(name, Sink::Control);
    }
}

/// Whether `c` is a control client that is set up and not exiting.
///
/// # Safety
/// `c` must be a valid client.
unsafe fn control_should_notify(c: *mut client) -> bool {
    unsafe {
        (*c).flags.intersects(client_flag::CONTROL)
            && !(*c).flags.intersects(client_flag::EXIT)
            && (*c).control_state.is_some()
    }
}

/// The control clients to notify, attached to a session if
/// `need_session`.
///
/// # Safety
/// The client registry must hold valid clients.
unsafe fn control_clients(need_session: bool) -> Vec<*mut client> {
    unsafe {
        clients_iter()
            .filter(|&c| control_should_notify(c))
            .filter(|&c| !need_session || !client_get_session(c).is_null())
            .collect()
    }
}

/// Writes a notification line to every control client that should see
/// it, `attached` limiting it to clients with a session.
///
/// # Safety
/// The client registry must hold valid clients.
unsafe fn control_notify_all(line: &str, attached: bool) {
    unsafe {
        for c in control_clients(attached) {
            control_write!(c, "{}", line);
        }
    }
}

/// The control sink: writes the notification for an event.
///
/// # Safety
/// The client registry must hold valid clients, and the payload's entities
/// must be valid or gone from their registries.
pub unsafe fn control_notify_event(name: &str, payload: &Payload) {
    unsafe {
        let window = || payload.get_window("window").and_then(|id| window_from_id(id));
        match name {
            "pane-mode-changed" => {
                if let Some(value) = payload.print("pane") {
                    control_notify_all(&format!("%pane-mode-changed {value}"), false);
                }
            }
            "window-layout-changed" => {
                if let Some(w) = window() {
                    control_notify_layout_changed(w);
                }
            }
            "window-pane-changed" => {
                let Some(w) = window() else { return };
                let active = window_active_pane(w);
                if active.is_null() {
                    return;
                }
                let line = format!("%window-pane-changed @{} %{}", (*w).id, (*active).id);
                control_notify_all(&line, false);
            }
            "window-unlinked" | "window-linked" | "window-renamed" => {
                if let Some(w) = window() {
                    control_notify_window(name, w);
                }
            }
            "client-session-changed" => {
                let Some(cc) = payload.get_client("client").and_then(|id| client_from_id(id)) else {
                    return;
                };
                let s = client_get_session(cc);
                if s.is_null() {
                    return;
                }
                for c in control_clients(true) {
                    if c == cc {
                        control_write!(c, "%session-changed ${} {}", (*s).id, (*s).name);
                    } else {
                        control_write!(
                            c,
                            "%client-session-changed {} ${} {}",
                            _s((*cc).name),
                            (*s).id,
                            (*s).name
                        );
                    }
                }
            }
            "client-detached" => {
                if let Some(cc) = payload.get_client("client").and_then(|id| client_from_id(id)) {
                    let line = format!("%client-detached {}", _s((*cc).name));
                    control_notify_all(&line, false);
                }
            }
            "session-renamed" => {
                if let Some(s) = payload.get_session("session").and_then(|id| session_from_id(id)) {
                    let line = format!("%session-renamed ${} {}", (*s).id, (*s).name);
                    control_notify_all(&line, false);
                }
            }
            "session-created" | "session-closed" => control_notify_all("%sessions-changed", false),
            "session-window-changed" => {
                // A session being destroyed has no current window any more.
                let Some(s) = payload.get_session("session").and_then(|id| session_from_id(id)) else {
                    return;
                };
                if (*s).curw.is_null() {
                    return;
                }
                let line = format!(
                    "%session-window-changed ${} @{}",
                    (*s).id,
                    (*winlink_window((*s).curw)).id
                );
                control_notify_all(&line, false);
            }
            "paste-buffer-changed" | "paste-buffer-deleted" => {
                if let Some(buffer) = payload.get_string("paste_buffer") {
                    control_notify_all(&format!("%{name} {buffer}"), false);
                }
            }
            _ => {}
        }
    }
}

/// %layout-change for every control client whose session has the window,
/// formatted for that client.
///
/// # Safety
/// `w` must be a valid window and the client registry hold valid clients.
unsafe fn control_notify_layout_changed(w: *mut window) {
    const TEMPLATE: &str =
        "%layout-change #{window_id} #{window_layout} #{window_visible_layout} #{window_raw_flags}";
    unsafe {
        // When the last pane in a window is closed it has no layout root,
        // and the window goes away soon, so there is nothing to tell.
        if (*w).winlinks.is_empty() || window_layout_root(w).is_null() {
            return;
        }
        for c in control_clients(true) {
            let s = client_get_session(c);
            let Some(wl) = winlink_find_by_window(&raw mut (*s).windows, w) else {
                continue;
            };
            let cp = format_single(null_mut(), TEMPLATE, c, s, wl.as_ptr(), null_mut());
            control_write!(c, "{}", _s(cp));
            free_(cp);
        }
    }
}

/// %window-add, %window-close or %window-renamed for clients whose session
/// has the window, the %unlinked-window-* form for the others.
///
/// # Safety
/// `w` must be a valid window and the client registry hold valid clients.
unsafe fn control_notify_window(event: &str, w: *mut window) {
    unsafe {
        let (kind, suffix) = match event {
            "window-linked" => ("window-add", String::new()),
            "window-unlinked" => ("window-close", String::new()),
            _ => (
                "window-renamed",
                format!(" {}", (*w).name.as_deref().unwrap_or("")),
            ),
        };
        for c in control_clients(true) {
            let cs = client_get_session(c);
            if winlink_find_by_window_id(&raw mut (*cs).windows, (*w).id).is_null() {
                control_write!(c, "%unlinked-{} @{}{}", kind, (*w).id, suffix);
            } else {
                control_write!(c, "%{} @{}{}", kind, (*w).id, suffix);
            }
        }
    }
}
