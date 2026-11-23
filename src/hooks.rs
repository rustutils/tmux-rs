// Copyright (c) 2026 Nicholas Marriott <nicholas.marriott@gmail.com>
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

//! Hooks: commands run when an event fires.
//!
//! A hook is an option whose value is a list of commands (`set-hook -g
//! window-renamed 'display hi'`), or a user option starting with `@` set
//! with set-hook. Each hook name has an event sink. When the event fires,
//! the sink finds the option on the event's target (session options, then
//! pane, then window) and queues its commands, after the command the hook
//! belongs to or at the end of the queue. The commands run with the event's
//! payload as format variables prefixed `hook_` (`#{hook_window}`), and
//! `#{hook}` is the hook name.
//!
//! Commands started by a hook do not fire hooks themselves
//! (`CMDQ_STATE_NOHOOKS`), so a hook cannot trigger itself.
//!
//! Hook monitors (set-hook -B) are not ported yet. In C they live here
//! too and are stored on the option entries, so they wait for the options
//! conversion.

use crate::events::{Payload, Sink, events_add_sink, events_has_sink, payload_get_target};
use crate::options_::*;
use crate::options_table::OPTIONS_TABLE;
use crate::*;

/// What `hooks_insert` needs to queue a hook's commands.
struct HookData<'a> {
    name: &'a str,
    fs: cmd_find_state,
    /// The hook_* variables for the commands.
    formats: *mut format_tree,
}

/// Queues one hook command list after `item`, or at the end of the global
/// queue when `item` is null. Returns the new item, so the next list goes
/// after it.
///
/// # Safety
/// `item` must be null or valid, `cmdlist` and `state` valid or null.
unsafe fn hooks_insert_one(
    item: *mut cmdq_item,
    hd: &HookData,
    cmdlist: *mut cmd_list,
    state: *mut cmdq_state,
) -> *mut cmdq_item {
    unsafe {
        if cmdlist.is_null() {
            return item;
        }
        if log_get_level() != 0 {
            let s = cmd_list_print(&*cmdlist, 0);
            log_debug!("hooks_insert_one: hook {} is: {}", hd.name, _s(s));
            free_(s);
        }
        let new_item = cmdq_get_command(cmdlist, state);
        if !item.is_null() {
            cmdq_insert_after(item, new_item)
        } else {
            cmdq_append(null_mut(), new_item)
        }
    }
}

/// Finds the hook option for the target and queues its commands.
///
/// # Safety
/// `item` must be null or valid. The target in `hd` must be empty or
/// name live entities.
unsafe fn hooks_insert(mut item: *mut cmdq_item, hd: &mut HookData) {
    let __func__ = "hooks_insert";
    unsafe {
        log_debug!("{}: inserting hook {}", __func__, hd.name);

        let mut fs: cmd_find_state = zeroed();
        cmd_find_clear_state(&raw mut fs, cmd_find_flags::empty());
        if cmd_find_empty_state(&raw const hd.fs) != 0 || !cmd_find_valid_state(&raw const hd.fs) {
            cmd_find_from_nothing(&raw mut fs, cmd_find_flags::empty());
        } else {
            cmd_find_copy_state(&raw mut fs, &raw const hd.fs);
        }

        let mut oo = match fs.s.and_then(|id| session_from_id(id)) {
            Some(s) => (*s).options,
            None => GLOBAL_S_OPTIONS,
        };
        let mut o = options_get(&mut *oo, hd.name);
        if o.is_null()
            && let Some(wp) = fs.wp.and_then(|id| pane_from_id(id))
        {
            oo = (*wp).options;
            o = options_get(&mut *oo, hd.name);
        }
        if o.is_null() && !fs.wl.is_null() {
            oo = (*winlink_window(fs.wl)).options;
            o = options_get(&mut *oo, hd.name);
        }
        if o.is_null() {
            log_debug!("{}: hook {} not found", __func__, hd.name);
            return;
        }
        options_hook_fired(o);

        let event = if item.is_null() {
            null_mut()
        } else {
            cmdq_get_event(item)
        };
        let state = cmdq_new_state(&raw mut fs, event, cmdq_state_flags::CMDQ_STATE_NOHOOKS);
        cmdq_add_formats(state, hd.formats);

        if hd.name.starts_with('@') {
            let value = options_get_string(oo, hd.name);
            match cmd_parse_from_string(&cstr_to_string_lossy(value), None) {
                Err(error) => {
                    log_debug!(
                        "{}: can't parse hook {}: {}",
                        __func__,
                        hd.name,
                        error.to_string_lossy()
                    );
                }
                Ok(cmdlist) => {
                    hooks_insert_one(item, hd, cmdlist, state);
                }
            }
        } else {
            for (_, cmdlist) in options_array_commands(o) {
                item = hooks_insert_one(item, hd, cmdlist, state);
            }
        }

        cmdq_free_state(state);
    }
}

