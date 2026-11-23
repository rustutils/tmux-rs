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
use crate::compat::{strlcat, strlcpy};
use crate::libc::{strchr, strlen, strncmp};
use crate::xmalloc::{xrealloc_, xreallocarray_};
use crate::*;
use crate::options_::*;

pub mod cmd_attach_session;
pub mod cmd_bind_key;
pub mod cmd_break_pane;
pub mod cmd_capture_pane;
pub mod cmd_choose_tree;
pub mod cmd_command_prompt;
pub mod cmd_confirm_before;
pub mod cmd_copy_mode;
pub mod cmd_detach_client;
pub mod cmd_display_menu;
pub mod cmd_display_message;
pub mod cmd_display_panes;
pub mod cmd_find;
pub mod cmd_find_window;
pub mod cmd_if_shell;
pub mod cmd_join_pane;
pub mod cmd_kill_pane;
pub mod cmd_kill_server;
pub mod cmd_kill_session;
pub mod cmd_kill_window;
pub mod cmd_list_buffers;
pub mod cmd_list_clients;
pub mod cmd_list_keys;
pub mod cmd_list_panes;
pub mod cmd_list_sessions;
pub mod cmd_list_windows;
pub mod cmd_load_buffer;
pub mod cmd_lock_server;
pub mod cmd_move_window;
pub mod cmd_new_session;
pub mod cmd_new_window;
pub mod cmd_paste_buffer;
pub mod cmd_pipe_pane;
pub mod cmd_queue;
pub mod cmd_refresh_client;
pub mod cmd_rename_session;
pub mod cmd_rename_window;
pub mod cmd_resize_pane;
pub mod cmd_resize_window;
pub mod cmd_respawn_pane;
pub mod cmd_respawn_window;
pub mod cmd_rotate_window;
pub mod cmd_run_shell;
pub mod cmd_save_buffer;
pub mod cmd_select_layout;
pub mod cmd_select_pane;
pub mod cmd_select_window;
pub mod cmd_send_keys;
pub mod cmd_server_access;
pub mod cmd_set_buffer;
pub mod cmd_set_environment;
pub mod cmd_set_option;
pub mod cmd_show_environment;
pub mod cmd_show_messages;
pub mod cmd_show_options;
pub mod cmd_show_prompt_history;
pub mod cmd_source_file;
pub mod cmd_split_window;
pub mod cmd_swap_pane;
pub mod cmd_swap_window;
pub mod cmd_switch_client;
pub mod cmd_unbind_key;
pub mod cmd_wait_for;

