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
use crate::*;
use crate::hook_monitor::{HookTree, hooks_monitor_add, hooks_monitor_remove};
use crate::monitor::{MonitorFlags, monitor_parse};
use crate::options_::*;

pub static CMD_SET_OPTION_ENTRY: cmd_entry = cmd_entry {
    name: "set-option",
    alias: Some("set"),

    args: args_parse::new("aFgopqst:uUw", 1, 2, Some(cmd_set_option_args_parse)),
    usage: "[-aFgopqsuUw] [-t target-pane] option [value]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_PANE,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Old(cmd_set_option_exec),
    source: cmd_entry_flag::zeroed(),
};

pub static CMD_SET_WINDOW_OPTION_ENTRY: cmd_entry = cmd_entry {
    name: "set-window-option",
    alias: Some("setw"),

    args: args_parse::new("aFgoqt:u", 1, 2, Some(cmd_set_option_args_parse)),
    usage: "[-aFgoqu] [-t target-window] option [value]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_WINDOW,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Old(cmd_set_option_exec),
    source: cmd_entry_flag::zeroed(),
};

pub static CMD_SET_HOOK_ENTRY: cmd_entry = cmd_entry {
    name: "set-hook",
    alias: None,

    args: args_parse::new("agpERTt:uB:w", 0, 2, Some(cmd_set_option_args_parse)),
    usage: "[-agpERTuw] [-B name:what:format] [-t target-pane] [hook] [command]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_PANE,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Old(cmd_set_option_exec),
    source: cmd_entry_flag::zeroed(),
};

/// The value of set-option and the command of set-hook may be a command
/// block. With set-hook -B the only argument is the command.
pub unsafe fn cmd_set_option_args_parse(args: *mut args, idx: u32) -> args_parse_type {
    // SAFETY: args is the arguments being parsed, flags are done.
    if unsafe { args_has(args, 'B') } {
        return args_parse_type::ARGS_PARSE_COMMANDS_OR_STRING;
    }
    match idx {
        1 => args_parse_type::ARGS_PARSE_COMMANDS_OR_STRING,
        _ => args_parse_type::ARGS_PARSE_STRING,
    }
}

/// set-hook -B: adds a monitor to hook `@name`, or removes it with -u.
/// An argument sets the hook's command, as set-option would.
///
/// # Safety
/// `self_` and `item` must be valid.
unsafe fn cmd_set_hook_monitor_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let target = cmdq_get_target(item);
        let error = |message: String| {
            cmdq_error!(item, "{}", message);
            cmd_retval::CMD_RETURN_ERROR
        };
        if args_count(args) > 1 {
            return error("too many arguments".into());
        }

        let value = cstr_to_string_lossy(args_get_(args, 'B'));
        let parsed = monitor_parse(&value);
        let name = match (&parsed, args_has(args, 'u')) {
            (Some((name, _, _)), _) => name.clone(),
            // With -u a value that does not parse is a name.
            (None, true) => value.clone(),
            (None, false) => return error(format!("invalid subscription: {value}")),
        };
        if !name.starts_with('@') {
            return error("monitor hook name must start with @".into());
        }

        let mut oo: *mut options = null_mut();
        let scope = match options_scope_from_name(args, 0, &name, target, &raw mut oo) {
            Ok(scope) => scope,
            Err(cause) => return error(cause),
        };

        if args_has(args, 'u') {
            hooks_monitor_remove(oo, &name);
            return cmd_retval::CMD_RETURN_NORMAL;
        }
        let Some((_, what, format)) = parsed else {
            unreachable!("checked above");
        };

        if args_count(args) != 0 {
            let mut value = CStr::from_ptr(args_string(args, 0).cast()).to_bytes().to_vec();
            if args_has(args, 'F') {
                let expanded = format_single_from_target(item, args_string(args, 0));
                value = CStr::from_ptr(expanded.cast()).to_bytes().to_vec();
                free_(expanded);
            }
            let o = options_get_only(oo, &name);
            if !args_has(args, 'o') || o.is_null() {
                if args_has(args, 'a') && !o.is_null() {
                    let old = CStr::from_ptr(options_get_string(oo, &name).cast());
                    value = [old.to_bytes(), &value].concat();
                }
                options_set_string_bytes(oo, &name, &value, false);
                options_push_changes(&name);
            }
        }

        let global = oo == GLOBAL_OPTIONS || oo == GLOBAL_S_OPTIONS || oo == GLOBAL_W_OPTIONS;
        let tree = if oo == GLOBAL_OPTIONS {
            HookTree::Server
        } else if oo == GLOBAL_S_OPTIONS {
            HookTree::GlobalSession
        } else if oo == GLOBAL_W_OPTIONS {
            HookTree::GlobalWindow
        } else if scope == OPTIONS_TABLE_PANE {
            HookTree::Pane((*target).wp.expect("pane scope has a pane"))
        } else if scope == OPTIONS_TABLE_WINDOW {
            HookTree::Window(WindowId((*winlink_window((*target).wl)).id))
        } else {
            HookTree::Session((*target).s.expect("session scope has a session"))
        };
        let session = if global { None } else { (*target).s };
        let flags = MonitorFlags {
            initial: false,
            only_true: args_has(args, 'T'),
        };
        hooks_monitor_add(oo, tree, &name, what, &format, flags, session);
        cmd_retval::CMD_RETURN_NORMAL
    }
}

