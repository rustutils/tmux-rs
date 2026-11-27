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

//! The adapter between format trees and the format engine (tmux-format).
//!
//! [`ServerHost`] gives the engine the clock, strftime, regular
//! expressions and colours through libc and the other crates.
//! [`TreeEnv`] is a `format_tree` seen through the engine's `FormatEnv` trait:
//! lookups in `format_find`'s order (options, the callback table, the
//! `format_add` entries, the environment), the children of the loops built
//! as format.c's loop functions build them, `#()` jobs kept in
//! [`FormatJobs`] stores, and the attached client's terminal.

use core::ffi::CStr;
use std::ffi::CString;
use std::ptr::{NonNull, null_mut};

use tmux_format::{
    ClientQuery, ExpandFlags, FormatEnv, Host, LoopChild, LoopKind, LoopSort, NameKind, Tm, Value,
};

use crate::colour::colour_toescape;
use crate::format_jobs::FormatJobs;
use crate::global::{with_format_engine, with_format_jobs};
use crate::options_::{
    ArrayKey, options_array_keys, options_entries, options_is_array, options_name, options_parse_get,
    options_table_entry, options_to_string,
};
use crate::sort::{SortCriteria, SortOrder};
use crate::*;

/// Expands `fmt` with the tree `ft`: `format_expand`, or `format_expand_time`
/// when `time` is set.
///
/// # Safety
/// `ft` must be a valid format tree whose client, item and entities are
/// valid for the expansion.
pub unsafe fn format_engine_expand(ft: *mut format_tree, fmt: &[u8], time: bool) -> Vec<u8> {
    // SAFETY: the caller passes a valid tree, the env does not outlive it.
    let env = unsafe { TreeEnv::borrowed(ft) };
    with_format_engine(|engine| engine.expand(&ServerHost, &env, fmt, ExpandFlags { time }))
}

/// The engine's view of the server: libc's clock and strftime, tmux-regsub
/// for regular expressions, colour.rs and fuzzy.rs.
pub struct ServerHost;

impl Host for ServerHost {
    fn now(&self) -> i64 {
        // SAFETY: time(3) accepts a null pointer and only returns the time.
        unsafe { ::libc::time(null_mut()) }
    }

    fn timer_ms(&self) -> u64 {
        // SAFETY: get_timer only reads the clocks into a local.
        unsafe { get_timer() }
    }

    fn local_time(&self, t: i64) -> Option<Tm> {
        let mut tm = std::mem::MaybeUninit::<::libc::tm>::zeroed();
        // SAFETY: localtime_r reads `t` and writes the broken down time into
        // `tm`, a struct of plain fields that all zeroes is valid for.
        let tm = unsafe {
            if ::libc::localtime_r(&t, tm.as_mut_ptr()).is_null() {
                return None;
            }
            tm.assume_init()
        };
        Some(Tm {
            sec: tm.tm_sec,
            min: tm.tm_min,
            hour: tm.tm_hour,
            mday: tm.tm_mday,
            mon: tm.tm_mon,
            year: tm.tm_year,
            wday: tm.tm_wday,
            yday: tm.tm_yday,
            isdst: tm.tm_isdst,
        })
    }

    fn strftime(&self, fmt: &[u8], tm: &Tm, max: usize) -> Option<Vec<u8>> {
        let fmt = CString::new(fmt).ok()?;
        // SAFETY: all zeroes is a valid struct tm.
        let mut ctm: ::libc::tm = unsafe { std::mem::zeroed() };
        ctm.tm_sec = tm.sec;
        ctm.tm_min = tm.min;
        ctm.tm_hour = tm.hour;
        ctm.tm_mday = tm.mday;
        ctm.tm_mon = tm.mon;
        ctm.tm_year = tm.year;
        ctm.tm_isdst = tm.isdst;
        // mktime fills in the time zone (%Z, %z) and the days for the
        // local time localtime_r gave, without changing it.
        // SAFETY: mktime only reads and normalises the struct.
        unsafe { ::libc::mktime(&raw mut ctm) };
        let mut buf = vec![0u8; max.max(1)];
        // SAFETY: `buf` has `max` bytes, `fmt` is a C string and `ctm` a
        // valid struct tm. strftime writes at most `max` bytes.
        let n = unsafe { ::libc::strftime(buf.as_mut_ptr().cast(), buf.len(), fmt.as_ptr(), &raw const ctm) };
        if n == 0 {
            return None;
        }
        buf.truncate(n);
        Some(buf)
    }

