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
use crate::hook_monitor::{
    hooks_monitor_fired, hooks_monitor_get, hooks_monitor_to_string, monitor_target_string,
};
use crate::options_::*;

pub static CMD_SHOW_OPTIONS_ENTRY: cmd_entry = cmd_entry {
    name: "show-options",
    alias: Some("show"),

    args: args_parse::new("AgF:Hpqst:vw", 0, 1, None),
    usage: "[-AgHpqsvw] [-F format] [-t target-pane] [option]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_PANE,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Old(cmd_show_options_exec),
    source: cmd_entry_flag::zeroed(),
};

pub static CMD_SHOW_WINDOW_OPTIONS_ENTRY: cmd_entry = cmd_entry {
    name: "show-window-options",
    alias: Some("showw"),

    args: args_parse::new("F:gvt:", 0, 1, None),
    usage: "[-gv] [-F format] [-t target-window] [option]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_WINDOW,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Old(cmd_show_options_exec),

    source: cmd_entry_flag::zeroed(),
};

pub static CMD_SHOW_HOOKS_ENTRY: cmd_entry = cmd_entry {
    name: "show-hooks",
    alias: None,

    args: args_parse::new("BF:gpt:w", 0, 1, None),
    usage: "[-Bgpw] [-F format] [-t target-pane] [hook]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_PANE,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Old(cmd_show_options_exec),

    source: cmd_entry_flag::zeroed(),
};

/// How show-options prints an option without -F: the value only with
/// -v, otherwise the name, `[key]`, `*` if inherited and the value,
/// quoted for strings.
/// How show-hooks -B prints a monitor, as set-hook -B takes it.
const SHOW_HOOKS_MONITOR_TEMPLATE: &CStr =
    c"#{option_name}:#{hook_monitor_target}:#{hook_monitor_format}";

const SHOW_OPTIONS_TEMPLATE: &CStr = c"#{?option_value_only,#{option_value},#{option_name}#{?option_has_array_key,[#{option_array_key}],}#{?option_is_parent,*,}#{?option_has_value, #{?option_is_string,#{q/a:option_value},#{option_value}},}}";

/// show-options, show-window-options and show-hooks.
unsafe fn cmd_show_options_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let target = cmdq_get_target(item);
        let mut oo: *mut options = null_mut();
        let quiet = args_has(args, 'q');

        let window = std::ptr::eq(cmd_get_entry(self_), &CMD_SHOW_WINDOW_OPTIONS_ENTRY) as i32;
        let show_monitors =
            std::ptr::eq(cmd_get_entry(self_), &CMD_SHOW_HOOKS_ENTRY) && args_has(args, 'B');

        if args_count(args) == 0 {
            let scope = match options_scope_from_flags(args, window, target, &raw mut oo) {
                Ok(s) => s,
                Err(_) if quiet => return cmd_retval::CMD_RETURN_NORMAL,
                Err(cause) => {
                    cmdq_error!(item, "{}", cause);
                    return cmd_retval::CMD_RETURN_ERROR;
                }
            };
            if show_monitors {
                for o in options_entries(oo) {
                    cmd_show_hooks_print_monitor(self_, item, o);
                }
                return cmd_retval::CMD_RETURN_NORMAL;
            }
            return cmd_show_options_all(self_, item, scope, oo);
        }
        let expanded = format_single_from_target(item, args_string(args, 0));
        let argument = cstr_to_string_lossy(expanded);
        free_(expanded);

        let (name, key) = match options_match(&argument) {
            Ok(found) => found,
            Err(_) if quiet => return cmd_retval::CMD_RETURN_NORMAL,
            Err(e) => {
                match e {
                    MatchError::Ambiguous => cmdq_error!(item, "ambiguous option: {}", argument),
                    MatchError::Invalid => cmdq_error!(item, "invalid option: {}", argument),
                }
                return cmd_retval::CMD_RETURN_ERROR;
            }
        };
        if let Err(cause) = options_scope_from_name(args, window, &name, target, &raw mut oo) {
            if quiet {
                return cmd_retval::CMD_RETURN_NORMAL;
            }
            cmdq_error!(item, "{}", cause);
            return cmd_retval::CMD_RETURN_ERROR;
        }
        let mut o = options_get_only(oo, &name);
        let mut parent = false;
        if args_has(args, 'A') && o.is_null() {
            o = options_get(&mut *oo, &name);
            parent = true;
        }
        if !o.is_null() && show_monitors {
            cmd_show_hooks_print_monitor(self_, item, o);
        } else if !o.is_null() {
            // An empty array has nothing inherited to show.
            if key.is_none() && options_is_array(o) && options_array_keys(o).is_empty() {
                parent = false;
            }
            cmd_show_options_print(self_, item, o, key.as_ref(), parent);
        } else if name.starts_with('@') {
            if quiet {
                return cmd_retval::CMD_RETURN_NORMAL;
            }
            cmdq_error!(item, "invalid option: {}", argument);
            return cmd_retval::CMD_RETURN_ERROR;
        }
        cmd_retval::CMD_RETURN_NORMAL
    }
}