use cmd_attach_session::CMD_ATTACH_SESSION_ENTRY;
use cmd_bind_key::CMD_BIND_KEY_ENTRY;
use cmd_break_pane::CMD_BREAK_PANE_ENTRY;
use cmd_capture_pane::{CMD_CAPTURE_PANE_ENTRY, CMD_CLEAR_HISTORY_ENTRY};
use cmd_choose_tree::{
    CMD_CHOOSE_BUFFER_ENTRY, CMD_CHOOSE_CLIENT_ENTRY, CMD_CHOOSE_TREE_ENTRY,
    CMD_CUSTOMIZE_MODE_ENTRY,
};
use cmd_command_prompt::CMD_COMMAND_PROMPT_ENTRY;
use cmd_confirm_before::CMD_CONFIRM_BEFORE_ENTRY;
use cmd_copy_mode::{CMD_CLOCK_MODE_ENTRY, CMD_COPY_MODE_ENTRY};
use cmd_detach_client::CMD_DETACH_CLIENT_ENTRY;
use cmd_detach_client::CMD_SUSPEND_CLIENT_ENTRY;
use cmd_display_menu::{CMD_DISPLAY_MENU_ENTRY, CMD_DISPLAY_POPUP_ENTRY};
use cmd_display_message::CMD_DISPLAY_MESSAGE_ENTRY;
use cmd_display_panes::CMD_DISPLAY_PANES_ENTRY;
use cmd_find_window::CMD_FIND_WINDOW_ENTRY;
use cmd_if_shell::CMD_IF_SHELL_ENTRY;
use cmd_join_pane::{CMD_JOIN_PANE_ENTRY, CMD_MOVE_PANE_ENTRY};
use cmd_kill_pane::CMD_KILL_PANE_ENTRY;
use cmd_kill_server::CMD_KILL_SERVER_ENTRY;
use cmd_kill_server::CMD_START_SERVER_ENTRY;
use cmd_kill_session::CMD_KILL_SESSION_ENTRY;
use cmd_kill_window::CMD_KILL_WINDOW_ENTRY;
use cmd_kill_window::CMD_UNLINK_WINDOW_ENTRY;
use cmd_list_buffers::CMD_LIST_BUFFERS_ENTRY;
use cmd_list_clients::CMD_LIST_CLIENTS_ENTRY;
use cmd_list_keys::{CMD_LIST_COMMANDS_ENTRY, CMD_LIST_KEYS_ENTRY};
use cmd_list_panes::CMD_LIST_PANES_ENTRY;
use cmd_list_sessions::CMD_LIST_SESSIONS_ENTRY;
use cmd_list_windows::CMD_LIST_WINDOWS_ENTRY;
use cmd_load_buffer::CMD_LOAD_BUFFER_ENTRY;
use cmd_lock_server::{CMD_LOCK_CLIENT_ENTRY, CMD_LOCK_SERVER_ENTRY, CMD_LOCK_SESSION_ENTRY};
use cmd_move_window::CMD_LINK_WINDOW_ENTRY;
use cmd_move_window::CMD_MOVE_WINDOW_ENTRY;
use cmd_new_session::CMD_HAS_SESSION_ENTRY;
use cmd_new_session::CMD_NEW_SESSION_ENTRY;
use cmd_new_window::CMD_NEW_WINDOW_ENTRY;
use cmd_paste_buffer::CMD_PASTE_BUFFER_ENTRY;
use cmd_pipe_pane::CMD_PIPE_PANE_ENTRY;
use cmd_refresh_client::CMD_REFRESH_CLIENT_ENTRY;
use cmd_rename_session::CMD_RENAME_SESSION_ENTRY;
use cmd_rename_window::CMD_RENAME_WINDOW_ENTRY;
use cmd_resize_pane::CMD_RESIZE_PANE_ENTRY;
use cmd_resize_window::CMD_RESIZE_WINDOW_ENTRY;
use cmd_respawn_pane::CMD_RESPAWN_PANE_ENTRY;
use cmd_respawn_window::CMD_RESPAWN_WINDOW_ENTRY;
use cmd_rotate_window::CMD_ROTATE_WINDOW_ENTRY;
use cmd_run_shell::CMD_RUN_SHELL_ENTRY;
use cmd_save_buffer::CMD_SAVE_BUFFER_ENTRY;
use cmd_save_buffer::CMD_SHOW_BUFFER_ENTRY;
use cmd_select_layout::CMD_NEXT_LAYOUT_ENTRY;
use cmd_select_layout::CMD_PREVIOUS_LAYOUT_ENTRY;
use cmd_select_layout::CMD_SELECT_LAYOUT_ENTRY;
use cmd_select_pane::CMD_LAST_PANE_ENTRY;
use cmd_select_pane::CMD_SELECT_PANE_ENTRY;
use cmd_select_window::CMD_LAST_WINDOW_ENTRY;
use cmd_select_window::CMD_NEXT_WINDOW_ENTRY;
use cmd_select_window::CMD_PREVIOUS_WINDOW_ENTRY;
use cmd_select_window::CMD_SELECT_WINDOW_ENTRY;
use cmd_send_keys::CMD_SEND_KEYS_ENTRY;
use cmd_send_keys::CMD_SEND_PREFIX_ENTRY;
use cmd_server_access::CMD_SERVER_ACCESS_ENTRY;
use cmd_set_buffer::CMD_DELETE_BUFFER_ENTRY;
use cmd_set_buffer::CMD_SET_BUFFER_ENTRY;
use cmd_set_environment::CMD_SET_ENVIRONMENT_ENTRY;
use cmd_set_option::CMD_SET_HOOK_ENTRY;
use cmd_set_option::CMD_SET_OPTION_ENTRY;
use cmd_set_option::CMD_SET_WINDOW_OPTION_ENTRY;
use cmd_show_environment::CMD_SHOW_ENVIRONMENT_ENTRY;
use cmd_show_messages::CMD_SHOW_MESSAGES_ENTRY;
use cmd_show_options::CMD_SHOW_HOOKS_ENTRY;
use cmd_show_options::CMD_SHOW_OPTIONS_ENTRY;
use cmd_show_options::CMD_SHOW_WINDOW_OPTIONS_ENTRY;
use cmd_show_prompt_history::{CMD_CLEAR_PROMPT_HISTORY_ENTRY, CMD_SHOW_PROMPT_HISTORY_ENTRY};
use cmd_source_file::CMD_SOURCE_FILE_ENTRY;
use cmd_split_window::CMD_SPLIT_WINDOW_ENTRY;
use cmd_swap_pane::CMD_SWAP_PANE_ENTRY;
use cmd_swap_window::CMD_SWAP_WINDOW_ENTRY;
use cmd_switch_client::CMD_SWITCH_CLIENT_ENTRY;
use cmd_unbind_key::CMD_UNBIND_KEY_ENTRY;
use cmd_wait_for::CMD_WAIT_FOR_ENTRY;

