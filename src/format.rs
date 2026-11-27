// Copyright (c) 2011 Nicholas Marriott <nicholas.marriott@gmail.com>
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
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ffi::CStr;

use crate::format_env::{format_bytes, format_engine_expand, format_to_c};

use crate::compat::HOST_NAME_MAX;
use crate::libc::{getpwuid, getuid, localtime_r, strchr, strftime, tm};
use crate::*;
use crate::options_::*;

bitflags::bitflags! {
    #[repr(transparent)]
    #[derive(Copy, Clone)]
    pub struct format_flags: i32 {
        const FORMAT_STATUS  = 1;
        const FORMAT_FORCE   = 2;
        const FORMAT_NOJOBS  = 4;
        const FORMAT_VERBOSE = 8;
    }
}

pub const FORMAT_NONE: i32 = 0;
pub const FORMAT_PANE: u32 = 0x80000000u32;
pub const FORMAT_WINDOW: u32 = 0x40000000u32;

pub type format_cb = unsafe fn(_: &format_tree) -> format_table_type;

#[repr(i32)]
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum format_type {
    FORMAT_TYPE_UNKNOWN,
    FORMAT_TYPE_SESSION,
    FORMAT_TYPE_WINDOW,
    FORMAT_TYPE_PANE,
}

/// An entry that `format_add`, `format_add_tv` or `format_add_cb` put in a
/// format tree.
pub enum format_entry {
    /// A value, as bytes.
    Value(Vec<u8>),
    /// A time in seconds (`format_add_tv`).
    Time(time_t),
    /// A callback, run the first time the entry is looked up and kept.
    Callback(format_cb, std::cell::OnceCell<Vec<u8>>),
}

pub struct format_tree {
    pub type_: format_type,

    pub c: *mut client,
    pub s: Option<SessionId>,
    pub wl: Option<Winlink>,
    pub w: Option<WindowId>,
    pub wp: Option<PaneId>,
    /// The paste buffer the buffer_* formats describe, a copy taken when
    /// the tree was set up.
    pub pb: Option<PasteBuffer>,

    pub item: *mut cmdq_item,
    pub client: *mut client,
    pub flags: format_flags,
    pub tag: u32,

    pub m: mouse_event,

    /// The `format_add` entries by name, in name order as C's tree.
    pub tree: BTreeMap<Vec<u8>, format_entry>,
}

/// Is logging enabled?
pub fn format_logging(ft: &format_tree) -> bool {
    log_get_level() != 0 || ft.flags.intersects(format_flags::FORMAT_VERBOSE)
}

pub unsafe fn format_cb_host(_ft: &format_tree) -> format_table_type {
    unsafe {
        let mut host = MaybeUninit::<[u8; HOST_NAME_MAX + 1]>::uninit();

        if libc::gethostname(host.as_mut_ptr().cast(), HOST_NAME_MAX + 1) != 0 {
            "".into()
        } else {
            format!("{}", _s(host.as_ptr().cast::<u8>())).into()
        }
    }
}

/// Callback for `host_short`.
pub unsafe fn format_cb_host_short(_ft: &format_tree) -> format_table_type {
    unsafe {
        let mut host = MaybeUninit::<[u8; HOST_NAME_MAX + 1]>::uninit();

        if libc::gethostname(host.as_mut_ptr().cast(), HOST_NAME_MAX + 1) != 0 {
            return "".into();
        }

        let cp = strchr(host.as_mut_ptr().cast(), b'.' as i32);
        if !cp.is_null() {
            *cp = b'\0';
        }
        format!("{}", _s(&raw const host as *const u8)).into()
    }
}

/// Callback for pid.
pub unsafe fn format_cb_pid(_ft: &format_tree) -> format_table_type {
    unsafe { format!("{}", libc::getpid()).into() }
}

/// Callback for `session_attached_list`.
pub unsafe fn format_cb_session_attached_list(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());

        if s.is_null() {
            return format_table_type::None;
        }

        let buffer = evbuffer_new();
        if buffer.is_null() {
            fatalx("out of memory");
        }

        for loop_ in clients_iter() {
            if client_get_session(loop_) == s {
                if EVBUFFER_LENGTH(buffer) > 0 {
                    evbuffer_add(buffer, c!(",").cast(), 1);
                }
                evbuffer_add_printf!(buffer, "{}", _s((*loop_).name));
            }
        }

        let size = EVBUFFER_LENGTH(buffer);
        let result = if size != 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(EVBUFFER_DATA(buffer), size)).into_owned().into()
        } else {
            format_table_type::None
        };
        evbuffer_free(buffer);
        result
    }
}

/// Callback for `session_alerts`.
pub unsafe fn format_cb_session_alerts(ft: &format_tree) -> format_table_type {
    unsafe {
        let s: *mut session = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        const SIZEOF_ALERTS: usize = 1024;
        const SIZEOF_TMP: usize = 16;
        let mut alerts = MaybeUninit::<[u8; 1024]>::uninit();
        let alerts: *mut u8 = alerts.as_mut_ptr().cast();
        let mut tmp = MaybeUninit::<[u8; 16]>::uninit();
        let tmp: *mut u8 = tmp.as_mut_ptr().cast();

        if s.is_null() {
            return format_table_type::None;
        }

        *alerts = b'\0';
        for wl in (*s).windows() {
            if !wl.flags.intersects(WINLINK_ALERTFLAGS) {
                continue;
            }
            _ = xsnprintf_!(tmp, SIZEOF_TMP, "{}", wl.idx);

            if *alerts != b'\0' {
                strlcat(alerts, c!(","), SIZEOF_ALERTS);
            }
            strlcat(alerts, tmp, SIZEOF_ALERTS);
            if wl.flags.intersects(winlink_flags::WINLINK_ACTIVITY) {
                strlcat(alerts, c!("#"), SIZEOF_ALERTS);
            }
            if wl.flags.intersects(winlink_flags::WINLINK_BELL) {
                strlcat(alerts, c!("!"), SIZEOF_ALERTS);
            }
            if wl.flags.intersects(winlink_flags::WINLINK_SILENCE) {
                strlcat(alerts, c!("~"), SIZEOF_ALERTS);
            }
        }
        format!("{}", _s(alerts)).into()
    }
}

/// Callback for `session_stack`.
pub unsafe fn format_cb_session_stack(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        const SIZEOF_RESULT: usize = 1024;
        const SIZEOF_TMP: usize = 16;

        let mut result = MaybeUninit::<[u8; 1024]>::uninit();
        let result: *mut u8 = result.as_mut_ptr().cast();
        let mut tmp = MaybeUninit::<[u8; 16]>::uninit();
        let tmp: *mut u8 = tmp.as_mut_ptr().cast();

        if s.is_null() {
            return format_table_type::None;
        }

        _ = xsnprintf_!(result, SIZEOF_RESULT, "{}", (*s).curw().unwrap().idx);
        for wl in (*s).lastw() {
            _ = xsnprintf_!(tmp, SIZEOF_TMP, "{}", wl.idx);

            if *result != b'\0' {
                strlcat(result, c!(","), SIZEOF_RESULT);
            }
            strlcat(result, tmp, SIZEOF_RESULT);
        }
        format!("{}", _s(result)).into()
    }
}

/// Callback for `window_stack_index`.
pub unsafe fn format_cb_window_stack_index(ft: &format_tree) -> format_table_type {
    unsafe {
        let Some(wl) = ft.wl else {
            return format_table_type::None;
        };
        let s = session_from_id(wl.session).unwrap_or(null_mut());
        if s.is_null() { return "0".into(); }

        // Its place on the stack counting from 1, 0 if it is not on it.
        let id = wl.id;
        match (*s).lastw().iter().position(|wl| wl.id == id) {
            Some(i) => format!("{}", i + 1).into(),
            None => "0".into(),
        }
    }
}

/// Callback for `window_linked_sessions_list`.
pub unsafe fn format_cb_window_linked_sessions_list(ft: &format_tree) -> format_table_type {
    unsafe {
        let Some(wl) = ft.wl else {
            return format_table_type::None;
        };
        let w = winlink_window(wl);

        let buffer = evbuffer_new();
        if buffer.is_null() {
            fatalx("out of memory");
        }

        for wl in (*w).winlinks() {
            if EVBUFFER_LENGTH(buffer) > 0 {
                evbuffer_add(buffer, c!(",").cast(), 1);
            }
            let s = session_from_id(wl.session).unwrap_or(null_mut());
            if !s.is_null() { evbuffer_add_printf!(buffer, "{}", (*s).name); }
        }

        let size = EVBUFFER_LENGTH(buffer);
        let mut value = format_table_type::None;
        if size != 0 {
            value = format_table_type::String(
                String::from_utf8_lossy(std::slice::from_raw_parts(EVBUFFER_DATA(buffer), size)).into_owned().into(),
            );
        }
        evbuffer_free(buffer);
        value
    }
}

/// Callback for `window_active_sessions`.
pub unsafe fn format_cb_window_active_sessions(ft: &format_tree) -> format_table_type {
    unsafe {
        let Some(wl) = ft.wl else {
            return format_table_type::None;
        };
        let w = winlink_window(wl);

        let n = (*w).winlinks().iter()
            .filter(|wl| {
                let s = session_from_id(wl.session).unwrap_or(null_mut());
                !s.is_null() && (*s).curw_id() == Some(wl.id)
            })
            .count() as u32;

        format!("{n}").into()
    }
}

/// Callback for `window_active_sessions_list`.
pub unsafe fn format_cb_window_active_sessions_list(ft: &format_tree) -> format_table_type {
    unsafe {
        let Some(wl) = ft.wl else {
            return format_table_type::None;
        };
        let w = winlink_window(wl);

        let buffer = evbuffer_new();
        if buffer.is_null() {
            fatalx("out of memory");
        }

        for wl in (*w).winlinks() {
            let s = session_from_id(wl.session).unwrap_or(null_mut());
            if !s.is_null() && (*s).curw_id() == Some(wl.id) {
                if EVBUFFER_LENGTH(buffer) > 0 {
                    evbuffer_add(buffer, c!(",").cast(), 1);
                }
                evbuffer_add_printf!(buffer, "{}", (*s).name);
            }
        }

        let size = EVBUFFER_LENGTH(buffer);
        let mut value = format_table_type::None;
        if size != 0 {
            value = format_table_type::String(
                String::from_utf8_lossy(std::slice::from_raw_parts(EVBUFFER_DATA(buffer), size)).into_owned().into(),
            );
        }
        evbuffer_free(buffer);
        value
    }
}

/// Callback for `window_active_clients`.
pub unsafe fn format_cb_window_active_clients(ft: &format_tree) -> format_table_type {
    unsafe {
        let Some(wl) = ft.wl else {
            return format_table_type::None;
        };
        let w = winlink_window(wl);

        let mut n = 0u32;
        for loop_ in clients_iter() {
            let client_session = client_get_session(loop_);
            if client_session.is_null() {
                continue;
            }

            let curw_w = winlink_window((*client_session).curw_id());
            if w == curw_w {
                n += 1;
            }
        }

        format!("{n}").into()
    }
}