    fn fnmatch(&self, pattern: &[u8], text: &[u8], casefold: bool) -> bool {
        let (Ok(pattern), Ok(text)) = (CString::new(pattern), CString::new(text)) else {
            return false;
        };
        let flags = if casefold { ::libc::FNM_CASEFOLD } else { 0 };
        // SAFETY: both are C strings that fnmatch only reads.
        unsafe { ::libc::fnmatch(pattern.as_ptr(), text.as_ptr(), flags) == 0 }
    }

    fn regex_match(&self, pattern: &[u8], text: &[u8], icase: bool) -> bool {
        let (Ok(pattern), Ok(text)) = (CString::new(pattern), CString::new(text)) else {
            return false;
        };
        let flags = ::libc::REG_EXTENDED | if icase { ::libc::REG_ICASE } else { 0 };
        tmux_regsub::regmatch(&pattern, &text, flags)
    }

    fn regex_sub(&self, pattern: &[u8], with: &[u8], text: &[u8], icase: bool) -> Option<Vec<u8>> {
        let pattern = CString::new(pattern).ok()?;
        let with = CString::new(with).ok()?;
        let text = CString::new(text).ok()?;
        let flags = ::libc::REG_EXTENDED | if icase { ::libc::REG_ICASE } else { 0 };
        tmux_regsub::regsub(&pattern, &with, &text, flags)
    }

    fn colour(&self, name: &[u8]) -> Option<i32> {
        let name = std::str::from_utf8(name).ok()?;
        let c = colour_fromstring(name);
        (c != -1).then_some(c)
    }

    fn colour_rgb(&self, colour: i32) -> Option<u32> {
        let c = colour_force_rgb(colour);
        (c != -1).then_some(c as u32)
    }

    fn fuzzy_match(&self, pattern: &[u8], text: &[u8], width: u32) -> Option<Vec<bool>> {
        // Each style directive is parsed on top of the last, as C does.
        // SAFETY: all zeroes is a valid style, style_set then fills it.
        let mut sy: style = unsafe { std::mem::zeroed() };
        // SAFETY: `sy` and the default cell are valid.
        unsafe { style_set(&raw mut sy, &raw const GRID_DEFAULT_CELL) };
        let parse_style = |inner: &[u8]| {
            let inner = CString::new(inner).ok()?;
            // SAFETY: `sy` is a valid style and `inner` a C string.
            let ok = unsafe { style_parse(&raw mut sy, &raw const GRID_DEFAULT_CELL, inner.as_ptr().cast()) } == 0;
            ok.then_some(sy.align)
        };
        let m = crate::fuzzy::fuzzy_match(pattern, text, width, parse_style)?;
        Some((0..width).map(|i| m.columns.bit_test(i)).collect())
    }
}

/// A format tree seen by the engine. It either borrows a tree the caller
/// owns or owns one built for a loop item and frees it when dropped.
pub struct TreeEnv {
    ft: *mut format_tree,
    owned: bool,
    /// For a loop item, the line C logs before expanding it.
    loop_message: Option<Vec<u8>>,
}

impl Drop for TreeEnv {
    fn drop(&mut self) {
        if self.owned {
            // SAFETY: an owned tree came from format_create in this module
            // and nothing else holds it.
            unsafe { format_free(self.ft) };
        }
    }
}

impl TreeEnv {
    /// Wraps a tree the caller keeps.
    ///
    /// # Safety
    /// `ft` must be a valid format tree, with a valid client, item and
    /// entities, for as long as the env lives. Every method below relies
    /// on this.
    pub unsafe fn borrowed(ft: *mut format_tree) -> Self {
        Self { ft, owned: false, loop_message: None }
    }

    /// Takes over a tree from `format_create`.
    ///
    /// # Safety
    /// As for [`TreeEnv::borrowed`], and the tree is freed on drop.
    unsafe fn owned(ft: *mut format_tree) -> Self {
        Self { ft, owned: true, loop_message: None }
    }

    /// A loop item.
    fn child<'a>(ft: *mut format_tree, active: bool, message: String) -> LoopChild<'a> {
        // SAFETY: `ft` was just made by format_create from the parent's
        // valid client and item, and format_defaults filled it.
        let mut env = unsafe { Self::owned(ft) };
        env.loop_message = Some(message.into_bytes());
        LoopChild { env: Box::new(env), active }
    }
}

/// The sort criteria for a loop's sort.
fn criteria(sort: LoopSort) -> SortCriteria {
    let order = match sort.order {
        tmux_format::SortOrder::Order => SortOrder::Order,
        tmux_format::SortOrder::Index => SortOrder::Index,
        tmux_format::SortOrder::Name => SortOrder::Name,
        tmux_format::SortOrder::Activity => SortOrder::Activity,
        tmux_format::SortOrder::Creation => SortOrder::Creation,
        tmux_format::SortOrder::Z => SortOrder::Z,
    };
    SortCriteria::new(Some(order), sort.reversed)
}