/// Prints one option, or each item of an array, with -F or the default
/// template. `parent` marks an inherited option.
unsafe fn cmd_show_options_print(
    self_: *mut cmd,
    item: *mut cmdq_item,
    o: *mut options_entry,
    key: Option<&ArrayKey>,
    parent: bool,
) {
    unsafe {
        let args = cmd_get_args(self_);
        let template = args_get_(args, 'F');
        let mut has_value = true;

        let value = match key {
            Some(key) => options_to_string(o, Some(key), false),
            None if options_is_array(o) => {
                let keys = options_array_keys(o);
                if !keys.is_empty() {
                    for key in &keys {
                        cmd_show_options_print(self_, item, o, Some(key), parent);
                    }
                    return;
                }
                if template.is_null() && args_has(args, 'v') {
                    return;
                }
                has_value = false;
                Vec::new()
            }
            None => options_to_string(o, None, false),
        };
        let template = if template.is_null() {
            SHOW_OPTIONS_TEMPLATE.as_ptr().cast()
        } else {
            template
        };

        let oe = options_table_entry(o);
        let is_hook = !oe.is_null() && (*oe).flags & OPTIONS_TABLE_IS_HOOK != 0;
        let is_user = oe.is_null();

        let ft = format_create_from_target(item);
        format_add!(ft, "option_name", "{}", options_name(o));
        format_add_bytes(ft, b"option_value", &value);
        format_add!(ft, "option_value_only", "{}", args_has(args, 'v') as i32);
        format_add!(ft, "option_is_parent", "{}", parent as i32);
        format_add!(ft, "option_is_array", "{}", options_is_array(o) as i32);
        format_add!(ft, "option_is_string", "{}", options_is_string(o) as i32);
        format_add!(ft, "option_is_hook", "{}", is_hook as i32);
        format_add!(ft, "option_is_user", "{}", is_user as i32);
        format_add!(ft, "option_has_value", "{}", has_value as i32);
        if std::ptr::eq(cmd_get_entry(self_), &CMD_SHOW_HOOKS_ENTRY) {
            format_add!(ft, "hook_fire_count", "{}", (*o).fire_count());
            let fire_time = (*o).fire_time();
            if fire_time != 0 {
                let tv = timeval {
                    tv_sec: fire_time,
                    tv_usec: 0,
                };
                format_add_tv(ft, c!("hook_fire_time"), &tv);
            }
        }
        match key {
            Some(key) => {
                format_add!(ft, "option_array_key", "{}", key);
                format_add!(ft, "option_has_array_key", "1");
            }
            None => {
                format_add!(ft, "option_array_key", "");
                format_add!(ft, "option_has_array_key", "0");
            }
        }
        let line = format_expand(ft, template);
        format_free(ft);

        cmdq_print_bytes(item, CStr::from_ptr(line.cast()).to_bytes());
        free_(line);
    }
}