/// Callback for `window_active_clients_list`.
pub unsafe fn format_cb_window_active_clients_list(ft: &format_tree) -> format_table_type {
    unsafe {
        let Some(wl) = ft.wl else {
            return format_table_type::None;
        };
        let w = winlink_window(wl);

        let buffer = evbuffer_new();
        if buffer.is_null() {
            fatalx("out of memory");
        }

        for loop_ in clients_iter() {
            let client_session = client_get_session(loop_);
            if client_session.is_null() {
                continue;
            }

            let curw_w = winlink_window((*client_session).curw_id());
            if w == curw_w {
                if EVBUFFER_LENGTH(buffer) > 0 {
                    evbuffer_add(buffer, c!(",").cast(), 1);
                }
                evbuffer_add_printf!(buffer, "{}", _s((*loop_).name));
            }
        }

        let size = EVBUFFER_LENGTH(buffer);
        let mut value = format_table_type::None;
        if size != 0 {
            value = format_table_type::String(
                String::from_utf8_lossy(std::slice::from_raw_parts(EVBUFFER_DATA(buffer), size)).into_owned().into(),
            );
        }
        evbuffer_free(buffer);
        value
    }
}

/// Callback for `window_layout`.
pub unsafe fn format_cb_window_layout(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());

        if w.is_null() {
            return format_table_type::None;
        }

        if !window_saved_layout_root(w).is_null() {
            return layout_dump(w, window_saved_layout_root(w))
                .map(Into::into)
                .unwrap_or_default();
        }
        layout_dump(w, window_layout_root(w))
            .map(Into::into)
            .unwrap_or_default()
    }
}

/// Callback for `window_visible_layout`.
pub unsafe fn format_cb_window_visible_layout(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());

        if w.is_null() {
            return format_table_type::None;
        }

        layout_dump(w, window_layout_root(w))
            .map(Into::into)
            .unwrap_or_default()
    }
}

/// Callback for `pane_start_command`.
pub unsafe fn format_cb_start_command(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());

        if wp.is_null() {
            return format_table_type::None;
        }

        cmd_stringify_argv((*wp).argc, (*wp).argv).into()
    }
}

/// Callback for `pane_start_path`.
pub unsafe fn format_cb_start_path(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());

        if wp.is_null() {
            return format_table_type::None;
        }

        match (*wp).cwd.as_deref() {
            None => "".into(),
            Some(p) => p.display().to_string().into(),
        }
    }
}

/// Callback for `pane_current_command`.
pub unsafe fn format_cb_current_command(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());

        if wp.is_null() || (*wp).shell.is_none() {
            return format_table_type::None;
        }

        let mut cmd = osdep_get_name((*wp).fd, (*wp).tty.as_ptr());
        if cmd.is_null() || *cmd == b'\0' {
            free_(cmd);
            cmd = CString::new(cmd_stringify_argv((*wp).argc, (*wp).argv))
                .unwrap()
                .into_raw()
                .cast();
            if cmd.is_null() || *cmd == b'\0' {
                free_(cmd);
                let shell_c = std::ffi::CString::new(
                    (*wp).shell.as_deref().unwrap().to_string_lossy().as_bytes(),
                )
                .unwrap();
                cmd = xstrdup(shell_c.as_ptr().cast()).as_ptr().cast();
            }
        }
        let value = parse_window_name(cmd);
        free_(cmd);
        value.into()
    }
}

/// Callback for `pane_current_path`.
pub unsafe fn format_cb_current_path(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());

        if wp.is_null() {
            return format_table_type::None;
        }

        osdep_get_cwd((*wp).fd).map_or(format_table_type::None, |cwd| cwd.to_string_lossy().into_owned().into())
    }
}

/// Size of C's packed `struct grid_cell_entry`. `history_bytes` reports C's
/// sizes on LP64 so it prints what 3.8 prints, not this port's layout.
const C_GRID_CELL_ENTRY_SIZE: usize = 5;
/// Size of C's packed `struct grid_extd_entry`.
const C_GRID_EXTD_ENTRY_SIZE: usize = 23;
/// Size of C's `struct grid_line` on LP64.
const C_GRID_LINE_SIZE: usize = 40;

/// Callback for `history_bytes`, the memory C would use for the grid.
pub unsafe fn format_cb_history_bytes(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());

        if wp.is_null() {
            return format_table_type::None;
        }

        let gd: *mut Grid = &raw mut *(*wp).base.grid;
        let mut size: usize = 0;

        for i in 0..((*gd).hsize + (*gd).sy) {
            let gl = (*gd).get_line(i);
            size += gl.celldata.len() * C_GRID_CELL_ENTRY_SIZE;
            size += gl.extddata.len() * C_GRID_EXTD_ENTRY_SIZE;
        }
        size += ((*gd).hsize + (*gd).sy) as usize * C_GRID_LINE_SIZE;

        format!("{size}").into()
    }
}

/// Callback for `history_all_bytes`.
pub unsafe fn format_cb_history_all_bytes(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());

        if wp.is_null() {
            return format_table_type::None;
        }

        let gd: *mut Grid = &raw mut *(*wp).base.grid;
        let lines = (*gd).hsize + (*gd).sy;
        let mut cells = 0;
        let mut extended_cells = 0;

        for i in 0..lines {
            let gl: *mut GridLine = (*gd).get_line(i);
            cells += (*gl).celldata.len() as u32;
            extended_cells += (*gl).extddata.len() as u32;
        }

        format!(
            "{},{},{},{},{},{}",
            lines,
            lines as usize * C_GRID_LINE_SIZE,
            cells,
            cells as usize * C_GRID_CELL_ENTRY_SIZE,
            extended_cells,
            extended_cells as usize * C_GRID_EXTD_ENTRY_SIZE,
        )
        .into()
    }
}

/// Callback for `pane_tabs`.
pub unsafe fn format_cb_pane_tabs(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());

        if wp.is_null() {
            return format_table_type::None;
        }

        let buffer = evbuffer_new();
        if buffer.is_null() {
            fatalx("out of memory");
        }

        let mut first = true;
        for i in 0..(*wp).base.grid.sx {
            if !(*wp).base.tabs.as_ref().unwrap().borrow().bit_test(i) {
                continue;
            }

            if !first {
                evbuffer_add(buffer, c!(",").cast(), 1);
            }
            evbuffer_add_printf!(buffer, "{i}");
            first = false;
        }

        let size = EVBUFFER_LENGTH(buffer);
        let result = if size != 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(EVBUFFER_DATA(buffer), size)).into_owned().into()
        } else {
            format_table_type::None
        };
        evbuffer_free(buffer);
        result
    }
}

/// Callback for `pane_fg`.
pub unsafe fn format_cb_pane_fg(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        let mut gc = MaybeUninit::<GridCell>::uninit();

        if wp.is_null() {
            return format_table_type::None;
        }

        tty_default_colours(gc.as_mut_ptr(), wp);

        colour_tostring((*gc.as_ptr()).fg).into()
    }
}

/// Callback for `pane_bg`.
pub unsafe fn format_cb_pane_bg(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        let mut gc = MaybeUninit::<GridCell>::uninit();

        if wp.is_null() {
            return format_table_type::None;
        }

        tty_default_colours(gc.as_mut_ptr(), wp);

        colour_tostring((*gc.as_ptr()).bg).into()
    }
}

/// Callback for `session_group_list`.
pub unsafe fn format_cb_session_group_list(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if s.is_null() {
            return format_table_type::None;
        }

        let Some(sg) = session_group_contains(s) else {
            return format_table_type::None;
        };

        let buffer = evbuffer_new();
        if buffer.is_null() {
            fatalx("out of memory");
        }

        for loop_ in session_group_sessions(&sg) {
            if EVBUFFER_LENGTH(buffer) > 0 {
                evbuffer_add(buffer, c!(",").cast(), 1);
            }
            evbuffer_add_printf!(buffer, "{}", (*loop_).name);
        }

        let size = EVBUFFER_LENGTH(buffer);
        let result = if size != 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(EVBUFFER_DATA(buffer), size)).into_owned().into()
        } else {
            format_table_type::None
        };
        evbuffer_free(buffer);
        result
    }
}

/// Callback for `session_group_attached_list`.
pub unsafe fn format_cb_session_group_attached_list(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if s.is_null() {
            return format_table_type::None;
        }

        let Some(sg) = session_group_contains(s) else {
            return format_table_type::None;
        };

        let buffer = evbuffer_new();
        if buffer.is_null() {
            fatalx("out of memory");
        }

        for loop_ in clients_iter() {
            let client_session = client_get_session(loop_);
            if client_session.is_null() {
                continue;
            }

            for session_loop in session_group_sessions(&sg) {
                if session_loop == client_session {
                    if EVBUFFER_LENGTH(buffer) > 0 {
                        evbuffer_add(buffer, c!(",").cast(), 1);
                    }
                    evbuffer_add_printf!(buffer, "{}", _s((*loop_).name));
                }
            }
        }

        let size = EVBUFFER_LENGTH(buffer);
        let result = if size != 0 {
            String::from_utf8_lossy(std::slice::from_raw_parts(EVBUFFER_DATA(buffer), size)).into_owned().into()
        } else {
            format_table_type::None
        };
        evbuffer_free(buffer);
        result
    }
}

/// Callback for `pane_in_mode`.
pub unsafe fn format_cb_pane_in_mode(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if wp.is_null() {
            return format_table_type::None;
        }

        let n = (*wp).modes.len() as u32;

        format!("{n}").into()
    }
}

/// Callback for `pane_at_top`.
pub unsafe fn format_cb_pane_at_top(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if wp.is_null() {
            return format_table_type::None;
        }

        let w = window_pane_window(wp);
        let status: i64 = options_get_number___(&*(*w).options, "pane-border-status");
        let flag = if status == pane_status::PANE_STATUS_TOP as i64 {
            (*wp).yoff == 1
        } else {
            (*wp).yoff == 0
        };

        if flag { "1" } else { "0" }.into()
    }
}

/// Callback for `pane_at_bottom`.
pub unsafe fn format_cb_pane_at_bottom(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if wp.is_null() {
            return format_table_type::None;
        }

        let w = window_pane_window(wp);
        let status: i64 = options_get_number___(&*(*w).options, "pane-border-status");
        let flag = if status == pane_status::PANE_STATUS_BOTTOM as i64 {
            (*wp).yoff + (*wp).sy == (*w).sy - 1
        } else {
            (*wp).yoff + (*wp).sy == (*w).sy
        };

        if flag { "1" } else { "0" }.into()
    }
}

/// Callback for `cursor_character`.
pub unsafe fn format_cb_cursor_character(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if wp.is_null() {
            return format_table_type::None;
        }
        let gc = (*wp).base.grid.view_get_cell((*wp).base.cx, (*wp).base.cy);
        if !gc.flags.intersects(GridFlag::PADDING) {
            format!(
                "{1:0$}",
                gc.data.size as usize,
                _s((&raw const gc.data.data).cast::<u8>())
            )
            .into()
        } else {
            format_table_type::None
        }
    }
}