/// `format_table_get` by bytes.
fn table_get(key: &[u8]) -> Option<&'static format_table_entry> {
    let i = FORMAT_TABLE.binary_search_by(|e| e.key.as_bytes().cmp(key)).ok()?;
    Some(&FORMAT_TABLE[i])
}

/// `format_find`'s lookup, before the modifiers.
///
/// # Safety
/// `ft` must be a valid tree with valid entities.
unsafe fn format_lookup(ft: &format_tree, key: &[u8], for_time: bool) -> Option<Value> {
    unsafe {
        if let Ok(name) = std::str::from_utf8(key) {
            let mut idx: Option<ArrayKey> = None;
            let wp = ft.wp.and_then(|id| pane_from_id(id));
            let w = ft.w.and_then(|id| window_from_id(id));
            let s = ft.s.and_then(|id| session_from_id(id));
            let mut o = options_parse_get(GLOBAL_OPTIONS, name, &mut idx, false);
            if o.is_null()
                && let Some(wp) = wp
            {
                o = options_parse_get((*wp).options, name, &mut idx, false);
            }
            if o.is_null()
                && let Some(w) = w
            {
                o = options_parse_get((*w).options, name, &mut idx, false);
            }
            if o.is_null() {
                o = options_parse_get(GLOBAL_W_OPTIONS, name, &mut idx, false);
            }
            if o.is_null()
                && let Some(s) = s
            {
                o = options_parse_get((*s).options, name, &mut idx, false);
            }
            if o.is_null() {
                o = options_parse_get(GLOBAL_S_OPTIONS, name, &mut idx, false);
            }
            if !o.is_null() {
                return Some(Value::Bytes(options_to_string(o, idx.as_ref(), true).into_bytes()));
            }
        }

        if let Some(fte) = table_get(key) {
            return match (fte.cb)(ft) {
                format_table_type::Time(tv) => Some(Value::Time(tv.tv_sec)),
                format_table_type::String(s) => Some(Value::Bytes(s.into_owned().into_bytes())),
                format_table_type::None => None,
            };
        }

        if let Some(fe) = ft.tree.get(key) {
            return Some(match fe {
                format_entry::Time(t) => Value::Time(*t),
                format_entry::Value(v) => Value::Bytes(v.clone()),
                format_entry::Callback(cb, cached) => Value::Bytes(
                    cached
                        .get_or_init(|| match cb(ft) {
                            format_table_type::String(s) => s.into_owned().into_bytes(),
                            _ => Vec::new(),
                        })
                        .clone(),
                ),
            });
        }

        if !for_time {
            // A session entry, even a removed one, hides the global one.
            let s = ft.s.and_then(|id| session_from_id(id));
            let value = match s.and_then(|s| (*s).environ.find(key)) {
                Some(entry) => entry.value().map(<[u8]>::to_vec),
                None => global_environ_value(key),
            };
            if let Some(value) = value {
                return Some(Value::Bytes(value));
            }
        }
        None
    }
}

/// `format_add_window_neighbour`: the index, active flag and user options
/// of the window before or after a window in a `W:` loop.
///
/// # Safety
/// `nft` must be a valid tree, `s` a valid session.
unsafe fn add_window_neighbour(nft: *mut format_tree, wl: Winlink, s: *mut session, prefix: &str) {
    unsafe {
        format_add!(nft, &format!("{prefix}_window_index"), "{}", wl.idx);
        format_add!(
            nft,
            &format!("{prefix}_window_active"),
            "{}",
            i32::from(Some(wl.id) == (*s).curw_id())
        );
        let w = winlink_window(wl);
        if w.is_null() {
            return;
        }
        for o in options_entries((*w).options) {
            let name = options_name(o);
            if name.starts_with('@') {
                let value = options_to_string(o, None, true);
                format_add!(nft, &format!("{prefix}_{name}"), "{value}");
            }
        }
    }
}

impl TreeEnv {
    /// The tree.
    fn ft(&self) -> &format_tree {
        // SAFETY: valid for the env's life, see TreeEnv::borrowed.
        unsafe { &*self.ft }
    }