/// set-hook -E: fires a user event, whose name starts with `@`, with the
/// command's target as payload.
///
/// # Safety
/// `self_` and `item` must be valid.
unsafe fn cmd_set_hook_event_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        if args_count(args) == 0 {
            cmdq_error!(item, "missing argument");
            return cmd_retval::CMD_RETURN_ERROR;
        }
        if args_count(args) != 1 {
            cmdq_error!(item, "too many arguments");
            return cmd_retval::CMD_RETURN_ERROR;
        }
        let argument = format_single_from_target(item, args_string(args, 0));
        let name = cstr_to_string_lossy(argument);
        free_(argument);
        if !name.starts_with('@') {
            cmdq_error!(item, "event name must start with @");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        let target = cmdq_get_target(item);
        let mut payload = crate::events::Payload::new();
        payload.set_target(crate::events::payload_target(target));
        crate::events::payload_add_find_state(&mut payload, cmdq_get_client(item), target);
        crate::events::events_fire(&name, payload, null_mut());
        cmd_retval::CMD_RETURN_NORMAL
    }
}

/// set-option, set-window-option and set-hook.
pub unsafe fn cmd_set_option_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let hook = std::ptr::eq(cmd_get_entry(self_), &CMD_SET_HOOK_ENTRY);
        if hook && args_has(args, 'E') {
            return cmd_set_hook_event_exec(self_, item);
        }
        if hook && args_has(args, 'B') {
            return cmd_set_hook_monitor_exec(self_, item);
        }
        if args_count(args) == 0 {
            cmdq_error!(item, "missing argument");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        // Expand argument.
        let expanded = format_single_from_target(item, args_string(args, 0));
        let argument = cstr_to_string_lossy(expanded);
        free_(expanded);

        // If set-hook -R, fire the hook straight away.
        if hook && args_has(args, 'R') {
            hooks_run(item, &argument);
            return cmd_retval::CMD_RETURN_NORMAL;
        }

        match cmd_set_option_change(self_, item, &argument) {
            Ok(()) => cmd_retval::CMD_RETURN_NORMAL,
            Err(None) => cmd_retval::CMD_RETURN_NORMAL,
            Err(Some(cause)) => {
                cmdq_error!(item, "{}", cause);
                cmd_retval::CMD_RETURN_ERROR
            }
        }
    }
}

/// The value argument, expanded with -F, as bytes.
unsafe fn cmd_set_option_value(args: *mut args, item: *mut cmdq_item) -> Option<Vec<u8>> {
    unsafe {
        if args_count(args) < 2 {
            return None;
        }
        let value = args_string(args, 1);
        if !args_has(args, 'F') {
            return Some(CStr::from_ptr(value.cast()).to_bytes().to_vec());
        }
        let expanded = format_single_from_target(item, value);
        let bytes = CStr::from_ptr(expanded.cast()).to_bytes().to_vec();
        free_(expanded);
        Some(bytes)
    }
}