/// Callback for `mouse_word`.
pub unsafe fn format_cb_mouse_word(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.m.valid {
            return format_table_type::None;
        }
        let Some(wp) = cmd_mouse_pane(&ft.m, null_mut(), null_mut()) else {
            return format_table_type::None;
        };
        let mut x = 0;
        let mut y = 0;
        if cmd_mouse_at(wp.as_ptr(), &ft.m, &mut x, &mut y, 0) != 0 {
            return format_table_type::None;
        }

        if !(*wp.as_ptr()).modes.is_empty() {
            if window_pane_mode(wp.as_ptr()) != WINDOW_PANE_NO_MODE {
                return window_copy_get_word(wp.as_ptr(), x, y).into();
            }
            return format_table_type::None;
        }
        let gd: *mut Grid = &raw mut *(*wp.as_ptr()).base.grid;
        format_grid_word(gd, x, (*gd).hsize + y).into()
    }
}

/// Callback for `mouse_hyperlink`.
pub unsafe fn format_cb_mouse_hyperlink(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.m.valid {
            return format_table_type::None;
        }
        let Some(wp) = cmd_mouse_pane(&ft.m, null_mut(), null_mut()) else {
            return format_table_type::None;
        };
        let mut x = 0;
        let mut y = 0;
        if cmd_mouse_at(wp.as_ptr(), &ft.m, &mut x, &mut y, 0) != 0 {
            return format_table_type::None;
        }
        let gd: *mut Grid = &raw mut *(*wp.as_ptr()).base.grid;
        format_grid_hyperlink(gd, x, (*gd).hsize + y, (*wp.as_ptr()).screen)
            .map(Into::into)
            .unwrap_or_default()
    }
}

/// Callback for `mouse_line`.
pub unsafe fn format_cb_mouse_line(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.m.valid {
            return format_table_type::None;
        }
        let Some(wp) = cmd_mouse_pane(&ft.m, null_mut(), null_mut()) else {
            return format_table_type::None;
        };
        let mut x = 0;
        let mut y = 0;
        if cmd_mouse_at(wp.as_ptr(), &ft.m, &mut x, &mut y, 0) != 0 {
            return format_table_type::None;
        }

        if !(*wp.as_ptr()).modes.is_empty() {
            if window_pane_mode(wp.as_ptr()) != WINDOW_PANE_NO_MODE {
                return window_copy_get_line(wp.as_ptr(), y).into();
            }
            return format_table_type::None;
        }
        let gd: *mut Grid = &raw mut *(*wp.as_ptr()).base.grid;
        format_grid_line(gd, (*gd).hsize + y).into()
    }
}

/// Callback for `mouse_status_line`.
pub unsafe fn format_cb_mouse_status_line(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.m.valid {
            return format_table_type::None;
        }
        if ft.c.is_null() || !(*ft.c).tty.flags.intersects(tty_flags::TTY_STARTED) {
            return format_table_type::None;
        }

        let y = if ft.m.statusat == 0 && ft.m.y < ft.m.statuslines {
            ft.m.y
        } else if ft.m.statusat > 0 && ft.m.y >= ft.m.statusat as u32 {
            ft.m.y - ft.m.statusat as u32
        } else {
            return format_table_type::None;
        };

        format!("{y}").into()
    }
}

/// Callback for `mouse_status_range`.
pub unsafe fn format_cb_mouse_status_range(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.m.valid {
            return format_table_type::None;
        }
        if ft.c.is_null() || !(*ft.c).tty.flags.intersects(tty_flags::TTY_STARTED) {
            return format_table_type::None;
        }

        
        
        let (x, y) = if ft.m.statusat == 0 && ft.m.y < ft.m.statuslines {
            (ft.m.x, ft.m.y)
        } else if ft.m.statusat > 0 && ft.m.y >= ft.m.statusat as u32 {
            (ft.m.x, ft.m.y - ft.m.statusat as u32)
        } else {
            return format_table_type::None;
        };

        let sr = status_get_range(ft.c, x, y);
        if sr.is_null() {
            return format_table_type::None;
        }

        match (*sr).type_ {
            style_range_type::STYLE_RANGE_NONE => format_table_type::None,
            style_range_type::STYLE_RANGE_LEFT => "left".into(),
            style_range_type::STYLE_RANGE_RIGHT => "right".into(),
            style_range_type::STYLE_RANGE_PANE => "pane".into(),
            style_range_type::STYLE_RANGE_WINDOW => "window".into(),
            style_range_type::STYLE_RANGE_SESSION => "session".into(),
            style_range_type::STYLE_RANGE_USER => format!("{}", _s((*sr).string.as_ptr())).into(),
        }
    }
}

pub unsafe fn format_cb_alternate_on(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.saved_grid.is_some() {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_alternate_saved_x(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.saved_cx).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_alternate_saved_y(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.saved_cy).into();
        }
        format_table_type::None
    }
}

pub fn format_cb_buffer_name(ft: &format_tree) -> format_table_type {
    ft.pb.as_ref().map_or(format_table_type::None, |pb| pb.name().to_string().into())
}

pub fn format_cb_buffer_sample(ft: &format_tree) -> format_table_type {
    ft.pb.as_ref().map_or(format_table_type::None, |pb| pb.sample().into())
}

pub fn format_cb_buffer_size(ft: &format_tree) -> format_table_type {
    ft.pb.as_ref().map_or(format_table_type::None, |pb| pb.data().len().to_string().into())
}

pub unsafe fn format_cb_client_cell_height(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() && (*ft.c).tty.flags.intersects(tty_flags::TTY_STARTED) {
            return format!("{}", (*ft.c).tty.ypixel).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_cell_width(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() && (*ft.c).tty.flags.intersects(tty_flags::TTY_STARTED) {
            return format!("{}", (*ft.c).tty.xpixel).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_control_mode(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            if (*ft.c).flags.intersects(client_flag::CONTROL) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_discarded(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return format!("{}", (*ft.c).discarded).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_flags(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return server_client_get_flags((*ft.c).flags, (*ft.c).pause_age).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_height(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() && (*ft.c).tty.flags.intersects(tty_flags::TTY_STARTED) {
            return format!("{}", (*ft.c).tty.sy).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_key_table(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return (*ft.c).keytable.name_lossy().into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_last_session(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            let ls = client_get_last_session(ft.c);
            if !ls.is_null() && session_alive(ls) {
                return format!("{}", (*ls).name).into();
            }
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_name(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return format!("{}", _s((*ft.c).name)).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_pid(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return format!("{}", (*ft.c).pid as c_long).into();
        }
        format_table_type::None
    }
}

/// Callback for `client_prefix`.
pub unsafe fn format_cb_client_prefix(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            if server_client_is_default_key_table(ft.c, &(*ft.c).keytable) {
                return "0".into();
            }
            return "1".into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_readonly(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            if (*ft.c).flags.intersects(client_flag::READONLY) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_session(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            let s = client_get_session(ft.c);
            if !s.is_null() {
                return format!("{}", (*s).name).into();
            }
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_termfeatures(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return tty_get_features((*ft.c).term_features).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_termname(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return (*ft.c).term_name.clone().unwrap_or_default().into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_termtype(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return (*ft.c).term_type.clone().unwrap_or_default().into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_tty(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return (*ft.c).ttyname.clone().unwrap_or_default().into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_uid(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            let uid = proc_get_peer_uid((*ft.c).peer);
            if uid != -1_i32 as uid_t {
                return format!("{}", uid as c_long).into();
            }
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_user(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            let uid = proc_get_peer_uid((*ft.c).peer);
            if uid != -1_i32 as uid_t
                && let Some(pw) = NonNull::new(libc::getpwuid(uid))
            {
                return format!("{}", _s((*pw.as_ptr()).pw_name)).into();
            }
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_utf8(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            if (*ft.c).flags.intersects(client_flag::UTF8) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_width(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return format!("{}", (*ft.c).tty.sx).into();
        }
        format_table_type::None
    }
}

pub unsafe fn format_cb_client_written(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return format!("{}", (*ft.c).written).into();
        }
        format_table_type::None
    }
}

/// Callback for `config_files`.
pub unsafe fn format_cb_config_files(_ft: &format_tree) -> format_table_type {
    let files = CFG_FILES.lock().unwrap();
    let names: Vec<String> = files.iter().map(|file| file.to_string_lossy().into_owned()).collect();
    names.join(",").into()
}

/// Callback for `cursor_flag`.
pub unsafe fn format_cb_cursor_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_CURSOR) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `cursor_x`.
pub unsafe fn format_cb_cursor_x(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.cx).into();
        }
        format_table_type::None
    }
}

/// Callback for `cursor_y`.
pub unsafe fn format_cb_cursor_y(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.cy).into();
        }
        format_table_type::None
    }
}

/// Callback for `history_limit`.
pub unsafe fn format_cb_history_limit(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.grid.hlimit).into();
        }
        format_table_type::None
    }
}

/// Callback for `history_size`.
pub unsafe fn format_cb_history_size(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.grid.hsize).into();
        }
        format_table_type::None
    }
}

/// Callback for `insert_flag`.
pub unsafe fn format_cb_insert_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_INSERT) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `keypad_cursor_flag`.
pub unsafe fn format_cb_keypad_cursor_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_KCURSOR) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `keypad_flag`.
pub unsafe fn format_cb_keypad_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_KKEYPAD) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `mouse_all_flag`.
pub unsafe fn format_cb_mouse_all_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_MOUSE_ALL) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `mouse_any_flag`.
pub unsafe fn format_cb_mouse_any_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(ALL_MOUSE_MODES) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `mouse_button_flag`.
pub unsafe fn format_cb_mouse_button_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut()))
                .base
                .mode
                .intersects(mode_flag::MODE_MOUSE_BUTTON)
            {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `mouse_pane`.
pub unsafe fn format_cb_mouse_pane(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.m.valid {
            if let Some(wp) = cmd_mouse_pane(&ft.m, null_mut(), null_mut()) {
                return format!("%{}", (*wp.as_ptr()).id).into();
            }
            return format_table_type::None;
        }
        format_table_type::None
    }
}

/// Callback for `mouse_sgr_flag`.
pub unsafe fn format_cb_mouse_sgr_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_MOUSE_SGR) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `mouse_standard_flag`.
pub unsafe fn format_cb_mouse_standard_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut()))
                .base
                .mode
                .intersects(mode_flag::MODE_MOUSE_STANDARD)
            {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `mouse_utf8_flag`.
pub unsafe fn format_cb_mouse_utf8_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_MOUSE_UTF8) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `mouse_x`.
pub unsafe fn format_cb_mouse_x(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.m.valid {
            return format_table_type::None;
        }
        let wp = cmd_mouse_pane(&ft.m, null_mut(), null_mut());
        let mut x: u32 = 0;
        let mut y: u32 = 0;
        if let Some(wp) = wp
            && cmd_mouse_at(wp.as_ptr(), &ft.m, &mut x, &mut y, 0) == 0
        {
            return format!("{x}").into();
        }
        if !ft.c.is_null() && (*ft.c).tty.flags.intersects(tty_flags::TTY_STARTED) {
            if ft.m.statusat == 0 && ft.m.y < ft.m.statuslines {
                return format!("{}", ft.m.x).into();
            }
            if ft.m.statusat > 0 && ft.m.y >= ft.m.statusat as u32 {
                return format!("{}", ft.m.x).into();
            }
        }
        format_table_type::None
    }
}