    /// `format_loop_sessions`.
    fn loop_sessions(&self, sort: LoopSort) -> Vec<LoopChild<'_>> {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries and sort_get only hand out live entities.
        unsafe {
            let ft = self.ft;
            let current = if (*ft).c.is_null() { null_mut() } else { client_get_session((*ft).c) };
            let l = sort_get_sessions(&criteria(sort));
            let n = l.len();
            let mut out = Vec::with_capacity(n);
            for (i, s) in l.into_iter().enumerate() {
                let nft = format_create((*ft).client, (*ft).item, FORMAT_NONE, (*ft).flags);
                format_add!(nft, "loop_index", "{i}");
                format_add!(nft, "loop_last_flag", "{}", i32::from(i + 1 == n));
                format_defaults(nft, (*ft).c, NonNull::new(s), None, None);
                let active = !current.is_null() && (*s).id == (*current).id;
                out.push(Self::child(nft, active, format!("session loop: ${}", (*s).id)));
            }
            out
        }
    }

    /// `format_loop_windows`, with 3.8's neighbour variables.
    fn loop_windows(&self, sort: LoopSort) -> Option<Vec<LoopChild<'_>>> {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries and sort_get only hand out live entities.
        unsafe {
            let ft = self.ft;
            let s = (*ft).s.and_then(|id| session_from_id(id))?;
            let l = sort_get_winlinks_session(s, &criteria(sort));
            let n = l.len();
            let curw = (*s).curw_id();
            let mut out = Vec::with_capacity(n);
            for (i, &wl) in l.iter().enumerate() {
                let w = winlink_window(wl);
                let nft = format_create((*ft).client, (*ft).item, FORMAT_WINDOW as i32 | (*w).id as i32, (*ft).flags);
                format_add!(nft, "loop_index", "{i}");
                format_add!(nft, "loop_last_flag", "{}", i32::from(i + 1 == n));
                let after = i > 0 && Some(l[i - 1].id) == curw;
                let before = i + 1 < n && Some(l[i + 1].id) == curw;
                format_add!(nft, "window_after_active", "{}", i32::from(after));
                format_add!(nft, "window_before_active", "{}", i32::from(before));
                if i + 1 < n {
                    add_window_neighbour(nft, l[i + 1], s, "next");
                }
                if i > 0 {
                    add_window_neighbour(nft, l[i - 1], s, "prev");
                }
                format_defaults(nft, (*ft).c, NonNull::new(s), Some(wl.id), None);
                out.push(Self::child(nft, Some(wl.id) == curw, format!("window loop: {} @{}", wl.idx, (*w).id)));
            }
            Some(out)
        }
    }

    /// `format_loop_panes`.
    fn loop_panes(&self, sort: LoopSort) -> Option<Vec<LoopChild<'_>>> {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries and sort_get only hand out live entities.
        unsafe {
            let ft = self.ft;
            let w = (*ft).w.and_then(|id| window_from_id(id))?;
            let s = (*ft).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
            let l = sort_get_panes_window(w, &criteria(sort));
            let n = l.len();
            let active = window_active_pane(w);
            let mut out = Vec::with_capacity(n);
            for (i, wp) in l.into_iter().enumerate() {
                let nft = format_create((*ft).client, (*ft).item, FORMAT_PANE as i32 | (*wp).id as i32, (*ft).flags);
                format_add!(nft, "loop_index", "{i}");
                format_add!(nft, "loop_last_flag", "{}", i32::from(i + 1 == n));
                format_defaults(nft, (*ft).c, NonNull::new(s), (*ft).wl.map(|wl| wl.id), NonNull::new(wp));
                out.push(Self::child(nft, wp == active, format!("pane loop: %{}", (*wp).id)));
            }
            Some(out)
        }
    }

    /// `format_loop_clients`.
    fn loop_clients(&self, sort: LoopSort) -> Vec<LoopChild<'_>> {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries and sort_get only hand out live entities.
        unsafe {
            let ft = self.ft;
            let s = (*ft).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
            let wp = (*ft).wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
            let l = sort_get_clients(&criteria(sort));
            let n = l.len();
            let mut out = Vec::with_capacity(n);
            for (i, c) in l.into_iter().enumerate() {
                let nft = format_create(c, (*ft).item, 0, (*ft).flags);
                format_add!(nft, "loop_index", "{i}");
                format_add!(nft, "loop_last_flag", "{}", i32::from(i + 1 == n));
                format_defaults(nft, c, NonNull::new(s), (*ft).wl.map(|wl| wl.id), NonNull::new(wp));
                out.push(Self::child(nft, false, format!("client loop: {}", _s((*c).name))));
            }
            out
        }
    }

    /// A new child tree for the `O:` and `V:` loops, with the parent's
    /// client, session, winlink and pane.
    fn scoped_child(&self) -> *mut format_tree {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries and sort_get only hand out live entities.
        unsafe {
            let ft = self.ft;
            format_create((*ft).client, (*ft).item, FORMAT_NONE, (*ft).flags)
        }
    }

    /// Fills in a child of the `O:` and `V:` loops after its own entries.
    fn scoped_defaults(&self, nft: *mut format_tree) {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries and sort_get only hand out live entities.
        unsafe {
            let ft = self.ft;
            let s = (*ft).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
            let wp = (*ft).wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
            format_defaults(nft, (*ft).c, NonNull::new(s), (*ft).wl.map(|wl| wl.id), NonNull::new(wp));
        }
    }

    /// `format_loop_options`: the options of the scope the flags pick, one
    /// item per option or per array item.
    fn loop_options(&self, flags: &[u8]) -> Vec<LoopChild<'_>> {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries and sort_get only hand out live entities.
        unsafe {
            let ft = self.ft;
            let flags: &[u8] = if flags.is_empty() { b"s" } else { flags };
            let has = |c: u8| flags.contains(&c);
            let oo: *mut options = if has(b'v') {
                GLOBAL_OPTIONS
            } else {
                let global = has(b'g');
                if has(b'w') {
                    if global {
                        GLOBAL_W_OPTIONS
                    } else {
                        (*ft).w.and_then(|id| window_from_id(id)).map_or(null_mut(), |w| (*w).options)
                    }
                } else if has(b's') {
                    if global {
                        GLOBAL_S_OPTIONS
                    } else {
                        (*ft).s.and_then(|id| session_from_id(id)).map_or(null_mut(), |s| (*s).options)
                    }
                } else if has(b'p') {
                    if global {
                        null_mut()
                    } else {
                        (*ft).wp.and_then(|id| pane_from_id(id)).map_or(null_mut(), |wp| (*wp).options)
                    }
                } else if global {
                    GLOBAL_S_OPTIONS
                } else {
                    null_mut()
                }
            };
            if oo.is_null() {
                return Vec::new();
            }

            let entries = options_entries(oo);
            let mut out = Vec::new();
            let mut i = 0u32;
            for (at, &o) in entries.iter().enumerate() {
                let last_option = at + 1 == entries.len();
                let name = options_name(o);
                let oe = options_table_entry(o);
                let is_hook = !oe.is_null() && (*oe).flags & OPTIONS_TABLE_IS_HOOK != 0;
                let is_array = options_is_array(o);
                let keys = if is_array { options_array_keys(o) } else { Vec::new() };
                let n = keys.len();
                let common = |nft: *mut format_tree| {
                    format_add!(nft, "option_name", "{name}");
                    format_add!(nft, "option_array_count", "{n}");
                    format_add!(nft, "option_is_hook", "{}", i32::from(is_hook));
                    format_add!(nft, "option_is_user", "{}", i32::from(oe.is_null()));
                };
                if keys.is_empty() {
                    let nft = self.scoped_child();
                    common(nft);
                    format_add!(nft, "option_value", "{}", options_to_string(o, None, false));
                    format_add!(nft, "option_is_array", "{}", i32::from(is_array));
                    format_add!(nft, "option_array_key", "");
                    format_add!(nft, "option_array_index", "");
                    format_add!(nft, "option_array_first", "{}", i32::from(is_array));
                    format_add!(nft, "option_array_last", "{}", i32::from(is_array));
                    format_add!(nft, "loop_last_flag", "{}", i32::from(last_option));
                    format_add!(nft, "loop_index", "{i}");
                    self.scoped_defaults(nft);
                    out.push(Self::child(nft, false, format!("option loop: {name}")));
                    i += 1;
                    continue;
                }
                for (k, key) in keys.iter().enumerate() {
                    let nft = self.scoped_child();
                    common(nft);
                    format_add!(nft, "option_value", "{}", options_to_string(o, Some(key), false));
                    format_add!(nft, "option_is_array", "1");
                    format_add!(nft, "option_array_key", "{key}");
                    format_add!(nft, "option_array_index", "{key}");
                    format_add!(nft, "option_array_first", "{}", i32::from(k == 0));
                    format_add!(nft, "option_array_last", "{}", i32::from(k + 1 == n));
                    format_add!(nft, "loop_last_flag", "{}", i32::from(k + 1 == n && last_option));
                    format_add!(nft, "loop_index", "{i}");
                    self.scoped_defaults(nft);
                    out.push(Self::child(nft, false, format!("option loop: {name}[{key}]")));
                    i += 1;
                }
            }
            out
        }
    }

    /// `format_loop_environ`: the variables of the session (`s` or no
    /// flags), global (`g`) or client (`c`) environment.
    fn loop_environ(&self, flags: &[u8]) -> Vec<LoopChild<'_>> {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries and sort_get only hand out live entities.
        unsafe {
            let ft = self.ft;
            let env: Option<Environ> = match flags {
                b"" | b"s" => (*ft).s.and_then(|id| session_from_id(id)).map(|s| (*(*s).environ).clone()),
                b"g" => {
                    let mut copy = None;
                    global_environ_change(|env| copy = Some(env.clone()));
                    copy
                }
                b"c" => (!(*ft).client.is_null()).then(|| (*(*(*ft).client).environ).clone()),
                _ => None,
            };
            let Some(env) = env else {
                return Vec::new();
            };
            let entries: Vec<&EnvironEntry> = env.iter().collect();
            let n = entries.len();
            let mut out = Vec::with_capacity(n);
            for (i, entry) in entries.into_iter().enumerate() {
                let nft = self.scoped_child();
                format_add_bytes(nft, b"environ_name", entry.name());
                format_add_bytes(nft, b"environ_value", entry.value().unwrap_or(b""));
                format_add!(nft, "environ_hidden", "{}", i32::from(entry.is_hidden()));
                format_add!(nft, "environ_removed", "{}", i32::from(entry.value().is_none()));
                format_add!(nft, "loop_last_flag", "{}", i32::from(i + 1 == n));
                format_add!(nft, "loop_index", "{i}");
                self.scoped_defaults(nft);
                let message = format!("environment loop: {}", String::from_utf8_lossy(entry.name()));
                out.push(Self::child(nft, false, message));
            }
            out
        }
    }
}