/// Queues the hook for an event, with the payload as hook_* variables.
///
/// # Safety
/// `item` must be null or valid, and the payload's entities valid or gone
/// from their registries.
unsafe fn hooks_insert_event(item: *mut cmdq_item, name: &str, payload: &Payload) {
    unsafe {
        if !item.is_null()
            && cmdq_get_flags(item).intersects(cmdq_state_flags::CMDQ_STATE_NOHOOKS)
        {
            return;
        }

        let c = payload
            .get_client("client")
            .and_then(|id| client_from_id(id))
            .unwrap_or(null_mut());
        let ft = format_create(c, item, FORMAT_NONE, format_flags::FORMAT_NOJOBS);
        for (key, value) in payload.formats("hook_") {
            format_add!(ft, &key, "{}", value);
        }
        format_add!(ft, "hook", "{}", name);
        format_log_debug(ft, c!("hooks_insert_event"));

        let mut hd = HookData {
            name,
            fs: zeroed(),
            formats: ft,
        };
        cmd_find_clear_state(&raw mut hd.fs, cmd_find_flags::empty());
        payload_get_target(payload, &raw mut hd.fs);

        hooks_insert(item, &mut hd);
        format_free(ft);
    }
}

/// The hooks sink: queues the hook for an event.
///
/// `item` is the command an after-* or command-error hook belongs to, the
/// hook then goes right after it. Otherwise the hook goes at the end of
/// the queue, unless the command running now was started by a hook.
///
/// # Safety
/// `item` must be null or valid, and the payload's entities valid or gone
/// from their registries.
pub unsafe fn hooks_event(name: &str, payload: &Payload, item: *mut cmdq_item) {
    unsafe {
        if !item.is_null() {
            hooks_insert_event(item, name, payload);
            return;
        }
        let running = cmdq_running(null_mut());
        if running.is_null()
            || !cmdq_get_flags(running).intersects(cmdq_state_flags::CMDQ_STATE_NOHOOKS)
        {
            hooks_insert_event(null_mut(), name, payload);
        }
    }
}

/// Registers the hooks sink for `name`, if it is not there yet.
pub fn hooks_add_event(name: &str) {
    if !events_has_sink(name, Sink::Hooks) {
        events_add_sink(name, Sink::Hooks);
    }
}

/// Whether the hooks sink is registered for `name`.
pub fn hooks_is_event(name: &str) -> bool {
    events_has_sink(name, Sink::Hooks)
}

/// Whether `name` can be fired as a hook event: a user event starting with
/// `@` or the name of a hook option.
pub fn hooks_valid_event_name(name: &str) -> bool {
    name.starts_with('@')
        || options_search(name).is_some_and(OptionTableEntry::is_hook)
}

/// Registers the hooks sink for every hook option, at server start.
pub fn hooks_build_events() {
    for oe in &OPTIONS_TABLE {
        if oe.flags & OPTIONS_TABLE_IS_HOOK != 0 {
            hooks_add_event(oe.name);
        }
    }
}

/// Runs a hook now, for set-hook -R: queues it after `item` with the
/// item's target.
///
/// # Safety
/// `item` must be a valid command queue item.
pub unsafe fn hooks_run(item: *mut cmdq_item, name: &str) {
    unsafe {
        let mut hd = HookData {
            name,
            fs: zeroed(),
            formats: format_create(null_mut(), null_mut(), 0, format_flags::FORMAT_NOJOBS),
        };
        cmd_find_copy_state(&raw mut hd.fs, cmdq_get_target(item));
        format_add!(hd.formats, "hook", "{}", name);
        format_log_debug(hd.formats, c!("hooks_run"));

        hooks_insert(item, &mut hd);
        format_free(hd.formats);
    }
}