pub static CMD_TABLE: [&cmd_entry; 90] = [
    &CMD_ATTACH_SESSION_ENTRY,
    &CMD_BIND_KEY_ENTRY,
    &CMD_BREAK_PANE_ENTRY,
    &CMD_CAPTURE_PANE_ENTRY,
    &CMD_CHOOSE_BUFFER_ENTRY,
    &CMD_CHOOSE_CLIENT_ENTRY,
    &CMD_CHOOSE_TREE_ENTRY,
    &CMD_CLEAR_HISTORY_ENTRY,
    &CMD_CLEAR_PROMPT_HISTORY_ENTRY,
    &CMD_CLOCK_MODE_ENTRY,
    &CMD_COMMAND_PROMPT_ENTRY,
    &CMD_CONFIRM_BEFORE_ENTRY,
    &CMD_COPY_MODE_ENTRY,
    &CMD_CUSTOMIZE_MODE_ENTRY,
    &CMD_DELETE_BUFFER_ENTRY,
    &CMD_DETACH_CLIENT_ENTRY,
    &CMD_DISPLAY_MENU_ENTRY,
    &CMD_DISPLAY_MESSAGE_ENTRY,
    &CMD_DISPLAY_POPUP_ENTRY,
    &CMD_DISPLAY_PANES_ENTRY,
    &CMD_FIND_WINDOW_ENTRY,
    &CMD_HAS_SESSION_ENTRY,
    &CMD_IF_SHELL_ENTRY,
    &CMD_JOIN_PANE_ENTRY,
    &CMD_KILL_PANE_ENTRY,
    &CMD_KILL_SERVER_ENTRY,
    &CMD_KILL_SESSION_ENTRY,
    &CMD_KILL_WINDOW_ENTRY,
    &CMD_LAST_PANE_ENTRY,
    &CMD_LAST_WINDOW_ENTRY,
    &CMD_LINK_WINDOW_ENTRY,
    &CMD_LIST_BUFFERS_ENTRY,
    &CMD_LIST_CLIENTS_ENTRY,
    &CMD_LIST_COMMANDS_ENTRY,
    &CMD_LIST_KEYS_ENTRY,
    &CMD_LIST_PANES_ENTRY,
    &CMD_LIST_SESSIONS_ENTRY,
    &CMD_LIST_WINDOWS_ENTRY,
    &CMD_LOAD_BUFFER_ENTRY,
    &CMD_LOCK_CLIENT_ENTRY,
    &CMD_LOCK_SERVER_ENTRY,
    &CMD_LOCK_SESSION_ENTRY,
    &CMD_MOVE_PANE_ENTRY,
    &CMD_MOVE_WINDOW_ENTRY,
    &CMD_NEW_SESSION_ENTRY,
    &CMD_NEW_WINDOW_ENTRY,
    &CMD_NEXT_LAYOUT_ENTRY,
    &CMD_NEXT_WINDOW_ENTRY,
    &CMD_PASTE_BUFFER_ENTRY,
    &CMD_PIPE_PANE_ENTRY,
    &CMD_PREVIOUS_LAYOUT_ENTRY,
    &CMD_PREVIOUS_WINDOW_ENTRY,
    &CMD_REFRESH_CLIENT_ENTRY,
    &CMD_RENAME_SESSION_ENTRY,
    &CMD_RENAME_WINDOW_ENTRY,
    &CMD_RESIZE_PANE_ENTRY,
    &CMD_RESIZE_WINDOW_ENTRY,
    &CMD_RESPAWN_PANE_ENTRY,
    &CMD_RESPAWN_WINDOW_ENTRY,
    &CMD_ROTATE_WINDOW_ENTRY,
    &CMD_RUN_SHELL_ENTRY,
    &CMD_SAVE_BUFFER_ENTRY,
    &CMD_SELECT_LAYOUT_ENTRY,
    &CMD_SELECT_PANE_ENTRY,
    &CMD_SELECT_WINDOW_ENTRY,
    &CMD_SEND_KEYS_ENTRY,
    &CMD_SEND_PREFIX_ENTRY,
    &CMD_SERVER_ACCESS_ENTRY,
    &CMD_SET_BUFFER_ENTRY,
    &CMD_SET_ENVIRONMENT_ENTRY,
    &CMD_SET_HOOK_ENTRY,
    &CMD_SET_OPTION_ENTRY,
    &CMD_SET_WINDOW_OPTION_ENTRY,
    &CMD_SHOW_BUFFER_ENTRY,
    &CMD_SHOW_ENVIRONMENT_ENTRY,
    &CMD_SHOW_HOOKS_ENTRY,
    &CMD_SHOW_MESSAGES_ENTRY,
    &CMD_SHOW_OPTIONS_ENTRY,
    &CMD_SHOW_PROMPT_HISTORY_ENTRY,
    &CMD_SHOW_WINDOW_OPTIONS_ENTRY,
    &CMD_SOURCE_FILE_ENTRY,
    &CMD_SPLIT_WINDOW_ENTRY,
    &CMD_START_SERVER_ENTRY,
    &CMD_SUSPEND_CLIENT_ENTRY,
    &CMD_SWAP_PANE_ENTRY,
    &CMD_SWAP_WINDOW_ENTRY,
    &CMD_SWITCH_CLIENT_ENTRY,
    &CMD_UNBIND_KEY_ENTRY,
    &CMD_UNLINK_WINDOW_ENTRY,
    &CMD_WAIT_FOR_ENTRY,
];