impl FormatEnv for TreeEnv {
    fn lookup(&self, name: &[u8], for_time: bool) -> Option<Value> {
        // SAFETY: the tree is valid (TreeEnv's invariant).
        unsafe { format_lookup(self.ft(), name, for_time) }
    }

    fn children(&self, kind: LoopKind, sort: LoopSort, flags: &[u8]) -> Option<Vec<LoopChild<'_>>> {
        match kind {
            LoopKind::Sessions => Some(self.loop_sessions(sort)),
            LoopKind::Windows => self.loop_windows(sort),
            LoopKind::Panes => self.loop_panes(sort),
            LoopKind::Clients => Some(self.loop_clients(sort)),
            LoopKind::Options => Some(self.loop_options(flags)),
            LoopKind::Environ => Some(self.loop_environ(flags)),
        }
    }

    fn jobs_disabled(&self) -> bool {
        self.ft().flags.intersects(format_flags::FORMAT_NOJOBS)
    }

    fn job(&self, cmd: &[u8], expanded: &[u8]) -> Option<Vec<u8>> {
        // SAFETY: the tree and its client are valid (TreeEnv's invariant).
        unsafe { format_job_get(self.ft, cmd, expanded) }
    }

    fn name_exists(&self, kind: NameKind, name: &[u8]) -> Option<bool> {
        // SAFETY: the tree is valid (TreeEnv's invariant), and the
        // registries only hand out live sessions and windows.
        unsafe {
            match kind {
                NameKind::Session => Some(sessions_iter().any(|s| (*s).name.as_bytes() == name)),
                NameKind::Window => {
                    let s = self.ft().s.and_then(|id| session_from_id(id))?;
                    Some((*s).windows().into_iter().any(|wl| {
                        let w = winlink_window(wl);
                        !w.is_null() && (*w).name.as_deref().unwrap_or("").as_bytes() == name
                    }))
                }
            }
        }
    }

    fn search_pane(&self, text: &[u8], regex: bool, icase: bool) -> Option<u32> {
        let term = CString::new(text).ok()?;
        // SAFETY: the registry hands out live panes and `term` is a C string.
        unsafe {
            let wp = self.ft().wp.and_then(|id| pane_from_id(id))?;
            Some(window_pane_search(wp, term.as_ptr().cast(), i32::from(regex), i32::from(icase)))
        }
    }

    fn client_query(&self, query: ClientQuery, name: &[u8]) -> Option<Vec<u8>> {
        let c = self.ft().c;
        // SAFETY: the tree's client is valid (TreeEnv's invariant), and an
        // attached client's terminal and environment are valid.
        unsafe {
            if c.is_null() || (*c).tty.term.is_null() || (*c).flags.intersects(CLIENT_UNATTACHEDFLAGS) {
                return None;
            }
            let flag = |b: bool| if b { b"1".to_vec() } else { b"0".to_vec() };
            Some(match query {
                ClientQuery::Capability => flag(tty_term_has_name((*c).tty.term, name)),
                ClientQuery::Feature => flag(tty_feature_present((*c).tty.term, name)),
                ClientQuery::Environ => (*c).environ.value(name).map(<[u8]>::to_vec).unwrap_or_default(),
            })
        }
    }

    fn colour_escape(&self, colour: i32, background: bool) -> Option<Vec<u8>> {
        let c = self.ft().c;
        let mut flags = term_flags::TERM_256COLOURS | term_flags::TERM_RGBCOLOURS;
        // SAFETY: the tree's client is valid (TreeEnv's invariant), and an
        // opened tty has a valid terminal.
        unsafe {
            if !c.is_null() && (*c).tty.flags.intersects(tty_flags::TTY_OPENED) && !(*c).tty.term.is_null() {
                flags = (*(*c).tty.term).flags;
            }
        }
        colour_toescape(colour, background, flags).map(String::into_bytes)
    }

    fn logging(&self) -> bool {
        format_logging(self.ft())
    }

    fn log(&self, depth: u32, message: &[u8]) {
        let ft = self.ft();
        let message = String::from_utf8_lossy(message);
        log_debug!("format: {}", message);
        if !ft.item.is_null() && ft.flags.intersects(format_flags::FORMAT_VERBOSE) {
            let spaces = &"          "[..(depth as usize).min(10)];
            // SAFETY: the tree's item is valid (TreeEnv's invariant).
            unsafe { cmdq_print!(ft.item, "#{spaces}{message}") };
        }
    }

    fn loop_message(&self) -> Option<Vec<u8>> {
        self.loop_message.clone()
    }

    fn pane_id(&self) -> Option<u32> {
        self.ft().wp.map(|id| id.0)
    }

    fn is_status(&self) -> bool {
        self.ft().flags.intersects(format_flags::FORMAT_STATUS)
    }

    fn start_cycle_timer(&self) {
        let c = self.ft().client;
        // SAFETY: the tree's client is valid (TreeEnv's invariant).
        unsafe {
            if !c.is_null() {
                format_cycle_start_timer(c);
            }
        }
    }
}

