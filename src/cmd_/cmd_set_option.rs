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
    exec: cmd_set_option_exec,
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
    exec: cmd_set_option_exec,
    source: cmd_entry_flag::zeroed(),
};

pub static CMD_SET_HOOK_ENTRY: cmd_entry = cmd_entry {
    name: "set-hook",
    alias: None,

    args: args_parse::new("agpERt:uw", 1, 2, Some(cmd_set_option_args_parse)),
    usage: "[-agpERuw] [-t target-pane] hook [command]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_PANE,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: cmd_set_option_exec,
    source: cmd_entry_flag::zeroed(),
};

pub unsafe fn cmd_set_option_args_parse(_args: *mut args, idx: u32) -> args_parse_type {
    match idx {
        1 => args_parse_type::ARGS_PARSE_COMMANDS_OR_STRING,
        _ => args_parse_type::ARGS_PARSE_STRING,
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