/// Callback for `mouse_y`.
pub unsafe fn format_cb_mouse_y(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.m.valid {
            return format_table_type::None;
        }
        let wp = cmd_mouse_pane(&ft.m, null_mut(), null_mut());
        let mut x: u32 = 0;
        let mut y: u32 = 0;
        if let Some(wp) = wp
            && cmd_mouse_at(wp.as_ptr(), &ft.m, &mut x, &mut y, 0) == 0
        {
            return format!("{y}").into();
        }
        if !ft.c.is_null() && (*ft.c).tty.flags.intersects(tty_flags::TTY_STARTED) {
            if ft.m.statusat == 0 && ft.m.y < ft.m.statuslines {
                return format!("{}", ft.m.y).into();
            }
            if ft.m.statusat > 0 && ft.m.y >= ft.m.statusat as u32 {
                return format!("{}", ft.m.y - ft.m.statusat as u32).into();
            }
        }
        format_table_type::None
    }
}

/// Callback for `next_session_id`.
pub unsafe fn format_cb_next_session_id(_ft: &format_tree) -> format_table_type {
    let value = NEXT_SESSION_ID.load(atomic::Ordering::Relaxed);
    format!("${value}").into()
}

/// Callback for `origin_flag`.
pub unsafe fn format_cb_origin_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_ORIGIN) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_active`.
pub unsafe fn format_cb_pane_active(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if !wp.is_null() {
            if wp == window_active_pane(window_pane_window(wp)) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_at_left`.
pub unsafe fn format_cb_pane_at_left(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).xoff == 0 {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_at_right`.
pub unsafe fn format_cb_pane_at_right(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if !wp.is_null() {
            if (*wp).xoff + (*wp).sx == (*window_pane_window(wp)).sx {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_bottom`.
pub unsafe fn format_cb_pane_bottom(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).yoff + (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).sy - 1).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_dead`.
pub unsafe fn format_cb_pane_dead(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).fd == -1 {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_dead_signal`.
pub unsafe fn format_cb_pane_dead_signal(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if !wp.is_null() {
            if (*wp).flags.intersects(window_pane_flags::PANE_STATUSREADY)
                && WIFSIGNALED((*wp).status)
            {
                return format!("{}", WTERMSIG((*wp).status)).into();
            }
            return format_table_type::None;
        }
        format_table_type::None
    }
}

/// Callback for `pane_dead_status`.
pub unsafe fn format_cb_pane_dead_status(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if !wp.is_null() {
            if (*wp).flags.intersects(window_pane_flags::PANE_STATUSREADY)
                && WIFEXITED((*wp).status)
            {
                return format!("{}", WEXITSTATUS((*wp).status)).into();
            }
            return format_table_type::None;
        }
        format_table_type::None
    }
}

/// Callback for `pane_dead_time`.
pub unsafe fn format_cb_pane_dead_time(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if !wp.is_null() && (*wp).flags.intersects(window_pane_flags::PANE_STATUSDRAWN) {
            return format_table_type::Time((*wp).dead_time);
        }
        format_table_type::None
    }
}

/// Callback for `pane_format`.
pub unsafe fn format_cb_pane_format(ft: &format_tree) -> format_table_type {
    if ft.type_ == format_type::FORMAT_TYPE_PANE {
        return "1".into();
    }
    "0".into()
}