/// How often an animation of the `A` modifier redraws, in milliseconds
/// (`FORMAT_CYCLE_PERIOD`).
const FORMAT_CYCLE_PERIOD: u64 = 100;

/// `format_cycle_start_timer`: redraws the status line of `c` after one
/// animation period, unless a timer is already waiting.
///
/// # Safety
/// `c` must be a valid client.
unsafe fn format_cycle_start_timer(c: *mut client) {
    unsafe {
        if (*c).cycle_timer.is_some() {
            return;
        }
        let id = (*c).id;
        (*c).cycle_timer = timer_add(
            std::time::Duration::from_millis(FORMAT_CYCLE_PERIOD),
            Box::new(move || {
                let Some(c) = client_from_id(id) else {
                    return;
                };
                (*c).cycle_timer = None;
                if (*c).message_string.is_none() && (*c).prompt.is_none() {
                    (*c).flags |= client_flag::REDRAWSTATUS;
                }
            }),
        );
    }
}

/// `format_add` with a byte value and name, which `format_add`! cannot take.
///
/// # Safety
/// `ft` must be a valid tree.
pub unsafe fn format_add_bytes(ft: *mut format_tree, key: &[u8], value: &[u8]) {
    unsafe {
        (*ft).tree.insert(key.to_vec(), format_entry::Value(value.to_vec()));
    }
}