/// Prints the monitor of a hook option (show-hooks -B), with -F or the
/// monitor template. Nothing for an option without a monitor.
unsafe fn cmd_show_hooks_print_monitor(self_: *mut cmd, item: *mut cmdq_item, o: *mut options_entry) {
    unsafe {
        let args = cmd_get_args(self_);
        let Some(value) = hooks_monitor_to_string(o) else {
            return;
        };
        let Some((target, format)) = hooks_monitor_get(o) else {
            return;
        };
        let template = args_get_(args, 'F');
        let template = if template.is_null() {
            SHOW_HOOKS_MONITOR_TEMPLATE.as_ptr().cast()
        } else {
            template
        };

        let ft = format_create_from_target(item);
        format_add!(ft, "option_name", "{}", options_name(o));
        format_add!(ft, "option_value", "{}", value);
        format_add!(ft, "option_value_only", "0");
        format_add!(ft, "option_is_parent", "0");
        format_add!(ft, "option_is_array", "0");
        format_add!(ft, "option_is_string", "1");
        format_add!(ft, "option_is_hook", "1");
        format_add!(ft, "option_is_user", "1");
        format_add!(ft, "option_has_value", "1");
        format_add!(ft, "option_array_key", "");
        format_add!(ft, "option_has_array_key", "0");
        format_add!(ft, "hook_monitor_target", "{}", monitor_target_string(target));
        format_add!(ft, "hook_monitor_format", "{}", format);
        let (fire_count, fire_time) = hooks_monitor_fired(o);
        format_add!(ft, "hook_fire_count", "{}", fire_count);
        if fire_time != 0 {
            let tv = timeval {
                tv_sec: fire_time,
                tv_usec: 0,
            };
            format_add_tv(ft, c!("hook_fire_time"), &tv);
        }
        let line = format_expand(ft, template);
        format_free(ft);

        cmdq_print_bytes(item, CStr::from_ptr(line.cast()).to_bytes());
        free_(line);
    }
}

/// Prints every option of a tree: user options first, then the table
/// options of the scope. Hooks only for show-hooks or with -H.
unsafe fn cmd_show_options_all(
    self_: *mut cmd,
    item: *mut cmdq_item,
    scope: i32,
    oo: *mut options,
) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let show_hooks = std::ptr::eq(cmd_get_entry(self_), &CMD_SHOW_HOOKS_ENTRY);

        // User options. Those set with set-hook are hooks: show-hooks
        // lists them and show-options only with -H.
        for o in options_entries(oo) {
            if !options_table_entry(o).is_null() {
                continue;
            }
            let name = options_name(o);
            let is_user_hook =
                name.starts_with('@') && (hooks_is_event(name) || (*o).monitor().is_some());
            if show_hooks {
                if is_user_hook {
                    cmd_show_options_print(self_, item, o, None, false);
                }
            } else if !is_user_hook || args_has(args, 'H') {
                cmd_show_options_print(self_, item, o, None, false);
            }
        }

        for oe in &OPTIONS_TABLE {
            if !oe.scope & scope != 0 {
                continue;
            }
            if (!show_hooks && !args_has(args, 'H') && oe.is_hook()) || (show_hooks && !oe.is_hook()) {
                continue;
            }

            let mut o = options_get_only(oo, oe.name);
            let mut parent = false;
            if o.is_null() {
                if !args_has(args, 'A') {
                    continue;
                }
                o = options_get(&mut *oo, oe.name);
                if o.is_null() {
                    continue;
                }
                parent = true;
            }
            cmd_show_options_print(self_, item, o, None, parent);
        }
    }
    cmd_retval::CMD_RETURN_NORMAL
}
