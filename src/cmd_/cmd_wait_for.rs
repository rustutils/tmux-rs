// Copyright (c) 2013 Nicholas Marriott <nicholas.marriott@gmail.com>
// Copyright (c) 2013 Thiago de Arruda <tpadilha84@gmail.com>
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
//! wait-for: wait on a named channel or for an event.
//!
//! Channels: `wait-for name` blocks the client's command until another
//! client runs `wait-for -S name`, `-L` and `-U` lock and unlock a channel.
//!
//! Events (tmux 3.8): `wait-for -E name` blocks until an event called
//! `name` fires (a hook name or an `@` name from set-hook -E). `-F` waits
//! until the format, expanded with the event's payload, is true, and `-v`
//! prints each event's payload as it arrives. Each waiting command is an
//! event sink.
//!
//! For both, `-l` lists the clients waiting on a name and `-w client`
//! wakes one of them.
use std::cell::RefCell;
use std::collections::BTreeMap;

use crate::events::{Payload, Sink, SinkId, events_add_sink, events_remove_sink};
use crate::*;

pub static CMD_WAIT_FOR_ENTRY: cmd_entry = cmd_entry {
    name: "wait-for",
    alias: Some("wait"),

    args: args_parse::new("EF:LSUlvw:", 1, 1, None),
    usage: "[-ELSUlv] [-F format] [-w waiter] name",

    flags: cmd_flag::empty(),
    exec: CmdExec::Old(cmd_wait_for_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// A command waiting for an event.
struct WaitEvent {
    /// Identifies it to its sink.
    id: u32,
    item: *mut cmdq_item,
    sink: SinkId,
    name: String,
    filter: Option<String>,
    verbose: bool,
}

thread_local! {
    /// Commands waiting for events, oldest first, and the last id used.
    /// Borrowed only to look up or change the list, never across a call
    /// out.
    static WAIT_EVENTS: RefCell<(Vec<WaitEvent>, u32)> = const { RefCell::new((Vec::new(), 0)) };
}

/// Takes the event waiters matching `f` out of the list.
fn wait_events_take(f: impl Fn(&WaitEvent) -> bool) -> Vec<WaitEvent> {
    WAIT_EVENTS.with_borrow_mut(|(list, _)| {
        let (taken, kept) = std::mem::take(list).into_iter().partition(|w| f(w));
        *list = kept;
        taken
    })
}

/// The name of the client a waiting command belongs to, or "".
///
/// # Safety
/// `item` must be a valid command queue item.
unsafe fn cmd_wait_for_item_client_name(item: *mut cmdq_item) -> String {
    unsafe {
        let c = cmdq_get_client(item);
        if c.is_null() {
            return String::new();
        }
        cstr_to_string_lossy((*c).name)
    }
}

/// Lets a command that waited for an event run on, and removes its sink.
///
/// # Safety
/// `wei.item` must be a valid command queue item.
unsafe fn cmd_wait_for_event_done(wei: WaitEvent) {
    unsafe { cmdq_continue(wei.item) };
    events_remove_sink(wei.sink);
}

/// The wait-for sink: an event a command waits for has fired. Prints the
/// payload for -v, and lets the command run on unless the -F filter is
/// false.
///
/// # Safety
/// The waiting command must be valid, and the payload's entities valid or
/// gone from their registries.
pub unsafe fn cmd_wait_for_event(id: u32, payload: &Payload) {
    unsafe {
        let Some((item, filter, verbose)) = WAIT_EVENTS.with_borrow(|(list, _)| {
            list.iter()
                .find(|w| w.id == id)
                .map(|w| (w.item, w.filter.clone(), w.verbose))
        }) else {
            return;
        };

        if verbose {
            for line in payload.lines() {
                cmdq_print!(item, "{}", line);
            }
        }

        if let Some(filter) = filter {
            let ft = format_create(cmdq_get_client(item), item, FORMAT_NONE, format_flags::FORMAT_NOJOBS);
            for (key, value) in payload.formats("") {
                format_add!(ft, &key, "{}", value);
            }
            let filter = CString::new(filter).unwrap_or_default();
            let expanded = format_expand(ft, filter.as_ptr().cast());
            let flag = format_true(expanded);
            free_(expanded);
            format_free(ft);
            if !flag {
                return;
            }
        }

        for wei in wait_events_take(|w| w.id == id) {
            cmd_wait_for_event_done(wei);
        }
    }
}

/// wait-for -E: waits for an event, or lists (-l) or wakes (-w) waiters.
///
/// # Safety
/// `item` and `args` must be valid.
unsafe fn cmd_wait_for_event_exec(item: *mut cmdq_item, name: &str, args: *mut args) -> cmd_retval {
    unsafe {
        if !hooks_valid_event_name(name) {
            cmdq_error!(item, "invalid event: {}", name);
            return cmd_retval::CMD_RETURN_ERROR;
        }
        if args_has(args, 'l') {
            let items: Vec<*mut cmdq_item> = WAIT_EVENTS.with_borrow(|(list, _)| {
                list.iter().filter(|w| w.name == name).map(|w| w.item).collect()
            });
            for wi in items {
                cmdq_print!(item, "{}", cmd_wait_for_item_client_name(wi));
            }
            return cmd_retval::CMD_RETURN_NORMAL;
        }
        if args_has(args, 'w') {
            let client_name = cstr_to_string_lossy(args_get(args, b'w'));
            let candidates: Vec<(u32, *mut cmdq_item)> = WAIT_EVENTS.with_borrow(|(list, _)| {
                list.iter().filter(|w| w.name == name).map(|w| (w.id, w.item)).collect()
            });
            for (id, wi) in candidates {
                if cmd_wait_for_item_client_name(wi) == client_name {
                    for wei in wait_events_take(|w| w.id == id) {
                        cmd_wait_for_event_done(wei);
                    }
                    return cmd_retval::CMD_RETURN_NORMAL;
                }
            }
            cmdq_error!(item, "waiter {} not found", client_name);
            return cmd_retval::CMD_RETURN_ERROR;
        }

        if cmdq_get_client(item).is_null() {
            cmdq_error!(item, "not able to wait");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        let filter = args_get(args, b'F');
        let filter = (!filter.is_null()).then(|| cstr_to_string_lossy(filter));
        let id = WAIT_EVENTS.with_borrow_mut(|(_, last)| {
            *last += 1;
            *last
        });
        let sink = events_add_sink(name, Sink::WaitFor(id));
        WAIT_EVENTS.with_borrow_mut(|(list, _)| {
            list.push(WaitEvent {
                id,
                item,
                sink,
                name: name.to_string(),
                filter,
                verbose: args_has(args, 'v'),
            });
        });
        cmd_retval::CMD_RETURN_WAIT
    }
}

pub struct wait_channel {
    pub name: *mut u8,
    pub locked: bool,
    pub woken: bool,

    pub waiters: Vec<*mut cmdq_item>,
    pub lockers: Vec<*mut cmdq_item>,
}

static mut WAIT_CHANNELS: BTreeMap<String, Box<wait_channel>> = BTreeMap::new();

unsafe fn wait_channel_find(name: *const u8) -> *mut wait_channel {
    unsafe {
        let key = cstr_to_str(name);
        (*(&raw mut WAIT_CHANNELS))
            .get_mut(key)
            .map_or(null_mut(), |wc| &mut **wc as *mut wait_channel)
    }
}

pub unsafe fn cmd_wait_for_add(name: *const u8) -> *mut wait_channel {
    unsafe {
        let key = cstr_to_str(name).to_string();
        let wc = Box::new(wait_channel {
            name: xstrdup(name).as_ptr(),
            locked: false,
            woken: false,
            waiters: Vec::new(),
            lockers: Vec::new(),
        });

        log_debug!("add wait channel {}", _s(wc.name));

        (*(&raw mut WAIT_CHANNELS)).insert(key.clone(), wc);
        &mut **(*(&raw mut WAIT_CHANNELS)).get_mut(&key).unwrap() as *mut wait_channel
    }
}

pub unsafe fn cmd_wait_for_remove(wc: *mut wait_channel) {
    unsafe {
        if (*wc).locked {
            return;
        }
        if !(*wc).waiters.is_empty() || !(*wc).woken {
            return;
        }

        let key = cstr_to_str((*wc).name).to_string();
        log_debug!("remove wait channel {}", _s((*wc).name));

        if let Some(removed) = (*(&raw mut WAIT_CHANNELS)).remove(&key) {
            free_(removed.name);
        }
    }
}

pub unsafe fn cmd_wait_for_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let name = args_string(args, 0);

        if args_has(args, 'E') {
            return cmd_wait_for_event_exec(item, &cstr_to_string_lossy(name), args);
        }

        let wc = wait_channel_find(name);
        if args_has(args, 'l') {
            return cmd_wait_for_list(item, wc);
        }
        if args_has(args, 'w') {
            return cmd_wait_for_wake(args, wc);
        }
        if args_has(args, 'S') {
            return cmd_wait_for_signal(item, name, wc);
        }
        if args_has(args, 'L') {
            return cmd_wait_for_lock(item, name, wc);
        }
        if args_has(args, 'U') {
            return cmd_wait_for_unlock(item, name, wc);
        }

        cmd_wait_for_wait(item, name, wc)
    }
}

/// Removes a channel nobody waits on, holds or has signalled.
///
/// # Safety
/// `wc` must be a channel in the list.
unsafe fn cmd_wait_for_remove_empty(wc: *mut wait_channel) {
    unsafe {
        if (*wc).locked || (*wc).woken {
            return;
        }
        if !(*wc).waiters.is_empty() || !(*wc).lockers.is_empty() {
            return;
        }
        let key = cstr_to_str((*wc).name).to_string();
        log_debug!("remove empty wait channel {}", key);
        if let Some(removed) = (*(&raw mut WAIT_CHANNELS)).remove(&key) {
            free_(removed.name);
        }
    }
}

/// wait-for -l: the clients waiting on or locking a channel.
///
/// # Safety
/// `item` must be valid and `wc` null or a channel in the list.
unsafe fn cmd_wait_for_list(item: *mut cmdq_item, wc: *mut wait_channel) -> cmd_retval {
    unsafe {
        if wc.is_null() {
            return cmd_retval::CMD_RETURN_NORMAL;
        }
        let items: Vec<*mut cmdq_item> = (*wc).waiters.iter().chain(&(*wc).lockers).copied().collect();
        for wi in items {
            cmdq_print!(item, "{}", cmd_wait_for_item_client_name(wi));
        }
        cmd_retval::CMD_RETURN_NORMAL
    }
}

/// wait-for -w: lets the named client's command on a channel run on.
///
/// # Safety
/// `args` must be valid and `wc` null or a channel in the list.
unsafe fn cmd_wait_for_wake(args: *mut args, wc: *mut wait_channel) -> cmd_retval {
    unsafe {
        if wc.is_null() {
            return cmd_retval::CMD_RETURN_NORMAL;
        }
        let client_name = cstr_to_string_lossy(args_get(args, b'w'));
        for list in [&raw mut (*wc).waiters, &raw mut (*wc).lockers] {
            if let Some(i) = (*list).iter().position(|&wi| cmd_wait_for_item_client_name(wi) == client_name) {
                let wi = (*list).remove(i);
                cmdq_continue(wi);
                cmd_wait_for_remove_empty(wc);
                return cmd_retval::CMD_RETURN_NORMAL;
            }
        }
        cmd_retval::CMD_RETURN_NORMAL
    }
}

pub unsafe fn cmd_wait_for_signal(
    _item: *const cmdq_item,
    name: *const u8,
    mut wc: *mut wait_channel,
) -> cmd_retval {
    unsafe {
        if wc.is_null() {
            wc = cmd_wait_for_add(name);
        }

        if (*wc).waiters.is_empty() && !(*wc).woken {
            log_debug!("signal wait channel {}, no waiters", _s((*wc).name));
            (*wc).woken = true;
            return cmd_retval::CMD_RETURN_NORMAL;
        }
        log_debug!("signal wait channel {}, with waiters", _s((*wc).name));

        for &wi in &(*wc).waiters {
            cmdq_continue(wi);
        }
        (*wc).waiters.clear();

        cmd_wait_for_remove(wc);

        cmd_retval::CMD_RETURN_NORMAL
    }
}

pub unsafe fn cmd_wait_for_wait(
    item: *mut cmdq_item,
    name: *const u8,
    mut wc: *mut wait_channel,
) -> cmd_retval {
    unsafe {
        let c = cmdq_get_client(item);

        if c.is_null() {
            cmdq_error!(item, "not able to wait");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        if wc.is_null() {
            wc = cmd_wait_for_add(name);
        }

        if (*wc).woken {
            log_debug!("wait channel {} already woken ({:p})", _s((*wc).name), c);
            cmd_wait_for_remove(wc);
            return cmd_retval::CMD_RETURN_NORMAL;
        }
        log_debug!("wait channel {} not woken ({:p})", _s((*wc).name), c);

        (*wc).waiters.push(item);
    }
    cmd_retval::CMD_RETURN_WAIT
}

pub unsafe fn cmd_wait_for_lock(
    item: *mut cmdq_item,
    name: *const u8,
    mut wc: *mut wait_channel,
) -> cmd_retval {
    unsafe {
        if cmdq_get_client(item).is_null() {
            cmdq_error!(item, "not able to lock");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        if wc.is_null() {
            wc = cmd_wait_for_add(name);
        }

        if (*wc).locked {
            (*wc).lockers.push(item);
            return cmd_retval::CMD_RETURN_WAIT;
        }
        (*wc).locked = true;
    }
    cmd_retval::CMD_RETURN_NORMAL
}

pub unsafe fn cmd_wait_for_unlock(
    item: *mut cmdq_item,
    name: *const u8,
    wc: *mut wait_channel,
) -> cmd_retval {
    unsafe {
        if wc.is_null() || !(*wc).locked {
            cmdq_error!(item, "channel {} not locked", _s(name));
            return cmd_retval::CMD_RETURN_ERROR;
        }

        if !(*wc).lockers.is_empty() {
            let wi = (*wc).lockers.remove(0);
            cmdq_continue(wi);
        } else {
            (*wc).locked = false;
            cmd_wait_for_remove(wc);
        }
    }
    cmd_retval::CMD_RETURN_NORMAL
}

/// Release the items of a client that has gone away from every channel
/// they wait on, so the client and its queue can be freed. The items are
/// continued and then fail when they run, since their client is dead.
pub unsafe fn cmd_wait_for_client_lost(c: *mut client) {
    unsafe {
        for wei in wait_events_take(|w| cmdq_get_client(w.item) == c) {
            cmd_wait_for_event_done(wei);
        }

        let mut empty = Vec::new();
        for wc in (*(&raw mut WAIT_CHANNELS)).values_mut() {
            for list in [&mut wc.waiters, &mut wc.lockers] {
                list.retain(|&wi| {
                    if cmdq_get_client(wi) != c {
                        return true;
                    }
                    cmdq_continue(wi);
                    false
                });
            }
            empty.push(&mut **wc as *mut wait_channel);
        }
        for wc in empty {
            cmd_wait_for_remove_empty(wc);
        }
    }
}

pub unsafe fn cmd_wait_for_flush() {
    unsafe {
        for wei in wait_events_take(|_| true) {
            cmd_wait_for_event_done(wei);
        }

        let keys: Vec<String> = (*(&raw mut WAIT_CHANNELS)).keys().cloned().collect();
        for key in keys {
            let Some(wc) = (*(&raw mut WAIT_CHANNELS)).get_mut(&key) else {
                continue;
            };
            let wc = &mut **wc as *mut wait_channel;
            for &wi in &(*wc).waiters {
                cmdq_continue(wi);
            }
            (*wc).waiters.clear();
            (*wc).woken = true;
            for &wi in &(*wc).lockers {
                cmdq_continue(wi);
            }
            (*wc).lockers.clear();
            (*wc).locked = false;
            cmd_wait_for_remove(wc);
        }
    }
}