/// The `#()` jobs of `ft`'s client, or the bridge store without one.
enum JobOwner {
    Client(*mut client),
    Global,
}

impl JobOwner {
    /// Runs `f` with the store.
    ///
    /// # Safety
    /// A client owner must be valid.
    unsafe fn with<R>(&self, f: impl FnOnce(&mut FormatJobs<JobId>) -> R) -> R {
        match *self {
            // SAFETY: the caller passes a valid client.
            JobOwner::Client(c) => unsafe { f((*c).jobs.get_or_insert_with(Default::default)) },
            JobOwner::Global => with_format_jobs(f),
        }
    }
}

/// `format_job_get` without its expansion: starts or restarts the job for
/// `cmd` (now expanding to `expanded`) and returns its last output.
///
/// # Safety
/// `ft` must be a valid tree with a valid client.
unsafe fn format_job_get(ft: *mut format_tree, cmd: &[u8], expanded: &[u8]) -> Option<Vec<u8>> {
    unsafe {
        let owner = if (*ft).client.is_null() { JobOwner::Global } else { JobOwner::Client((*ft).client) };
        let tag = (*ft).tag;
        let now = ::libc::time(null_mut());
        let force = (*ft).flags.intersects(format_flags::FORMAT_FORCE);
        let decision = owner.with(|jobs| jobs.get(tag, cmd, expanded, now, force));
        if let Some(job) = decision.stop {
            job_free(job);
        }
        if decision.start {
            let cwd = server_client_get_cwd((*ft).client, null_mut());
            let cwd = CString::new(cwd.as_os_str().as_encoded_bytes()).unwrap_or_default();
            let command = CString::new(expanded).unwrap_or_default();
            let job = job_run(
                command.as_ptr().cast(),
                0,
                null_mut(),
                None,
                null_mut(),
                cwd.as_ptr().cast(),
                Some(format_job_update),
                Some(format_job_complete),
                None,
                null_mut(),
                job_flag::JOB_NOWAIT,
                -1,
                -1,
            );
            owner.with(|jobs| jobs.started(tag, cmd, job, now));
        }
        let status = (*ft).flags.intersects(format_flags::FORMAT_STATUS);
        owner.with(|jobs| jobs.output(tag, cmd, status))
    }
}