// Instance of a command.
pub struct cmd {
    pub entry: &'static cmd_entry,
    pub args: *mut args,
    pub group: u32,
    pub file: *mut u8,
    pub line: u32,
}

/// Next group number for new command list.
static CMD_LIST_NEXT_GROUP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

macro_rules! cmd_log_argv {
   ($argc:expr, $argv:expr, $fmt:literal $(, $args:expr)* $(,)?) => {
        crate::cmd_::cmd_log_argv_($argc, $argv, format_args!($fmt $(, $args)*))
    };
}
pub(crate) use cmd_log_argv;

// Log an argument vector.
pub unsafe fn cmd_log_argv_(argc: i32, argv: *mut *mut u8, args: std::fmt::Arguments) {
    unsafe {
        let prefix = args.to_string();
        for i in 0..argc {
            log_debug!("{}: argv[{}]{}", prefix, i, _s(*argv.add(i as usize)));
        }
    }
}

pub unsafe fn cmd_append_argv(argc: *mut c_int, argv: *mut *mut *mut u8, arg: *const u8) {
    unsafe {
        *argv = xreallocarray_::<*mut u8>(*argv, (*argc) as usize + 1).as_ptr();
        *(*argv).add((*argc) as usize) = xstrdup(arg).as_ptr();
        *argc += 1;
    }
}

pub unsafe fn cmd_pack_argv(
    argc: c_int,
    argv: *mut *mut u8,
    mut buf: *mut u8,
    mut len: usize,
) -> c_int {
    unsafe {
        //
        if argc == 0 {
            return 0;
        }
        cmd_log_argv!(argc, argv, "cmd_pack_argv");

        *buf = b'\0';
        for i in 0..argc {
            if strlcpy(buf, *argv.add(i as usize), len) >= len {
                return -1;
            }
            let arglen = strlen(*argv.add(i as usize)) + 1;
            buf = buf.add(arglen);
            len -= arglen;
        }

        0
    }
}

pub unsafe fn cmd_unpack_argv(
    mut buf: *mut u8,
    mut len: usize,
    argc: c_int,
    argv: *mut *mut *mut u8,
) -> c_int {
    unsafe {
        if argc == 0 {
            return 0;
        }
        // The count comes from the client, so limit it like tmux 3.8. An
        // empty buffer cannot hold any arguments (C tmux 3.8 would write
        // before the buffer here).
        if !(0..=1000).contains(&argc) || len == 0 {
            return -1;
        }
        *argv = xcalloc_::<*mut u8>(argc as usize).as_ptr();

        *buf.add(len - 1) = b'\0';
        for i in 0..argc {
            if len == 0 {
                cmd_free_argv(argc, *argv);
                return -1;
            }

            let arglen = strlen(buf) + 1;
            *(*argv).add(i as usize) = xstrdup(buf).as_ptr();

            buf = buf.add(arglen);
            len -= arglen;
        }
        cmd_log_argv!(argc, *argv, "cmd_unpack_argv");

        0
    }
}

