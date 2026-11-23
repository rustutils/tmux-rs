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

//! Format monitors for the rest of tmux.
//!
//! [`MonitorSet`] in [`store`] decides which values changed and is safe
//! Rust. This file expands the formats: [`monitor_observe`] finds every
//! value the items of a set watch, in the order tmux 3.8 checks them, and
//! the owner of the set passes the values to [`MonitorSet::check`] and
//! reports what it returns. Control mode does this once a second for its
//! subscriptions (control.rs).
//!
//! Formats are expanded without running jobs: `#()` gives nothing.

pub mod store;

pub use self::store::{
    MonitorChange, MonitorFlags, MonitorKey, MonitorSet, MonitorTarget, Observation, monitor_parse,
};

use crate::*;

/// An item of a set, copied out so the set is not borrowed while formats
/// are expanded.
pub struct MonitorItem {
    pub name: String,
    pub target: MonitorTarget,
    pub format: String,
}

/// Copies the items out of a set.
pub fn monitor_items(set: &MonitorSet) -> Vec<MonitorItem> {
    set.items()
        .map(|(name, target, format)| MonitorItem {
            name: name.to_string(),
            target,
            format: format.to_string(),
        })
        .collect()
}

/// A format tree for a check, without jobs, as C's
/// `monitor_create_formats`.
///
/// # Safety
/// `c` must be null or valid, the others null or valid.
unsafe fn monitor_create_formats(
    c: *mut client,
    s: *mut session,
    wl: *mut winlink,
    wp: *mut window_pane,
) -> *mut format_tree {
    unsafe {
        let ft = format_create(null_mut(), null_mut(), 0, format_flags::FORMAT_NOJOBS);
        format_defaults(ft, c, NonNull::new(s), NonNull::new(wl), NonNull::new(wp));
        ft
    }
}

/// Expands `format` with a format tree.
///
/// # Safety
/// `ft` must be valid.
unsafe fn monitor_expand(ft: *mut format_tree, format: &str) -> String {
    unsafe {
        let format = CString::new(format).unwrap_or_default();
        let value = format_expand(ft, format.as_ptr().cast());
        let out = cstr_to_string_lossy(value);
        free_(value);
        out
    }
}

/// The values the items watch in session `s`, as seen by client `c`, in
/// the order C checks them: session items, then single panes and windows
/// (at each index the window has in the session), then every pane, then
/// every window. Within a step items go by name.
///
/// # Safety
/// `c` must be null or valid and `s` a valid live session.
pub unsafe fn monitor_observe(
    items: &[MonitorItem],
    c: *mut client,
    s: *mut session,
) -> Vec<Observation> {
    unsafe {
        let mut out = Vec::new();
        let mut observe = |item: &MonitorItem, key, ft| {
            out.push(Observation {
                name: item.name.clone(),
                key,
                value: monitor_expand(ft, &item.format),
            });
        };
        let session_id = Some(SessionId((*s).id));

        if items.iter().any(|i| i.target == MonitorTarget::Session) {
            let ft = monitor_create_formats(c, s, null_mut(), null_mut());
            for item in items.iter().filter(|i| i.target == MonitorTarget::Session) {
                observe(item, MonitorKey::Session, ft);
            }
            format_free(ft);
        }

        for item in items {
            match item.target {
                MonitorTarget::Pane(id) => {
                    let wp = window_pane_find_by_id(id);
                    if wp.is_null() || (*wp).fd == -1 {
                        continue;
                    }
                    let w = window_pane_window(wp);
                    for &wl in &(*w).winlinks {
                        if (*wl).session != session_id {
                            continue;
                        }
                        let ft = monitor_create_formats(c, s, wl, wp);
                        let key = MonitorKey::Pane {
                            pane: id,
                            index: (*wl).idx,
                        };
                        observe(item, key, ft);
                        format_free(ft);
                    }
                }
                MonitorTarget::Window(id) => {
                    let w = window_find_by_id(id);
                    if w.is_null() {
                        continue;
                    }
                    for &wl in &(*w).winlinks {
                        if (*wl).session != session_id {
                            continue;
                        }
                        let ft = monitor_create_formats(c, s, wl, null_mut());
                        let key = MonitorKey::Window {
                            window: id,
                            index: (*wl).idx,
                        };
                        observe(item, key, ft);
                        format_free(ft);
                    }
                }
                _ => {}
            }
        }

        let winlinks: Vec<*mut winlink> = (*s).windows.values().copied().collect();
        if items.iter().any(|i| i.target == MonitorTarget::AllPanes) {
            for &wl in &winlinks {
                let w = winlink_window(wl);
                for &wp in &(*w).panes {
                    let ft = monitor_create_formats(c, s, wl, wp);
                    let key = MonitorKey::Pane {
                        pane: (*wp).id,
                        index: (*wl).idx,
                    };
                    for item in items.iter().filter(|i| i.target == MonitorTarget::AllPanes) {
                        observe(item, key, ft);
                    }
                    format_free(ft);
                }
            }
        }
        if items.iter().any(|i| i.target == MonitorTarget::AllWindows) {
            for &wl in &winlinks {
                let ft = monitor_create_formats(c, s, wl, null_mut());
                let key = MonitorKey::Window {
                    window: (*winlink_window(wl)).id,
                    index: (*wl).idx,
                };
                for item in items.iter().filter(|i| i.target == MonitorTarget::AllWindows) {
                    observe(item, key, ft);
                }
                format_free(ft);
            }
        }
        out
    }
}

/// The current time in seconds, for fire times.
pub fn monitor_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