/// Finds the store that holds `job`: the bridge, or a client's.
///
/// # Safety
/// The clients must be valid.
unsafe fn job_owner(job: JobId) -> Option<JobOwner> {
    if with_format_jobs(|jobs| jobs.has_job(job)) {
        return Some(JobOwner::Global);
    }
    // SAFETY: clients_iter hands out live clients.
    unsafe {
        clients_iter()
            .find(|&c| (*c).jobs.as_ref().is_some_and(|jobs| jobs.has_job(job)))
            .map(JobOwner::Client)
    }
}

/// Redraws the status line of the client that owns a job's store.
///
/// # Safety
/// A client owner must be valid.
unsafe fn job_redraw(owner: &JobOwner) {
    if let JobOwner::Client(c) = *owner {
        // SAFETY: the caller passes a valid client.
        unsafe { server_status_client(c) };
    }
}

/// `format_job_update`: keeps the last complete line of output.
///
/// # Safety
/// `job` must be a running job.
pub unsafe fn format_job_update(job: JobId) {
    unsafe {
        let evb = job_get_input(job);
        let mut line = None;
        while let Some(next) = NonNull::new(evbuffer_readline(evb)) {
            line = Some(CStr::from_ptr(next.as_ptr().cast()).to_bytes().to_vec());
            free_(next.as_ptr());
        }
        let Some(line) = line else {
            return;
        };
        let Some(owner) = job_owner(job) else {
            return;
        };
        let now = ::libc::time(null_mut());
        if owner.with(|jobs| jobs.update(job, line, now)) {
            job_redraw(&owner);
        }
    }
}

/// `format_job_complete`: keeps the last line, or all the output without a
/// newline.
///
/// # Safety
/// `job` must be a job that just finished.
pub unsafe fn format_job_complete(job: JobId) {
    unsafe {
        let evb = job_get_input(job);
        let line = evbuffer_readline(evb);
        let out = if line.is_null() {
            let len = EVBUFFER_LENGTH(evb);
            if len == 0 { Vec::new() } else { std::slice::from_raw_parts(EVBUFFER_DATA(evb), len).to_vec() }
        } else {
            let v = CStr::from_ptr(line.cast()).to_bytes().to_vec();
            free_(line);
            v
        };
        let Some(owner) = job_owner(job) else {
            return;
        };
        if owner.with(|jobs| jobs.complete(job, out)) {
            job_redraw(&owner);
        }
    }
}

/// `format_tidy_jobs`: forgets the jobs that did not run for an hour.
///
/// # Safety
/// The clients must be valid.
pub unsafe fn format_tidy_jobs() {
    unsafe {
        let now = ::libc::time(null_mut());
        for job in with_format_jobs(|jobs| jobs.tidy(now, false)) {
            job_free(job);
        }
        for c in clients_iter() {
            let stop = match (*c).jobs.as_mut() {
                Some(jobs) => jobs.tidy(now, false),
                None => continue,
            };
            for job in stop {
                job_free(job);
            }
        }
    }
}

/// `format_lost_client`: stops and forgets the jobs of a client.
///
/// # Safety
/// `c` must be a valid client.
pub unsafe fn format_lost_client(c: *mut client) {
    unsafe {
        if let Some(mut jobs) = (*c).jobs.take() {
            for job in jobs.tidy(::libc::time(null_mut()), true) {
                job_free(job);
            }
        }
    }
}

/// `CStr` of a format, for the C-shaped callers.
///
/// # Safety
/// `fmt` must be null or a C string.
pub unsafe fn format_bytes<'a>(fmt: *const u8) -> &'a [u8] {
    if fmt.is_null() {
        return b"";
    }
    // SAFETY: the caller passes a C string.
    unsafe { CStr::from_ptr(fmt.cast()).to_bytes() }
}

/// A malloc'd C string of `v`, cut at a NUL if it has one, for the
/// C-shaped callers that free_ what `format_expand` returns.
pub fn format_to_c(v: &[u8]) -> *mut u8 {
    let n = v.iter().position(|&b| b == 0).unwrap_or(v.len());
    // SAFETY: xmalloc returns at least n + 1 writable bytes, which are
    // filled from `v` and terminated.
    unsafe {
        let p = xmalloc(n + 1).as_ptr().cast::<u8>();
        std::ptr::copy_nonoverlapping(v.as_ptr(), p, n);
        *p.add(n) = 0;
        p
    }
}