pub unsafe fn cmd_copy_argv(argc: c_int, argv: *const *mut u8) -> *mut *mut u8 {
    unsafe {
        if argc == 0 {
            return null_mut();
        }
        let new_argv: *mut *mut u8 = xcalloc(argc as usize + 1, size_of::<*mut u8>())
            .cast()
            .as_ptr();
        for i in 0..argc {
            if !(*argv.add(i as usize)).is_null() {
                *new_argv.add(i as usize) = xstrdup(*argv.add(i as usize)).as_ptr();
            }
        }
        new_argv
    }
}

pub unsafe fn cmd_free_argv(argc: c_int, argv: *mut *mut u8) {
    unsafe {
        if argc == 0 {
            return;
        }
        for i in 0..argc {
            free(*argv.add(i as usize) as _);
        }
        free(argv as _);
    }
}

pub unsafe fn cmd_stringify_argv(argc: c_int, argv: *mut *mut u8) -> String {
    unsafe {
        if argc == 0 {
            return String::new();
        }

        let mut buf = String::new();
        for i in 0..argc {
            let s = args_escape(*argv.add(i as usize));
            log_debug!(
                "{}: {} {} = {}",
                "cmd_stringify_argv",
                i,
                _s(*argv.add(i as usize)),
                _s(s)
            );

            if i != 0 {
                buf.push(' ');
            }
            buf.push_str(cstr_to_str(s));

            free(s as _);
        }
        buf
    }
}

pub unsafe fn cmd_get_entry(cmd: *const cmd) -> &'static cmd_entry {
    unsafe { (*cmd).entry }
}

pub unsafe fn cmd_get_args(cmd: *mut cmd) -> *mut args {
    unsafe { (*cmd).args }
}

pub unsafe fn cmd_get_group(cmd: *const cmd) -> c_uint {
    unsafe { (*cmd).group }
}

pub unsafe fn cmd_get_source(cmd: *mut cmd, file: *mut *const u8, line: &AtomicU32) {
    unsafe {
        if !file.is_null() {
            *file = (*cmd).file;
        }
        line.store((*cmd).line, std::sync::atomic::Ordering::SeqCst);
    }
}

pub unsafe fn cmd_get_alias(name: *const u8) -> *mut u8 {
    unsafe {
        let o = options_get_only(GLOBAL_OPTIONS, "command-alias");
        if o.is_null() {
            return null_mut();
        }
        let wanted = strlen(name);

        for a in options_array_items(o) {
            let ov = options_array_item_value(a);

            let equals = strchr((*ov).string, b'=' as i32);
            if !equals.is_null() {
                let n = equals.addr() - (*ov).string.addr();
                if n == wanted && strncmp(name, (*ov).string, n) == 0 {
                    return xstrdup(equals.add(1)).as_ptr();
                }
            }
        }
        null_mut()
    }
}

pub fn cmd_find(name: &str) -> Result<&'static cmd_entry, String> {
    let mut found = None;
    let mut ambiguous: bool = false;

    for entry in CMD_TABLE {
        if entry.alias.is_some_and(|alias| alias == name) {
            ambiguous = false;
            found = Some(entry);
            break;
        }

        if entry.name.starts_with(name) {
            if found.is_some() {
                ambiguous = true;
            }
            found = Some(entry);

            if entry.name == name {
                break;
            }
        }
    }

    if !ambiguous {
        match found {
            Some(value) => {
                log_debug!("cmd_find: {name} found");
                Ok(value)
            }
            None => Err(format!("unknown command: {name}")),
        }
    } else {
        let mut msg = format!("ambiguous command: {name}, could be: ");

        // TODO, once https://github.com/rust-lang/rust/issues/79524 is stabilized rewrite
        for entry in CMD_TABLE {
            if entry.name.starts_with(name) {
                msg.push_str(entry.name);
                msg.push_str(", ");
            }
        }

        // remove last ", "
        msg.truncate(msg.len() - 2);

        Err(msg)
    }
}

