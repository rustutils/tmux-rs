// Copyright (c) 2026 Dane Jensen <dhcjensen@gmail.com>
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
//! Sorted lists of entities (the `sort_get_*` functions of sort.c).
//!
//! Each function collects items from the registries in the order C walks
//! them, builds the [`crate::sort`] key for each, and sorts. The keys are
//! the only place entity fields are read for sorting, so the comparators
//! in sort.rs stay safe and testable.

use crate::libc::timeval;
use crate::sort::{
    BufferKey, ClientKey, KeyBindingKey, PaneKey, SessionKey, SortCriteria, SortError, Timestamp,
    WinlinkKey,
    sort_by,
};
use crate::*;

/// A timeval as a sort timestamp.
#[expect(
    clippy::useless_conversion,
    reason = "tv_usec is i32 on macOS, tv_sec is i64 everywhere so this always fires"
)]
fn timestamp(tv: &timeval) -> Timestamp {
    Timestamp {
        sec: i64::from(tv.tv_sec),
        usec: i64::from(tv.tv_usec),
    }
}

/// Paste buffers, given newest first, sorted by `criteria`.
pub fn sort_buffers(buffers: Vec<PasteBuffer>, criteria: &SortCriteria) -> Vec<PasteBuffer> {
    let mut l: Vec<(BufferKey, PasteBuffer)> = buffers
        .into_iter()
        .map(|pb| {
            let key = BufferKey {
                name: pb.name().to_string(),
                order: pb.order(),
                size: pb.data().len(),
            };
            (key, pb)
        })
        .collect();
    sort_by(&mut l, criteria, |(key, _)| key);
    l.into_iter().map(|(_, pb)| pb).collect()
}

/// Attached clients, in the order they connected unless sorted otherwise.
/// Clients that are exiting, suspended or not attached are left out.
///
/// # Safety
/// The client registry must hold valid clients.
pub unsafe fn sort_get_clients(criteria: &SortCriteria) -> Vec<*mut client> {
    unsafe {
        let mut l: Vec<(ClientKey, *mut client)> = clients_iter()
            .filter(|&c| {
                !(*c).flags.intersects(CLIENT_UNATTACHEDFLAGS)
                    && (*c).flags.contains(client_flag::ATTACHED)
            })
            .map(|c| {
                let key = ClientKey {
                    name: cstr_to_str((*c).name).to_string(),
                    sx: (*c).tty.sx,
                    sy: (*c).tty.sy,
                    creation: timestamp(&(*c).creation_time),
                    activity: timestamp(&(*c).activity_time),
                };
                (key, c)
            })
            .collect();
        sort_by(&mut l, criteria, |(key, _)| key);
        l.into_iter().map(|(_, c)| c).collect()
    }
}

/// The sort key of a session.
///
/// # Safety
/// `s` must be a valid session.
unsafe fn session_key(s: *mut session) -> SessionKey {
    unsafe {
        SessionKey {
            id: (*s).id,
            name: (*s).name.to_string(),
            creation: timestamp(&(*s).creation_time),
            activity: timestamp(&(*s).activity_time),
        }
    }
}

/// Sessions, by name unless sorted otherwise.
///
/// # Safety
/// The session registry must hold valid sessions.
pub unsafe fn sort_get_sessions(criteria: &SortCriteria) -> Vec<*mut session> {
    unsafe {
        let mut l: Vec<(SessionKey, *mut session)> =
            sessions_iter().map(|s| (session_key(s), s)).collect();
        sort_by(&mut l, criteria, |(key, _)| key);
        l.into_iter().map(|(_, s)| s).collect()
    }
}

/// The panes of a window, in layout order unless sorted otherwise.
///
/// # Safety
/// `w` must be a valid window whose panes are valid.
pub unsafe fn sort_get_panes_window(w: *mut window, criteria: &SortCriteria) -> Vec<*mut window_pane> {
    unsafe {
        let mut l: Vec<(PaneKey, *mut window_pane)> = (*w)
            .panes
            .iter()
            .zip(0u32..)
            .map(|(&wp, index)| {
                let key = PaneKey {
                    id: (*wp).id,
                    active_point: (*wp).active_point,
                    size: u64::from((*wp).sx) * u64::from((*wp).sy),
                    index,
                    // Without floating panes every pane is tiled, and C
                    // gives each tiled pane z index 1.
                    z: 1,
                    title: (*(*wp).screen).title.to_string_lossy().into_owned(),
                };
                (key, wp)
            })
            .collect();
        sort_by(&mut l, criteria, |(key, _)| key);
        l.into_iter().map(|(_, wp)| wp).collect()
    }
}