/// Changes option `argument` (`name` or `name[key]`). Errors are the
/// message to print, or None to stop quietly (-q, or -u on an option
/// that is not set). Options are pushed only after a change.
unsafe fn cmd_set_option_change(
    self_: *mut cmd,
    item: *mut cmdq_item,
    argument: &str,
) -> Result<(), Option<String>> {
    unsafe {
        let args = cmd_get_args(self_);
        let append = args_has(args, 'a');
        let quiet = args_has(args, 'q');
        let target = cmdq_get_target(item);
        let window = std::ptr::eq(cmd_get_entry(self_), &CMD_SET_WINDOW_OPTION_ENTRY) as i32;
        let quietly = |cause: String| if quiet { None } else { Some(cause) };

        // Parse option name and array key.
        let (name, key) = options_match(argument).map_err(|e| {
            quietly(match e {
                MatchError::Ambiguous => format!("ambiguous option: {argument}"),
                MatchError::Invalid => format!("invalid option: {argument}"),
            })
        })?;
        let value = cmd_set_option_value(args, item);

        // Get the scope and table for the option.
        let mut oo: *mut options = null_mut();
        let scope = options_scope_from_name(args, window, &name, target, &raw mut oo)
            .map_err(quietly)?;
        let mut o = options_get_only(oo, &name);
        let parent = options_get(&mut *oo, &name);

        // Check that array options and keys match up.
        if key.is_some() && (name.starts_with('@') || parent.is_null() || !options_is_array(parent))
        {
            return Err(Some(format!("not an array: {argument}")));
        }

        // With -o, check this option is not already set.
        if !args_has(args, 'u') && args_has(args, 'o') {
            let already = match &key {
                None => !o.is_null(),
                Some(key) => !o.is_null() && options_array_has(o, key),
            };
            if already {
                return Err(quietly(format!("already set: {argument}")));
            }
        }

        // Change the option.
        if args_has(args, 'U') && scope == OPTIONS_TABLE_WINDOW {
            let w = (*target).w.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
            let panes: Vec<*mut window_pane> = if w.is_null() { Vec::new() } else { (*w).panes.clone() };
            for loop_ in panes {
                let po = options_get_only((*loop_).options, &name);
                if po.is_null() {
                    continue;
                }
                options_remove_or_default(po, key.as_ref()).map_err(|e| Some(e.to_string()))?;
            }
        }
        if args_has(args, 'u') || args_has(args, 'U') {
            if o.is_null() {
                return Err(None);
            }
            options_remove_or_default(o, key.as_ref()).map_err(|e| Some(e.to_string()))?;
        } else if name.starts_with('@') {
            let Some(value) = value else {
                return Err(Some("empty value".into()));
            };
            options_set_string_bytes(oo, &name, &value, append);
            if std::ptr::eq(cmd_get_entry(self_), &CMD_SET_HOOK_ENTRY) {
                hooks_add_event(&name);
            }
        } else if key.is_none() && !options_is_array(parent) {
            let oe = options_table_entry(parent);
            options_from_string(oo, oe, (*oe).name, value.as_deref(), append)
                .map_err(|e| Some(e.to_string()))?;
        } else {
            let Some(value) = value else {
                return Err(Some("empty value".into()));
            };
            if o.is_null() {
                o = options_empty(oo, options_table_entry(parent));
            }
            match &key {
                None => {
                    if !append {
                        options_array_clear(o);
                    }
                    options_array_assign(o, &value).map_err(|e| Some(e.to_string()))?;
                }
                Some(key) => {
                    options_array_set(o, key, Some(&value), append)
                        .map_err(|e| Some(e.to_string()))?;
                }
            }
        }

        options_push_changes(&name);
        Ok(())
    }
}