pub unsafe fn cmd_parse(
    values: &[args_value],
    file: Option<&str>,
    line: c_uint,
) -> Result<*mut cmd, String> {
    unsafe {
        if values.is_empty() || !matches!(&values[0], args_value::String { .. }) {
            return Err("no command".to_string());
        }
        let args_value::String { string } = &values[0] else { unreachable!() };
        let entry = cmd_find(cstr_to_str(string.as_ptr().cast()))?;

        let args = match args_parse(&entry.args, values) {
            Ok(args) => args,
            Err(None) => {
                return Err(format!("usage: {} {}", entry.name, entry.usage));
            }
            Err(Some(error)) => {
                return Err(format!("command {}: {}", entry.name, error));
            }
        };

        let cmd: *mut cmd = Box::leak(Box::new(cmd {
            entry,
            args,
            group: 0,
            file: null_mut(),
            line: 0,
        }));

        if let Some(file) = file {
            let mut file = file.to_string();
            file.push('\0');
            (*cmd).file = file.leak().as_mut_ptr().cast();
        }
        (*cmd).line = line;

        Ok(cmd)
    }
}

pub unsafe fn cmd_free(cmd: *mut cmd) {
    unsafe {
        free((*cmd).file as _);

        args_free((*cmd).args);
        free(cmd as _);
    }
}

pub unsafe fn cmd_copy(cmd: *mut cmd, argc: c_int, argv: *mut *mut u8) -> *mut cmd {
    unsafe {
        let new_cmd: *mut cmd = Box::leak(Box::new(cmd {
            entry: (*cmd).entry,
            args: args_copy((*cmd).args, argc, argv),
            group: 0,
            file: null_mut(),
            line: 0,
        }));

        if !(*cmd).file.is_null() {
            (*new_cmd).file = xstrdup((*cmd).file).as_ptr();
        }
        (*new_cmd).line = (*cmd).line;

        new_cmd
    }
}

pub unsafe fn cmd_print(cmd: *mut cmd) -> *mut u8 {
    unsafe {
        let s = args_print((*cmd).args);
        let out = if *s != b'\0' {
            format_nul!("{} {}", (*cmd).entry.name, _s(s))
        } else {
            xstrdup__((*cmd).entry.name)
        };
        free(s as _);

        out
    }
}

pub fn cmd_list_new<'a>() -> &'a mut cmd_list {
    let group = CMD_LIST_NEXT_GROUP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    Box::leak(Box::new(cmd_list {
        references: 1,
        group,
        list: Vec::new(),
    }))
}

pub unsafe fn cmd_list_append(cmdlist: *mut cmd_list, cmd: *mut cmd) {
    unsafe {
        (*cmd).group = (*cmdlist).group;
        (*cmdlist).list.push(cmd);
    }
}

pub unsafe fn cmd_list_append_all(cmdlist: *mut cmd_list, from: *mut cmd_list) {
    unsafe {
        for &cmd in &(*from).list {
            (*cmd).group = (*cmdlist).group;
        }
        (*cmdlist).list.append(&mut (*from).list);
    }
}