/// The sort key of a winlink. None if its window is gone.
///
/// # Safety
/// `wl` must be a valid winlink.
unsafe fn winlink_key(wl: *mut winlink) -> Option<WinlinkKey> {
    unsafe {
        let w = winlink_window(wl);
        if w.is_null() {
            return None;
        }
        Some(WinlinkKey {
            idx: (*wl).idx,
            name: (*w).name.clone().unwrap_or_default(),
            creation: timestamp(&(*w).creation_time),
            activity: timestamp(&(*w).activity_time),
            size: u64::from((*w).sx) * u64::from((*w).sy),
        })
    }
}

/// Collects the keys of winlinks and sorts them.
///
/// # Safety
/// Every winlink must be valid.
unsafe fn sort_winlinks(
    winlinks: impl Iterator<Item = *mut winlink>,
    criteria: &SortCriteria,
) -> Vec<*mut winlink> {
    unsafe {
        let mut l: Vec<(WinlinkKey, *mut winlink)> = winlinks
            .filter_map(|wl| Some((winlink_key(wl)?, wl)))
            .collect();
        sort_by(&mut l, criteria, |(key, _)| key);
        l.into_iter().map(|(_, wl)| wl).collect()
    }
}

/// The winlinks of every session, sessions by name and windows by index
/// unless sorted otherwise.
///
/// # Safety
/// The session registry must hold valid sessions.
pub unsafe fn sort_get_winlinks(criteria: &SortCriteria) -> Vec<*mut winlink> {
    unsafe {
        let all: Vec<*mut winlink> = sessions_iter()
            .flat_map(|s| (*s).windows.values().copied().collect::<Vec<_>>())
            .collect();
        sort_winlinks(all.into_iter(), criteria)
    }
}

/// The winlinks of one session, by index unless sorted otherwise.
///
/// # Safety
/// `s` must be a valid session.
pub unsafe fn sort_get_winlinks_session(s: *mut session, criteria: &SortCriteria) -> Vec<*mut winlink> {
    unsafe {
        let all: Vec<*mut winlink> = (*s).windows.values().copied().collect();
        sort_winlinks(all.into_iter(), criteria)
    }
}

/// A key binding and the table it is in.
#[derive(Clone, Copy)]
pub struct TableBinding {
    /// The key table.
    pub table: *mut key_table,
    /// The binding.
    pub bd: *mut key_binding,
}

/// Collects the bindings of tables and sorts them.
///
/// # Safety
/// Every table must be valid.
unsafe fn sort_bindings(tables: &[*mut key_table], criteria: &SortCriteria) -> Vec<TableBinding> {
    unsafe {
        let mut l: Vec<(KeyBindingKey, TableBinding)> = Vec::new();
        for &table in tables {
            for bd in key_bindings_entries(table) {
                let key = KeyBindingKey {
                    table: (*table).name.clone(),
                    key: (*bd).key,
                    modifiers: (*bd).key & KEYC_MASK_MODIFIERS,
                };
                l.push((key, TableBinding { table, bd }));
            }
        }
        sort_by(&mut l, criteria, |(key, _)| key);
        l.into_iter().map(|(_, tb)| tb).collect()
    }
}

/// The bindings of every table, tables by name and bindings by key unless
/// sorted otherwise.
///
/// # Safety
/// The key tables must be valid.
pub unsafe fn sort_get_key_bindings(criteria: &SortCriteria) -> Vec<TableBinding> {
    unsafe { sort_bindings(&key_tables_entries(), criteria) }
}

/// The bindings of one table, by key unless sorted otherwise.
///
/// # Safety
/// `table` must be a valid key table.
pub unsafe fn sort_get_key_bindings_table(
    table: *mut key_table,
    criteria: &SortCriteria,
) -> Vec<TableBinding> {
    unsafe { sort_bindings(&[table], criteria) }
}

/// Sort criteria from a command's -O and -r flags. A name that is not
/// UTF-8 is an unknown order.
///
/// # Safety
/// `args` must be valid parsed arguments.
pub unsafe fn args_sort_criteria(args: *mut args) -> Result<SortCriteria, SortError> {
    unsafe {
        let order = args_get(args, b'O');
        let order = if order.is_null() {
            None
        } else {
            Some(CStr::from_ptr(order.cast()).to_str().unwrap_or(""))
        };
        SortCriteria::from_args(order, args_has(args, 'r'))
    }
}