/// Callback for `pane_height`.
pub unsafe fn format_cb_pane_height(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).sy).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_id`.
pub unsafe fn format_cb_pane_id(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("%{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).id).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_index`.
pub unsafe fn format_cb_pane_index(ft: &format_tree) -> format_table_type {
    unsafe {
        let mut idx: u32 = 0;
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if !wp.is_null() && window_pane_index(wp, &mut idx) == 0 {
            return format!("{idx}").into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_input_off`.
pub unsafe fn format_cb_pane_input_off(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut()))
                .flags
                .intersects(window_pane_flags::PANE_INPUTOFF)
            {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_unseen_changes`.
pub unsafe fn format_cb_pane_unseen_changes(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut()))
                .flags
                .intersects(window_pane_flags::PANE_UNSEENCHANGES)
            {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_key_mode`.
pub unsafe fn format_cb_pane_key_mode(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() && !(*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).screen.is_null() {
            match (*(*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).screen).mode & EXTENDED_KEY_MODES {
                mode_flag::MODE_KEYS_EXTENDED => return "Ext 1".into(),
                mode_flag::MODE_KEYS_EXTENDED_2 => {
                    return "Ext 2".into();
                }
                _ => return "VT10x".into(),
            }
        }
        format_table_type::None
    }
}

/// Callback for `pane_last`.
pub unsafe fn format_cb_pane_last(ft: &format_tree) -> format_table_type {
    unsafe {
        let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        if !wp.is_null() {
            if wp == (*window_pane_window(wp)).last_panes.first().copied().unwrap_or(null_mut()) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_left`.
pub unsafe fn format_cb_pane_left(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).xoff).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_marked`.
pub unsafe fn format_cb_pane_marked(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if server_check_marked() && (*(&raw const MARKED_PANE)).wp == ft.wp && ft.wp.is_some() {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_marked_set`.
pub unsafe fn format_cb_pane_marked_set(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if server_check_marked() {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_mode`.
pub unsafe fn format_cb_pane_mode(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            let wme = (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).modes.first().copied().unwrap_or(null_mut());
            if !wme.is_null() {
                return (*(*wme).mode).name.into();
            }
            return format_table_type::None;
        }
        format_table_type::None
    }
}

/// Callback for `pane_path`.
pub unsafe fn format_cb_pane_path(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
            return match (*wp).base.path.as_ref() {
                Some(p) => format!("{}", p.to_string_lossy()).into(),
                None => "".into(),
            };
        }
        format_table_type::None
    }
}

/// Callback for `pane_pid`.
pub unsafe fn format_cb_pane_pid(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).pid as i64).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_pipe`.
pub unsafe fn format_cb_pane_pipe(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).pipe_fd != -1 {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_right`.
pub unsafe fn format_cb_pane_right(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).xoff + (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).sx - 1).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_search_string`.
pub unsafe fn format_cb_pane_search_string(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            let wp = ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
            return (*wp).searchstr.clone().unwrap_or_default().into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_synchronized`.
pub unsafe fn format_cb_pane_synchronized(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if options_get_number___::<i64>(&*(*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).options, "synchronize-panes") != 0 {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_title`.
pub unsafe fn format_cb_pane_title(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", _s((*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.title.as_ptr().cast::<u8>())).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_top`.
pub unsafe fn format_cb_pane_top(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).yoff).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_tty`.
pub unsafe fn format_cb_pane_tty(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", _s((*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).tty.as_ptr())).into();
        }
        format_table_type::None
    }
}

/// Callback for `pane_width`.
pub unsafe fn format_cb_pane_width(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).sx).into();
        }
        format_table_type::None
    }
}

/// Callback for `scroll_region_lower`.
pub unsafe fn format_cb_scroll_region_lower(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.rlower).into();
        }
        format_table_type::None
    }
}

/// Callback for `scroll_region_upper`.
pub unsafe fn format_cb_scroll_region_upper(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            return format!("{}", (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.rupper).into();
        }
        format_table_type::None
    }
}

/// Callback for `server_sessions`.
pub unsafe fn format_cb_server_sessions(_ft: &format_tree) -> format_table_type {
    unsafe {
        let n: u32 = (*(&raw mut SESSIONS)).len() as u32;
        format!("{n}").into()
    }
}

/// Callback for `session_attached`.
pub unsafe fn format_cb_session_attached(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return format!("{}", (*s).attached).into();
        }
        format_table_type::None
    }
}

/// Callback for `session_format`.
pub unsafe fn format_cb_session_format(ft: &format_tree) -> format_table_type {
    if ft.type_ == format_type::FORMAT_TYPE_SESSION {
        return "1".into();
    }
    "0".into()
}

/// Callback for `session_group`.
pub unsafe fn format_cb_session_group(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null()
            && let Some(sg) = session_group_contains(s)
        {
            return sg.into();
        }
        format_table_type::None
    }
}

/// Callback for `session_group_attached`.
pub unsafe fn format_cb_session_group_attached(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null()
            && let Some(sg) = session_group_contains(s)
        {
            return format!("{}", session_group_attached_count(&sg)).into();
        }
        format_table_type::None
    }
}

/// Callback for `session_group_many_attached`.
pub unsafe fn format_cb_session_group_many_attached(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null()
            && let Some(sg) = session_group_contains(s)
        {
            if session_group_attached_count(&sg) > 1 {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `session_group_size`.
pub unsafe fn format_cb_session_group_size(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null()
            && let Some(sg) = session_group_contains(s)
        {
            return format!("{}", session_group_count(&sg)).into();
        }
        format_table_type::None
    }
}

/// Callback for `session_grouped`.
pub unsafe fn format_cb_session_grouped(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            if session_group_contains(s).is_some() {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `session_id`.
pub unsafe fn format_cb_session_id(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return format!("${}", (*s).id).into();
        }
        format_table_type::None
    }
}

/// Callback for `session_many_attached`.
pub unsafe fn format_cb_session_many_attached(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            if (*s).attached > 1 {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `session_marked`.
pub unsafe fn format_cb_session_marked(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            if server_check_marked() && MARKED_PANE.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut()) == s {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `session_name`.
pub unsafe fn format_cb_session_name(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return format!("{}", (*s).name).into();
        }
        format_table_type::None
    }
}

/// Callback for `session_path`.
pub unsafe fn format_cb_session_path(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return (*s).cwd.as_deref().map(|p| p.display().to_string()).unwrap_or_default().into();
        }
        format_table_type::None
    }
}

/// Callback for `session_windows`.
pub unsafe fn format_cb_session_windows(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return format!("{}", winlink_count((*s).sid())).into();
        }
        format_table_type::None
    }
}

/// Callback for `socket_path`.
pub unsafe fn format_cb_socket_path(_ft: &format_tree) -> format_table_type {
    socket_path().to_string_lossy().into_owned().into()
}

/// Callback for version.
pub unsafe fn format_cb_version(_ft: &format_tree) -> format_table_type {
    getversion().into()
}

/// Callback for `active_window_index`.
pub unsafe fn format_cb_active_window_index(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return format!("{}", (*s).curw().unwrap().idx).into();
        }
        format_table_type::None
    }
}

/// Callback for `last_window_index`.
pub unsafe fn format_cb_last_window_index(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            let wl = (*s).windows().last().copied().unwrap();
            return format!("{}", wl.idx).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_active`.
pub unsafe fn format_cb_window_active(ft: &format_tree) -> format_table_type {
    unsafe {
        if let Some(wl) = ft.wl {
            let s = session_from_id(wl.session).unwrap_or(null_mut());
            if !s.is_null() && ft.wl.map(|wl| wl.id) == (*s).curw_id() {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `window_activity_flag`.
pub fn format_cb_window_activity_flag(ft: &format_tree) -> format_table_type {
    if let Some(wl) = ft.wl {
        if wl
            .flags
            .intersects(winlink_flags::WINLINK_ACTIVITY)
        {
            return "1".into();
        }
        return "0".into();
    }
    format_table_type::None
}

/// Callback for `window_bell_flag`.
pub fn format_cb_window_bell_flag(ft: &format_tree) -> format_table_type {
    if let Some(wl) = ft.wl {
        if wl.flags.intersects(winlink_flags::WINLINK_BELL) {
            return "1".into();
        }
        return "0".into();
    }
    format_table_type::None
}

/// Callback for `window_bigger`.
pub unsafe fn format_cb_window_bigger(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            let mut ox: u32 = 0;
            let mut oy: u32 = 0;
            let mut sx: u32 = 0;
            let mut sy: u32 = 0;
            if tty_window_offset(&raw mut (*ft.c).tty, &mut ox, &mut oy, &mut sx, &mut sy) != 0 {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `window_cell_height`.
pub unsafe fn format_cb_window_cell_height(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            return format!("{}", (*w).ypixel).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_cell_width`.
pub unsafe fn format_cb_window_cell_width(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            return format!("{}", (*w).xpixel).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_end_flag`.
pub unsafe fn format_cb_window_end_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if let Some(wl) = ft.wl {
            let s = session_from_id(wl.session).unwrap_or(null_mut());
            if !s.is_null() && ft.wl.map(|wl| wl.id) == (*s).windows().last().map(|wl| wl.id) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `window_flags`.
pub unsafe fn format_cb_window_flags(ft: &format_tree) -> format_table_type {
    unsafe {
        if let Some(wl) = ft.wl {
            return window_printable_flags(wl, 1).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_format`.
pub unsafe fn format_cb_window_format(ft: &format_tree) -> format_table_type {
    if ft.type_ == format_type::FORMAT_TYPE_WINDOW {
        return "1".into();
    }
    "0".into()
}

/// Callback for `window_height`.
pub unsafe fn format_cb_window_height(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            return format!("{}", (*w).sy).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_id`.
pub unsafe fn format_cb_window_id(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            return format!("@{}", (*w).id).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_index`.
pub fn format_cb_window_index(ft: &format_tree) -> format_table_type {
    if let Some(wl) = ft.wl {
        return format!("{}", wl.idx).into();
    }
    format_table_type::None
}

/// Callback for `window_last_flag`.
pub unsafe fn format_cb_window_last_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if let Some(wl) = ft.wl {
            let s = session_from_id(wl.session).unwrap_or(null_mut());
            if !s.is_null() && ft.wl.map(|wl| wl.id) == (*s).lastw().first().map(|wl| wl.id) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `window_linked`.
pub fn format_cb_window_linked(ft: &format_tree) -> format_table_type {
    if let Some(wl) = ft.wl {
        // Linked into more than one place, groups included (3.8 no
        // longer uses the reference count here).
        let wid = wl.window;
        if window_linked_anywhere(wid) {
            return "1".into();
        }
        return "0".into();
    }
    format_table_type::None
}

/// Callback for `window_linked_sessions`.
pub fn format_cb_window_linked_sessions(ft: &format_tree) -> format_table_type {
    if let Some(wl) = ft.wl {
        let wid = wl.window;
        return format!("{}", window_linked_sessions(wid)).into();
    }
    format_table_type::None
}

/// Callback for `window_marked_flag`.
pub unsafe fn format_cb_window_marked_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if let Some(wl) = ft.wl {
            if server_check_marked() && MARKED_PANE.wl == Some(wl.id) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `window_name`.
pub unsafe fn format_cb_window_name(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            return (*w).name.clone().unwrap_or_default().into();
        }
        format_table_type::None
    }
}

/// Callback for `window_offset_x`.
pub unsafe fn format_cb_window_offset_x(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            let mut ox: u32 = 0;
            let mut oy: u32 = 0;
            let mut sx: u32 = 0;
            let mut sy: u32 = 0;
            if tty_window_offset(&raw mut (*ft.c).tty, &mut ox, &mut oy, &mut sx, &mut sy) != 0 {
                return format!("{ox}").into();
            }
        }
        format_table_type::None
    }
}

/// Callback for `window_offset_y`.
pub unsafe fn format_cb_window_offset_y(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            let mut ox: u32 = 0;
            let mut oy: u32 = 0;
            let mut sx: u32 = 0;
            let mut sy: u32 = 0;
            if tty_window_offset(&raw mut (*ft.c).tty, &mut ox, &mut oy, &mut sx, &mut sy) != 0 {
                return format!("{oy}").into();
            }
        }
        format_table_type::None
    }
}

/// Callback for `window_panes`.
pub unsafe fn format_cb_window_panes(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            return format!("{}", window_count_panes(&*w)).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_raw_flags`.
pub unsafe fn format_cb_window_raw_flags(ft: &format_tree) -> format_table_type {
    unsafe {
        if let Some(wl) = ft.wl {
            return window_printable_flags(wl, 0).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_silence_flag`.
pub fn format_cb_window_silence_flag(ft: &format_tree) -> format_table_type {
    if let Some(wl) = ft.wl {
        if wl.flags.intersects(winlink_flags::WINLINK_SILENCE) {
            return "1".into();
        }
        return "0".into();
    }
    format_table_type::None
}

/// Callback for `window_start_flag`.
pub unsafe fn format_cb_window_start_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if let Some(wl) = ft.wl {
            let s = session_from_id(wl.session).unwrap_or(null_mut());
            if !s.is_null() && ft.wl.map(|wl| wl.id) == (*s).windows().first().map(|wl| wl.id) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `window_width`.
pub unsafe fn format_cb_window_width(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            return format!("{}", (*w).sx).into();
        }
        format_table_type::None
    }
}

/// Callback for `window_zoomed_flag`.
pub unsafe fn format_cb_window_zoomed_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            if (*w).flags.intersects(window_flag::ZOOMED) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `wrap_flag`.
pub unsafe fn format_cb_wrap_flag(ft: &format_tree) -> format_table_type {
    unsafe {
        if ft.wp.is_some() {
            if (*ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())).base.mode.intersects(mode_flag::MODE_WRAP) {
                return "1".into();
            }
            return "0".into();
        }
        format_table_type::None
    }
}

/// Callback for `buffer_created`.
pub fn format_cb_buffer_created(ft: &format_tree) -> format_table_type {
    ft.pb.as_ref().map_or(format_table_type::None, |pb| {
        format_table_type::Time(timeval {
            tv_sec: pb.created(),
            tv_usec: 0,
        })
    })
}

/// Callback for `client_activity`.
pub unsafe fn format_cb_client_activity(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return format_table_type::Time((*ft.c).activity_time);
        }
        format_table_type::None
    }
}

/// Callback for `client_created`.
pub unsafe fn format_cb_client_created(ft: &format_tree) -> format_table_type {
    unsafe {
        if !ft.c.is_null() {
            return format_table_type::Time((*ft.c).creation_time);
        }
        format_table_type::None
    }
}

/// Callback for `session_activity`.
pub unsafe fn format_cb_session_activity(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return format_table_type::Time((*s).activity_time);
        }
        format_table_type::None
    }
}

/// Callback for `session_created`.
pub unsafe fn format_cb_session_created(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return format_table_type::Time((*s).creation_time);
        }
        format_table_type::None
    }
}

/// Callback for `session_last_attached`.
pub unsafe fn format_cb_session_last_attached(ft: &format_tree) -> format_table_type {
    unsafe {
        let s = ft.s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        if !s.is_null() {
            return format_table_type::Time((*s).last_attached_time);
        }
        format_table_type::None
    }
}

/// Callback for `start_time`.
pub unsafe fn format_cb_start_time(_ft: &format_tree) -> format_table_type {
    format_table_type::Time(START_TIME.get().copied().unwrap_or(timeval { tv_sec: 0, tv_usec: 0 }))
}

/// Callback for `window_activity`.
pub unsafe fn format_cb_window_activity(ft: &format_tree) -> format_table_type {
    unsafe {
        let w = ft.w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        if !w.is_null() {
            return format_table_type::Time((*w).activity_time);
        }
        format_table_type::None
    }
}

/// Callback for `buffer_mode_format`.
pub unsafe fn format_cb_buffer_mode_format(_ft: &format_tree) -> format_table_type {
    WINDOW_BUFFER_MODE.default_format.unwrap().into()
}

/// Callback for `client_mode_format`.
pub unsafe fn format_cb_client_mode_format(_ft: &format_tree) -> format_table_type {
    WINDOW_CLIENT_MODE.default_format.unwrap().into()
}

/// Callback for `tree_mode_format`.
pub unsafe fn format_cb_tree_mode_format(_ft: &format_tree) -> format_table_type {
    WINDOW_TREE_MODE.default_format.unwrap().into()
}

/// Callback for uid.
pub unsafe fn format_cb_uid(_ft: &format_tree) -> format_table_type {
    unsafe { format!("{}", getuid() as i64).into() }
}

/// Callback for user.
pub unsafe fn format_cb_user(_ft: &format_tree) -> format_table_type {
    unsafe {
        if let Some(pw) = NonNull::new(getpwuid(getuid())) {
            cstr_to_str((*pw.as_ptr()).pw_name.cast())
                .to_string()
                .into()
        } else {
            format_table_type::None
        }
    }
}

/// Format table type.
#[derive(Default)]
pub enum format_table_type {
    #[default]
    None,
    String(Cow<'static, str>),
    Time(timeval),
}

impl From<Cow<'static, str>> for format_table_type {
    fn from(value: Cow<'static, str>) -> Self {
        Self::String(value)
    }
}

impl From<String> for format_table_type {
    fn from(value: String) -> Self {
        format_table_type::String(Cow::Owned(value))
    }
}

impl From<&'static str> for format_table_type {
    fn from(value: &'static str) -> Self {
        format_table_type::String(Cow::Borrowed(value))
    }
}

/// Format table entry.
pub struct format_table_entry {
    pub key: &'static str,
    pub cb: format_cb,
}

impl format_table_entry {
    pub const fn new(key: &'static str, cb: format_cb) -> Self {
        Self { key, cb }
    }
}

// Format table. Default format variables (that are almost always in the tree
// and where the value is expanded by a callback in this file) are listed
// here. Only variables which are added by the caller go into the tree.
pub(crate) static FORMAT_TABLE: &[format_table_entry] = &[
    format_table_entry::new("active_window_index", format_cb_active_window_index),
    format_table_entry::new("alternate_on", format_cb_alternate_on),
    format_table_entry::new("alternate_saved_x", format_cb_alternate_saved_x),
    format_table_entry::new("alternate_saved_y", format_cb_alternate_saved_y),
    format_table_entry::new("buffer_created", format_cb_buffer_created),
    format_table_entry::new("buffer_mode_format", format_cb_buffer_mode_format),
    format_table_entry::new("buffer_name", format_cb_buffer_name),
    format_table_entry::new("buffer_sample", format_cb_buffer_sample),
    format_table_entry::new("buffer_size", format_cb_buffer_size),
    format_table_entry::new("client_activity", format_cb_client_activity),
    format_table_entry::new("client_cell_height", format_cb_client_cell_height),
    format_table_entry::new("client_cell_width", format_cb_client_cell_width),
    format_table_entry::new("client_control_mode", format_cb_client_control_mode),
    format_table_entry::new("client_created", format_cb_client_created),
    format_table_entry::new("client_discarded", format_cb_client_discarded),
    format_table_entry::new("client_flags", format_cb_client_flags),
    format_table_entry::new("client_height", format_cb_client_height),
    format_table_entry::new("client_key_table", format_cb_client_key_table),
    format_table_entry::new("client_last_session", format_cb_client_last_session),
    format_table_entry::new("client_mode_format", format_cb_client_mode_format),
    format_table_entry::new("client_name", format_cb_client_name),
    format_table_entry::new("client_pid", format_cb_client_pid),
    format_table_entry::new("client_prefix", format_cb_client_prefix),
    format_table_entry::new("client_readonly", format_cb_client_readonly),
    format_table_entry::new("client_session", format_cb_client_session),
    format_table_entry::new("client_termfeatures", format_cb_client_termfeatures),
    format_table_entry::new("client_termname", format_cb_client_termname),
    format_table_entry::new("client_termtype", format_cb_client_termtype),
    format_table_entry::new("client_tty", format_cb_client_tty),
    format_table_entry::new("client_uid", format_cb_client_uid),
    format_table_entry::new("client_user", format_cb_client_user),
    format_table_entry::new("client_utf8", format_cb_client_utf8),
    format_table_entry::new("client_width", format_cb_client_width),
    format_table_entry::new("client_written", format_cb_client_written),
    format_table_entry::new("config_files", format_cb_config_files),
    format_table_entry::new("cursor_character", format_cb_cursor_character),
    format_table_entry::new("cursor_flag", format_cb_cursor_flag),
    format_table_entry::new("cursor_x", format_cb_cursor_x),
    format_table_entry::new("cursor_y", format_cb_cursor_y),
    format_table_entry::new("history_all_bytes", format_cb_history_all_bytes),
    format_table_entry::new("history_bytes", format_cb_history_bytes),
    format_table_entry::new("history_limit", format_cb_history_limit),
    format_table_entry::new("history_size", format_cb_history_size),
    format_table_entry::new("host", format_cb_host),
    format_table_entry::new("host_short", format_cb_host_short),
    format_table_entry::new("insert_flag", format_cb_insert_flag),
    format_table_entry::new("keypad_cursor_flag", format_cb_keypad_cursor_flag),
    format_table_entry::new("keypad_flag", format_cb_keypad_flag),
    format_table_entry::new("last_window_index", format_cb_last_window_index),
    format_table_entry::new("mouse_all_flag", format_cb_mouse_all_flag),
    format_table_entry::new("mouse_any_flag", format_cb_mouse_any_flag),
    format_table_entry::new("mouse_button_flag", format_cb_mouse_button_flag),
    format_table_entry::new("mouse_hyperlink", format_cb_mouse_hyperlink),
    format_table_entry::new("mouse_line", format_cb_mouse_line),
    format_table_entry::new("mouse_pane", format_cb_mouse_pane),
    format_table_entry::new("mouse_sgr_flag", format_cb_mouse_sgr_flag),
    format_table_entry::new("mouse_standard_flag", format_cb_mouse_standard_flag),
    format_table_entry::new("mouse_status_line", format_cb_mouse_status_line),
    format_table_entry::new("mouse_status_range", format_cb_mouse_status_range),
    format_table_entry::new("mouse_utf8_flag", format_cb_mouse_utf8_flag),
    format_table_entry::new("mouse_word", format_cb_mouse_word),
    format_table_entry::new("mouse_x", format_cb_mouse_x),
    format_table_entry::new("mouse_y", format_cb_mouse_y),
    format_table_entry::new("next_session_id", format_cb_next_session_id),
    format_table_entry::new("origin_flag", format_cb_origin_flag),
    format_table_entry::new("pane_active", format_cb_pane_active),
    format_table_entry::new("pane_at_bottom", format_cb_pane_at_bottom),
    format_table_entry::new("pane_at_left", format_cb_pane_at_left),
    format_table_entry::new("pane_at_right", format_cb_pane_at_right),
    format_table_entry::new("pane_at_top", format_cb_pane_at_top),
    format_table_entry::new("pane_bg", format_cb_pane_bg),
    format_table_entry::new("pane_bottom", format_cb_pane_bottom),
    format_table_entry::new("pane_current_command", format_cb_current_command),
    format_table_entry::new("pane_current_path", format_cb_current_path),
    format_table_entry::new("pane_dead", format_cb_pane_dead),
    format_table_entry::new("pane_dead_signal", format_cb_pane_dead_signal),
    format_table_entry::new("pane_dead_status", format_cb_pane_dead_status),
    format_table_entry::new("pane_dead_time", format_cb_pane_dead_time),
    format_table_entry::new("pane_fg", format_cb_pane_fg),
    format_table_entry::new("pane_format", format_cb_pane_format),
    format_table_entry::new("pane_height", format_cb_pane_height),
    format_table_entry::new("pane_id", format_cb_pane_id),
    format_table_entry::new("pane_in_mode", format_cb_pane_in_mode),
    format_table_entry::new("pane_index", format_cb_pane_index),
    format_table_entry::new("pane_input_off", format_cb_pane_input_off),
    format_table_entry::new("pane_key_mode", format_cb_pane_key_mode),
    format_table_entry::new("pane_last", format_cb_pane_last),
    format_table_entry::new("pane_left", format_cb_pane_left),
    format_table_entry::new("pane_marked", format_cb_pane_marked),
    format_table_entry::new("pane_marked_set", format_cb_pane_marked_set),
    format_table_entry::new("pane_mode", format_cb_pane_mode),
    format_table_entry::new("pane_path", format_cb_pane_path),
    format_table_entry::new("pane_pid", format_cb_pane_pid),
    format_table_entry::new("pane_pipe", format_cb_pane_pipe),
    format_table_entry::new("pane_right", format_cb_pane_right),
    format_table_entry::new("pane_search_string", format_cb_pane_search_string),
    format_table_entry::new("pane_start_command", format_cb_start_command),
    format_table_entry::new("pane_start_path", format_cb_start_path),
    format_table_entry::new("pane_synchronized", format_cb_pane_synchronized),
    format_table_entry::new("pane_tabs", format_cb_pane_tabs),
    format_table_entry::new("pane_title", format_cb_pane_title),
    format_table_entry::new("pane_top", format_cb_pane_top),
    format_table_entry::new("pane_tty", format_cb_pane_tty),
    format_table_entry::new("pane_unseen_changes", format_cb_pane_unseen_changes),
    format_table_entry::new("pane_width", format_cb_pane_width),
    format_table_entry::new("pid", format_cb_pid),
    format_table_entry::new("scroll_region_lower", format_cb_scroll_region_lower),
    format_table_entry::new("scroll_region_upper", format_cb_scroll_region_upper),
    format_table_entry::new("server_sessions", format_cb_server_sessions),
    format_table_entry::new("session_activity", format_cb_session_activity),
    format_table_entry::new("session_alerts", format_cb_session_alerts),
    format_table_entry::new("session_attached", format_cb_session_attached),
    format_table_entry::new("session_attached_list", format_cb_session_attached_list),
    format_table_entry::new("session_created", format_cb_session_created),
    format_table_entry::new("session_format", format_cb_session_format),
    format_table_entry::new("session_group", format_cb_session_group),
    format_table_entry::new("session_group_attached", format_cb_session_group_attached),
    format_table_entry::new(
        "session_group_attached_list",
        format_cb_session_group_attached_list,
    ),
    format_table_entry::new("session_group_list", format_cb_session_group_list),
    format_table_entry::new(
        "session_group_many_attached",
        format_cb_session_group_many_attached,
    ),
    format_table_entry::new("session_group_size", format_cb_session_group_size),
    format_table_entry::new("session_grouped", format_cb_session_grouped),
    format_table_entry::new("session_id", format_cb_session_id),
    format_table_entry::new("session_last_attached", format_cb_session_last_attached),
    format_table_entry::new("session_many_attached", format_cb_session_many_attached),
    format_table_entry::new("session_marked", format_cb_session_marked),
    format_table_entry::new("session_name", format_cb_session_name),
    format_table_entry::new("session_path", format_cb_session_path),
    format_table_entry::new("session_stack", format_cb_session_stack),
    format_table_entry::new("session_windows", format_cb_session_windows),
    format_table_entry::new("socket_path", format_cb_socket_path),
    format_table_entry::new("start_time", format_cb_start_time),
    format_table_entry::new("tree_mode_format", format_cb_tree_mode_format),
    format_table_entry::new("uid", format_cb_uid),
    format_table_entry::new("user", format_cb_user),
    format_table_entry::new("version", format_cb_version),
    format_table_entry::new("window_active", format_cb_window_active),
    format_table_entry::new("window_active_clients", format_cb_window_active_clients),
    format_table_entry::new(
        "window_active_clients_list",
        format_cb_window_active_clients_list,
    ),
    format_table_entry::new("window_active_sessions", format_cb_window_active_sessions),
    format_table_entry::new(
        "window_active_sessions_list",
        format_cb_window_active_sessions_list,
    ),
    format_table_entry::new("window_activity", format_cb_window_activity),
    format_table_entry::new("window_activity_flag", format_cb_window_activity_flag),
    format_table_entry::new("window_bell_flag", format_cb_window_bell_flag),
    format_table_entry::new("window_bigger", format_cb_window_bigger),
    format_table_entry::new("window_cell_height", format_cb_window_cell_height),
    format_table_entry::new("window_cell_width", format_cb_window_cell_width),
    format_table_entry::new("window_end_flag", format_cb_window_end_flag),
    format_table_entry::new("window_flags", format_cb_window_flags),
    format_table_entry::new("window_format", format_cb_window_format),
    format_table_entry::new("window_height", format_cb_window_height),
    format_table_entry::new("window_id", format_cb_window_id),
    format_table_entry::new("window_index", format_cb_window_index),
    format_table_entry::new("window_last_flag", format_cb_window_last_flag),
    format_table_entry::new("window_layout", format_cb_window_layout),
    format_table_entry::new("window_linked", format_cb_window_linked),
    format_table_entry::new("window_linked_sessions", format_cb_window_linked_sessions),
    format_table_entry::new(
        "window_linked_sessions_list",
        format_cb_window_linked_sessions_list,
    ),
    format_table_entry::new("window_marked_flag", format_cb_window_marked_flag),
    format_table_entry::new("window_name", format_cb_window_name),
    format_table_entry::new("window_offset_x", format_cb_window_offset_x),
    format_table_entry::new("window_offset_y", format_cb_window_offset_y),
    format_table_entry::new("window_panes", format_cb_window_panes),
    format_table_entry::new("window_raw_flags", format_cb_window_raw_flags),
    format_table_entry::new("window_silence_flag", format_cb_window_silence_flag),
    format_table_entry::new("window_stack_index", format_cb_window_stack_index),
    format_table_entry::new("window_start_flag", format_cb_window_start_flag),
    format_table_entry::new("window_visible_layout", format_cb_window_visible_layout),
    format_table_entry::new("window_width", format_cb_window_width),
    format_table_entry::new("window_zoomed_flag", format_cb_window_zoomed_flag),
    format_table_entry::new("wrap_flag", format_cb_wrap_flag),
];

pub unsafe fn format_merge(ft: *mut format_tree, from: *mut format_tree) {
    unsafe {
        for (key, fe) in &(*from).tree {
            if let format_entry::Value(v) = fe {
                (*ft).tree.insert(key.clone(), format_entry::Value(v.clone()));
            }
        }
    }
}

pub unsafe fn format_get_pane(ft: &format_tree) -> *mut window_pane {
    unsafe { ft.wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut()) }
}

pub unsafe fn format_create_add_item(ft: *mut format_tree, item: *mut cmdq_item) {
    unsafe {
        let event = cmdq_get_event(item);
        let m = &(*event).m;

        cmdq_merge_formats(item, ft);
        memcpy__(&raw mut (*ft).m, m);
    }
}

pub unsafe fn format_create(
    c: *mut client,
    item: *mut cmdq_item,
    tag: i32,
    flags: format_flags,
) -> *mut format_tree {
    unsafe {
        let ft = xcalloc1::<format_tree>() as *mut format_tree;
        // xcalloc returns zeroed memory. HashMap is not valid when zeroed,
        // so we must use ptr::write to avoid dropping the invalid value.
        std::ptr::write(&raw mut (*ft).tree, BTreeMap::new());
        std::ptr::write(&raw mut (*ft).pb, None);

        if !c.is_null() {
            (*ft).client = c;
            (*c).references += 1;
        }
        (*ft).item = item;
        (*ft).tag = tag as u32;
        (*ft).flags = flags;

        if !item.is_null() {
            format_create_add_item(ft, item);
        }

        ft
    }
}

pub unsafe fn format_free(ft: *mut format_tree) {
    unsafe {
        // Drop the HashMap and the buffer before freeing the raw allocation.
        std::ptr::drop_in_place(&raw mut (*ft).tree);
        std::ptr::drop_in_place(&raw mut (*ft).pb);

        if !(*ft).client.is_null() {
            server_client_unref((*ft).client);
        }
        free(ft as *mut c_void);
    }
}

pub unsafe fn format_log_debug_cb(key: &str, value: &str, prefix: *mut u8) {
    unsafe {
        log_debug!("{}: {}={}", _s(prefix), key, value);
    }
}

pub unsafe fn format_log_debug(ft: *mut format_tree, prefix: *const u8) {
    unsafe {
        format_each(ft, format_log_debug_cb, prefix.cast_mut());
    }
}

pub unsafe fn format_each<T>(ft: *mut format_tree, cb: unsafe fn(&str, &str, *mut T), arg: *mut T) {
    unsafe {
        for fte in FORMAT_TABLE {
            let value = (fte.cb)(&*ft);
            match value {
                format_table_type::None => continue,
                format_table_type::Time(tv) => {
                    let s = format!("{}", tv.tv_sec);
                    cb(fte.key, &s, arg);
                }
                format_table_type::String(string) => {
                    cb(fte.key, &string, arg);
                }
            }
        }

        for (key, fe) in &(*ft).tree {
            let key = String::from_utf8_lossy(key);
            match fe {
                format_entry::Time(t) => cb(&key, &t.to_string(), arg),
                format_entry::Value(v) => cb(&key, &String::from_utf8_lossy(v), arg),
                format_entry::Callback(fe_cb, cached) => {
                    let v = cached.get_or_init(|| match fe_cb(&*ft) {
                        format_table_type::String(cow) => cow.into_owned().into_bytes(),
                        _ => Vec::new(),
                    });
                    cb(&key, &String::from_utf8_lossy(v), arg);
                }
            }
        }
    }
}

macro_rules! format_add {
   ($state:expr, $key:expr, $fmt:literal $(, $args:expr)* $(,)?) => {
        crate::format::format_add_($state, $key, format_args!($fmt $(, $args)*))
    };
}
pub(crate) use format_add;

/// Add a key-value pair.
pub unsafe fn format_add_(ft: *mut format_tree, key: &str, args: std::fmt::Arguments) {
    unsafe {
        (*ft).tree.insert(key.as_bytes().to_vec(), format_entry::Value(args.to_string().into_bytes()));
    }
}

/// Add a key and time.
pub unsafe fn format_add_tv(ft: *mut format_tree, key: *const u8, tv: *const timeval) {
    unsafe {
        let key = CStr::from_ptr(key.cast()).to_bytes().to_vec();
        (*ft).tree.insert(key, format_entry::Time((*tv).tv_sec));
    }
}

/// Add a key and function.
pub unsafe fn format_add_cb(ft: *mut format_tree, key: *const u8, cb: format_cb) {
    unsafe {
        let key = CStr::from_ptr(key.cast()).to_bytes().to_vec();
        (*ft).tree.insert(key, format_entry::Callback(cb, std::cell::OnceCell::new()));
    }
}

/// Make a prettier time.
pub unsafe fn format_pretty_time(t: time_t, seconds: i32) -> *mut u8 {
    unsafe {
        let mut now: time_t = libc::time(null_mut());
        if now < t {
            now = t;
        }
        let age = now - t;

        let mut now_tm = MaybeUninit::<tm>::uninit();
        let now_tm = now_tm.as_mut_ptr();
        let mut tm = MaybeUninit::<tm>::uninit();
        let tm = tm.as_mut_ptr();

        localtime_r(&raw const now, now_tm);
        localtime_r(&raw const t, tm);

        // Last 24 hours.
        const SIZEOF_S: usize = 9;
        let mut s = [0u8; 9];
        if age < 24 * 3600 {
            if seconds != 0 {
                strftime(s.as_mut_ptr(), SIZEOF_S, c!("%H:%M:%S"), tm);
            } else {
                strftime(s.as_mut_ptr(), SIZEOF_S, c!("%H:%M"), tm);
            }
            return xstrdup(s.as_ptr()).as_ptr();
        }

        // This month or last 28 days.
        if ((*tm).tm_year == (*now_tm).tm_year && (*tm).tm_mon == (*now_tm).tm_mon)
            || age < 28 * 24 * 3600
        {
            strftime(s.as_mut_ptr(), SIZEOF_S, c!("%a%d"), tm);
            return xstrdup(s.as_ptr()).as_ptr();
        }

        // Last 12 months.
        if ((*tm).tm_year == (*now_tm).tm_year && (*tm).tm_mon < (*now_tm).tm_mon)
            || ((*tm).tm_year == (*now_tm).tm_year - 1 && (*tm).tm_mon > (*now_tm).tm_mon)
        {
            strftime(s.as_mut_ptr(), SIZEOF_S, c!("%d%b"), tm);
            return xstrdup(s.as_ptr()).as_ptr();
        }

        // Older than that.
        strftime(s.as_mut_ptr(), SIZEOF_S, c!("%h%y"), tm);
        xstrdup(s.as_ptr()).as_ptr()
    }
}

/// Skip until end.
pub unsafe fn format_skip(mut s: *const u8, end: *const u8) -> *const u8 {
    unsafe {
        let mut brackets = 0;

        while *s != b'\0' {
            if *s == b'#' && *s.add(1) == b'{' {
                brackets += 1;
            }
            if *s == b'#' && *s.add(1) != b'\0' && !strchr(c!(",#{}:"), *s.add(1) as i32).is_null()
            {
                s = s.add(2);
                continue;
            }
            if *s == b'}' {
                brackets -= 1;
            }
            if !strchr(end, *s as i32).is_null() && brackets == 0 {
                break;
            }
            s = s.add(1);
        }
        if *s == b'\0' {
            return null_mut();
        }
        s
    }
}

/// Is this true?
pub unsafe fn format_true(s: *const u8) -> bool {
    unsafe { !s.is_null() && *s != b'\0' && (*s != b'0' || *s.add(1) != b'\0') }
}

/// Expand keys in a template, passing through strftime first.
pub unsafe fn format_expand_time(ft: *mut format_tree, fmt: *const u8) -> *mut u8 {
    unsafe { format_to_c(&format_engine_expand(ft, format_bytes(fmt), true)) }
}

/// Expand keys in a template.
pub unsafe fn format_expand(ft: *mut format_tree, fmt: *const u8) -> *mut u8 {
    unsafe { format_to_c(&format_engine_expand(ft, format_bytes(fmt), false)) }
}

/// Expand a single string.
pub unsafe fn format_single(
    item: *mut cmdq_item,
    fmt: &str,
    c: *mut client,
    s: *mut session,
    wl: Option<WinlinkId>,
    wp: *mut window_pane,
) -> *mut u8 {
    unsafe {
        let fmt = CString::new(fmt).unwrap(); // TODO shim to not have to rewrite
                                                       // format_expand now, remove later
        format_single_bytes(item, fmt.as_ptr().cast(), c, s, wl, wp)
    }
}

/// Expand a single string given as a C string, which may hold bytes that
/// are not UTF-8 (names from clients). They are expanded as they are.
///
/// # Safety
/// `fmt` must be a valid C string, the others null or valid.
pub unsafe fn format_single_bytes(
    item: *mut cmdq_item,
    fmt: *const u8,
    c: *mut client,
    s: *mut session,
    wl: Option<WinlinkId>,
    wp: *mut window_pane,
) -> *mut u8 {
    unsafe {
        let ft = format_create_defaults(item, c, s, wl, wp);
        let expanded: *mut u8 = format_expand(ft, fmt);
        format_free(ft);
        expanded
    }
}

/// Expand a single string using state.
pub unsafe fn format_single_from_state(
    item: *mut cmdq_item,
    fmt: &str,
    c: *mut client,
    fs: *mut cmd_find_state,
) -> *mut u8 {
    unsafe { format_single(item, fmt, c, (*fs).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut()), (*fs).wl, (*fs).wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())) }
}

/// Expand a single string using target.
pub unsafe fn format_single_from_target(item: *mut cmdq_item, fmt: *const u8) -> *mut u8 {
    unsafe {
        let tc = cmdq_get_target_client(item);
        let fs = cmdq_get_target(item);
        format_single_bytes(
            item,
            fmt,
            tc,
            (*fs).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut()),
            (*fs).wl,
            (*fs).wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut()),
        )
    }
}

/// Create and add defaults.
pub unsafe fn format_create_defaults(
    item: *mut cmdq_item,
    c: *mut client,
    s: *mut session,
    wl: Option<WinlinkId>,
    wp: *mut window_pane,
) -> *mut format_tree {
    unsafe {
        let ft = if !item.is_null() {
            format_create(
                cmdq_get_client(item),
                item,
                FORMAT_NONE,
                format_flags::empty(),
            )
        } else {
            format_create(null_mut(), item, FORMAT_NONE, format_flags::empty())
        };
        format_defaults(ft, c, NonNull::new(s), wl, NonNull::new(wp));
        ft
    }
}

/// Create and add defaults using state.
pub unsafe fn format_create_from_state(
    item: *mut cmdq_item,
    c: *mut client,
    fs: *mut cmd_find_state,
) -> *mut format_tree {
    unsafe { format_create_defaults(item, c, (*fs).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut()), (*fs).wl, (*fs).wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut())) }
}

/// Create and add defaults using target.
pub unsafe fn format_create_from_target(item: *mut cmdq_item) -> *mut format_tree {
    unsafe {
        let tc = cmdq_get_target_client(item);

        format_create_from_state(item, tc, cmdq_get_target(item))
    }
}

/// Set defaults for any of arguments that are not NULL.
pub unsafe fn format_defaults(
    ft: *mut format_tree,
    c: *mut client,
    s: Option<NonNull<session>>,
    wl: Option<WinlinkId>,
    wp: Option<NonNull<window_pane>>,
) {
    unsafe {
        let mut s = transmute_ptr(s);
        let mut wl = wl.and_then(winlink_get);
        let mut wp = transmute_ptr(wp);

        if !c.is_null() && !(*c).name.is_null() {
            log_debug!("{}: c={}", function_name!(), _s((*c).name));
        } else {
            log_debug!("{}: c=none", function_name!());
        }
        if !s.is_null() {
            log_debug!("{}: s=${}", function_name!(), (*s).id);
        } else {
            log_debug!("{}: s=none", function_name!());
        }
        if let Some(wl) = wl {
            log_debug!("{}: wl={}", function_name!(), wl.idx);
        } else {
            log_debug!("{}: wl=none", function_name!());
        }
        if !wp.is_null() {
            log_debug!("{}: wp=%%{}", function_name!(), (*wp).id);
        } else {
            log_debug!("{}: wp=none", function_name!());
        }

        if !c.is_null() && !s.is_null() && client_get_session(c) != s {
            log_debug!("{}: session does not match", function_name!());
        }

        (*ft).type_ = if !wp.is_null() {
            format_type::FORMAT_TYPE_PANE
        } else if wl.is_some() {
            format_type::FORMAT_TYPE_WINDOW
        } else if !s.is_null() {
            format_type::FORMAT_TYPE_SESSION
        } else {
            format_type::FORMAT_TYPE_UNKNOWN
        };

        if s.is_null() && !c.is_null() {
            s = client_get_session(c);
        }
        if wl.is_none() && !s.is_null() {
            wl = (*s).curw();
        }
        if wp.is_null()
            && let Some(wl) = wl
        {
            let w_a = winlink_window(wl);
            if !w_a.is_null() { wp = window_active_pane(w_a); }
        }

        if !c.is_null() {
            format_defaults_client(ft, c);
        }
        if !s.is_null() {
            format_defaults_session(ft, s);
        }
        if let Some(wl) = wl {
            format_defaults_winlink(ft, wl);
        }
        if !wp.is_null() {
            format_defaults_pane(ft, wp);
        }

        if let Some(pb) = paste_get_top() {
            format_defaults_paste_buffer(ft, pb);
        }
    }
}

/// Set default format keys for a session.
pub unsafe fn format_defaults_session(ft: *mut format_tree, s: *mut session) {
    unsafe {
        (*ft).s = if s.is_null() { None } else { Some(SessionId((*s).id)) };
    }
}

/// Set default format keys for a client.
pub unsafe fn format_defaults_client(ft: *mut format_tree, c: *mut client) {
    unsafe {
        if (*ft).s.is_none() {
            let s = client_get_session(c);
            (*ft).s = if s.is_null() { None } else { Some(SessionId((*s).id)) };
        }
        (*ft).c = c;
    }
}

/// Set default format keys for a window.
pub unsafe fn format_defaults_window(ft: *mut format_tree, w: *mut window) {
    unsafe {
        (*ft).w = if w.is_null() { None } else { Some(WindowId((*w).id)) };
    }
}

/// Set default format keys for a winlink.
pub unsafe fn format_defaults_winlink(ft: *mut format_tree, wl: Winlink) {
    unsafe {
        if (*ft).w.is_none() {
            let w_def = winlink_window(wl);
            format_defaults_window(ft, w_def);
        }
        (*ft).wl = Some(wl);
    }
}

/// Set default format keys for a window pane.
pub unsafe fn format_defaults_pane(ft: *mut format_tree, wp: *mut window_pane) {
    unsafe {
        if (*ft).w.is_none() {
            format_defaults_window(ft, window_pane_window(wp));
        }
        (*ft).wp = if wp.is_null() { None } else { Some(PaneId((*wp).id)) };

        if let Some(wme) = (*wp).modes.first().copied().and_then(NonNull::new)
            && let Some(formats) = (*(*wme.as_ptr()).mode).formats
        {
            formats(wme.as_ptr(), ft);
        }
    }
}

/// Set default format keys for paste buffer.
///
/// # Safety
/// `ft` must be a valid format tree from [`format_create`].
pub unsafe fn format_defaults_paste_buffer(ft: *mut format_tree, pb: PasteBuffer) {
    unsafe {
        (*ft).pb = Some(pb);
    }
}

/// Return word at given coordinates. Caller frees.
pub unsafe fn format_grid_word(gd: *mut Grid, mut x: u32, mut y: u32) -> String {
    unsafe {
        let mut ud: Vec<Utf8Data> = Vec::new();
        let mut gc: GridCell;
        let mut found = false;

        let ws_ptr: *const u8 = options_get_string_(GLOBAL_S_OPTIONS, "word-separators");
        let ws = std::ffi::CStr::from_ptr(ws_ptr.cast());

        loop {
            gc = (*gd).get_cell(x, y);
            if gc.flags.intersects(GridFlag::PADDING) {
                break;
            }
            if gc.data.in_cstr(ws)
                || (gc.data.size == 1 && gc.data.data[0] == b' ')
            {
                found = true;
                break;
            }

            if x == 0 {
                if y == 0 {
                    break;
                }
                let wrapped = (*gd)
                    .peek_line(y - 1)
                    .is_some_and(|gl| gl.flags.intersects(GridLineFlag::WRAPPED));
                if !wrapped {
                    break;
                }
                y -= 1;
                x = (*gd).line_length(y);
                if x == 0 {
                    break;
                }
            }
            x -= 1;
        }
        loop {
            if found {
                let end = (*gd).line_length(y);
                if end == 0 || x == end - 1 {
                    if y == (*gd).hsize + (*gd).sy - 1 {
                        break;
                    }
                    let wrapped = (*gd)
                        .peek_line(y)
                        .is_some_and(|gl| gl.flags.intersects(GridLineFlag::WRAPPED));
                    if !wrapped {
                        break;
                    }
                    y += 1;
                    x = 0;
                } else {
                    x += 1;
                }
            }
            found = true;

            gc = (*gd).get_cell(x, y);
            if gc.flags.intersects(GridFlag::PADDING) {
                break;
            }
            if gc.data.in_cstr(ws)
                || (gc.data.size == 1 && gc.data.data[0] == b' ')
            {
                break;
            }

            ud.push(gc.data);
        }

        utf8_to_string(&ud)
    }
}

/// Return line at given coordinates. Caller frees.
pub unsafe fn format_grid_line(gd: *mut Grid, y: u32) -> String {
    unsafe {
        let mut ud: Vec<Utf8Data> = Vec::new();
        for x in 0..(*gd).line_length(y) {
            let gc = (*gd).get_cell(x, y);
            if gc.flags.intersects(GridFlag::PADDING) {
                break;
            }

            ud.push(gc.data);
        }
        utf8_to_string(&ud)
    }
}

/// Return hyperlink at given coordinates. Caller frees.
pub unsafe fn format_grid_hyperlink(
    gd: *mut Grid,
    x: u32,
    y: u32,
    s: *mut screen,
) -> Option<String> {
    unsafe {
        let mut uri: *const u8 = null();

        let gc = (*gd).get_cell(x, y);
        if gc.flags.intersects(GridFlag::PADDING) {
            return None;
        }
        if (*s).hyperlinks.is_none() || gc.link == 0 {
            return None;
        }
        if !hyperlinks_get(
            (*s).hyperlinks.unwrap_or(null_mut()),
            gc.link,
            &mut uri,
            null_mut(),
            null_mut(),
        ) {
            return None;
        }
        Some(cstr_to_str(uri).to_string())
    }
}

/// Fuzz-friendly wrapper: expands a format string with FORMAT_NOJOBS and null
/// context (no client, session, window, or pane). Cannot execute shell commands.
/// Initializes global options once.
#[cfg(fuzzing)]
pub fn fuzz_format_expand(input: &[u8]) {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| unsafe {
        use crate::options_::*;
        use crate::options_table::OPTIONS_TABLE;
        use crate::tmux::{GLOBAL_OPTIONS, GLOBAL_S_OPTIONS, GLOBAL_W_OPTIONS};
        GLOBAL_OPTIONS = options_create(null_mut());
        GLOBAL_S_OPTIONS = options_create(null_mut());
        GLOBAL_W_OPTIONS = options_create(null_mut());
        for oe in &OPTIONS_TABLE {
            if oe.scope & OPTIONS_TABLE_SERVER != 0 {
                options_default(GLOBAL_OPTIONS, oe);
            }
            if oe.scope & OPTIONS_TABLE_SESSION != 0 {
                options_default(GLOBAL_S_OPTIONS, oe);
            }
            if oe.scope & OPTIONS_TABLE_WINDOW != 0 {
                options_default(GLOBAL_W_OPTIONS, oe);
            }
        }
    });

    // Must be NUL-terminated for C interop.
    if input.contains(&0) {
        return;
    }
    let mut cstr = Vec::with_capacity(input.len() + 1);
    cstr.extend_from_slice(input);
    cstr.push(0);

    unsafe {
        let ft = format_create(
            null_mut(),
            null_mut(),
            FORMAT_NONE,
            format_flags::FORMAT_NOJOBS,
        );
        format_defaults(ft, null_mut(), None, None, None);
        let result = format_expand(ft, cstr.as_ptr());
        free_(result);
        format_free(ft);
    }
}