pub unsafe fn cmd_list_move(cmdlist: *mut cmd_list, from: *mut cmd_list) {
    unsafe {
        (*cmdlist).list.append(&mut (*from).list);
        (*cmdlist).group = CMD_LIST_NEXT_GROUP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

pub unsafe fn cmd_list_free(cmdlist: *mut cmd_list) {
    unsafe {
        (*cmdlist).references -= 1;
        if (*cmdlist).references != 0 {
            return;
        }

        for &cmd in &(*cmdlist).list {
            cmd_free(cmd);
        }
        std::ptr::drop_in_place(&raw mut (*cmdlist).list);
        free_(cmdlist);
    }
}

pub unsafe fn cmd_list_copy(
    cmdlist: &cmd_list,
    argc: c_int,
    argv: *mut *mut u8,
) -> *mut cmd_list {
    unsafe {
        let mut group: u32 = cmdlist.group;
        let s = cmd_list_print(cmdlist, 0);
        log_debug!("{}: {}", "cmd_list_copy", _s(s));
        free(s as _);

        let new_cmdlist = cmd_list_new();
        for &cmd in &cmdlist.list {
            if (*cmd).group != group {
                new_cmdlist.group =
                    CMD_LIST_NEXT_GROUP.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                group = (*cmd).group;
            }
            let new_cmd = cmd_copy(cmd, argc, argv);
            cmd_list_append(new_cmdlist, new_cmd);
        }

        let s = cmd_list_print(new_cmdlist, 0);
        log_debug!("{}: {}", "cmd_list_copy", _s(s));
        free(s as _);

        new_cmdlist
    }
}

pub unsafe fn cmd_list_print(cmdlist: &cmd_list, escaped: c_int) -> *mut u8 {
    unsafe {
        let mut len = 1;
        let mut buf: *mut u8 = xcalloc(1, len).cast().as_ptr();

        let single_separator = if escaped != 0 { c!(" \\; ") } else { c!(" ; ") };
        let double_separator = if escaped != 0 {
            c!(" \\;\\; ")
        } else {
            c!(" ;; ")
        };

        for (idx, &cmd) in cmdlist.list.iter().enumerate() {
            let this = cmd_print(cmd);

            len += strlen(this) + 6;
            buf = xrealloc_(buf, len).as_ptr();

            strlcat(buf, this, len);

            if let Some(&next) = cmdlist.list.get(idx + 1) {
                let separator = if (*cmd).group != (*next).group {
                    double_separator
                } else {
                    single_separator
                };
                strlcat(buf, separator, len);
            }

            free_(this);
        }

        buf
    }
}

/// Get the commands in the list as a slice.
pub unsafe fn cmd_list_commands(cmdlist: *mut cmd_list) -> &'static [*mut cmd] {
    unsafe { &(*cmdlist).list }
}

pub unsafe fn cmd_list_all_have(cmdlist: *mut cmd_list, flag: cmd_flag) -> bool {
    unsafe {
        (*cmdlist).list.iter().all(|&cmd| (*cmd).entry.flags.intersects(flag))
    }
}

pub unsafe fn cmd_list_any_have(cmdlist: *mut cmd_list, flag: cmd_flag) -> bool {
    unsafe {
        (*cmdlist).list.iter().any(|&cmd| (*cmd).entry.flags.intersects(flag))
    }
}

pub unsafe fn cmd_mouse_at(
    wp: *mut window_pane,
    m: &mouse_event,
    xp: *mut c_uint,
    yp: *mut c_uint,
    last: c_int,
) -> c_int {
    unsafe {
        let x: u32;
        let mut y: u32;

        if last != 0 {
            x = m.lx + m.ox;
            y = m.ly + m.oy;
        } else {
            x = m.x + m.ox;
            y = m.y + m.oy;
        }
        log_debug!(
            "{}: x={}, y={}{}",
            "cmd_mouse_at",
            x,
            y,
            if last != 0 { " (last)" } else { "" }
        );

        if m.statusat == 0 && y >= m.statuslines {
            y -= m.statuslines;
        }

        if x < (*wp).xoff || x >= (*wp).xoff + (*wp).sx {
            return -1;
        }

        if y < (*wp).yoff || y >= (*wp).yoff + (*wp).sy {
            return -1;
        }

        if !xp.is_null() {
            *xp = x - (*wp).xoff;
        }
        if !yp.is_null() {
            *yp = y - (*wp).yoff;
        }
        0
    }
}

pub unsafe fn cmd_mouse_window(
    m: &mouse_event,
    sp: *mut *mut session,
) -> Option<NonNull<winlink>> {
    unsafe {
        let s: *mut session;

        if !m.valid {
            return None;
        }
        if m.s == -1
            || ({
                s = transmute_ptr(session_find_by_id(m.s as u32));
                s.is_null()
            })
        {
            return None;
        }
        let wl = if m.w == -1 {
            NonNull::new((*s).curw)
        } else {
            let w = window_find_by_id(m.w as u32);
            if w.is_null() {
                return None;
            }
            winlink_find_by_window(&raw mut (*s).windows, w)
        };
        if !sp.is_null() {
            *sp = s;
        }
        wl
    }
}

pub unsafe fn cmd_mouse_pane(
    m: &mouse_event,
    sp: *mut *mut session,
    wlp: *mut *mut winlink,
) -> Option<NonNull<window_pane>> {
    unsafe {
        let wl = cmd_mouse_window(m, sp)?;
        let wp;

        if m.wp == -1 {
            wp = NonNull::new(window_active_pane(winlink_window(wl.as_ptr())));
        } else {
            wp = Some(NonNull::new(window_pane_find_by_id(m.wp as u32))?);
            if !window_has_pane(&*winlink_window(wl.as_ptr()), wp.unwrap().as_ptr()) {
                return None;
            }
        }

        if !wlp.is_null() {
            *wlp = wl.as_ptr();
        }
        wp
    }
}

/// Replace the first `%%` and every `%idx` in `template` by `s`, returning
/// a new string allocated with xmalloc. See `template_replace`.
pub unsafe fn cmd_template_replace(template: *const u8, s: Option<&str>, idx: c_int) -> *mut u8 {
    unsafe {
        let template = std::ffi::CStr::from_ptr(template.cast()).to_bytes();
        let mut out = template_replace(template, s.unwrap_or("").as_bytes(), idx);
        // The result goes back to C-style callers, it cannot hold a NUL.
        out.retain(|&b| b != 0);
        let out = std::ffi::CString::new(out).unwrap();
        xstrdup(out.as_ptr().cast()).as_ptr()
    }
}

/// Replace the first `%%` and every `%idx` (a digit 1 to 9 equal to
/// `idx`) in `template` by `s`. Like tmux 3.8, the form decides the
/// quoting. `%%` is meant for single quotes, so each `'` in `s` becomes
/// `'\''`. `%%%` and `%idx%` are meant for double quotes, so the characters
/// `"\$;~` get a backslash. Plain `%idx` is not escaped. Escaping `'` stops
/// a session or window name from ending a quoted section and injecting
/// commands (upstream a177d0f5).
pub fn template_replace(template: &[u8], s: &[u8], idx: c_int) -> Vec<u8> {
    #[derive(PartialEq)]
    enum Quote {
        None,
        Single,
        Double,
    }

    let mut out = Vec::with_capacity(template.len());
    let mut replaced = false;
    let mut i = 0;
    while i < template.len() {
        let ch = template[i];
        i += 1;
        if ch != b'%' {
            out.push(ch);
            continue;
        }
        let mut quote = match template.get(i).copied() {
            Some(d @ b'1'..=b'9') if c_int::from(d - b'0') == idx => {
                i += 1;
                Quote::None
            }
            Some(b'%') if !replaced => {
                replaced = true;
                i += 1;
                Quote::Single
            }
            _ => {
                out.push(ch);
                continue;
            }
        };
        if template.get(i) == Some(&b'%') {
            quote = Quote::Double;
            i += 1;
        }
        for &c in s {
            if quote == Quote::Single && c == b'\'' {
                out.extend_from_slice(br"'\''");
                continue;
            }
            if quote == Quote::Double && br#""\$;~"#.contains(&c) {
                out.push(b'\\');
            }
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod test {
    use super::*;

    fn tr(template: &str, s: &str, idx: i32) -> String {
        String::from_utf8(template_replace(template.as_bytes(), s.as_bytes(), idx)).unwrap()
    }

    #[test]
    fn template_single_quote_escapes_quote() {
        assert_eq!(
            tr("switch-client -t '%%'", "a';x;'", 1),
            r"switch-client -t 'a'\'';x;'\'''"
        );
        assert_eq!(tr("'%%'", "plain", 1), "'plain'");
    }

    #[test]
    fn template_double_quote_escapes_specials() {
        assert_eq!(tr(r#""%%%""#, r#"a"b$c;d~e\f'g"#, 1), r#""a\"b\$c\;d\~e\\f'g""#);
        assert_eq!(tr(r#""%1%""#, "$x", 1), r#""\$x""#);
    }

    #[test]
    fn template_index_is_unescaped_and_replaced_everywhere() {
        assert_eq!(tr("%1 %1 %2", "a'b", 1), "a'b a'b %2");
        assert_eq!(tr("%2", "x", 2), "x");
    }

    #[test]
    fn template_only_first_percent_percent() {
        assert_eq!(tr("%% %%", "x", 1), "x %%");
        assert_eq!(tr("100%", "x", 1), "100%");
        assert_eq!(tr("no percent", "x", 1), "no percent");
    }

    #[test]
    fn template_keeps_utf8() {
        assert_eq!(tr("'%%'", "s\u{e9}ance \u{1f600}", 1), "'s\u{e9}ance \u{1f600}'");
    }

    // <https://github.com/richardscollin/tmux-rs/issues/50>
    #[test]
    fn test_template_replace() {
        unsafe {
            let out = cmd_template_replace(c"%1".as_ptr().cast(), Some("resize-pane -D 3"), 1);

            let m = libc::strlen(b"resize-pane -D 3\0junk".as_ptr().cast());

            // note the real test is that the return value is properly nul terminated
            let n = libc::strlen(out);

            free_(out);

            assert_eq!(n, m);
        }
    }
}
