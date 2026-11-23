// Copyright (c) 2021 Dallas Lyons <dallasdlyons@gmail.com>
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

use std::ffi::CStr;

use crate::libc::{getgrnam, getpwnam, getuid};
use crate::*;

pub static CMD_SERVER_ACCESS_ENTRY: cmd_entry = cmd_entry {
    name: "server-access",
    alias: None,

    args: args_parse::new("adglrw", 0, 1, None),
    usage: "[-adglrw] [-t target-pane] [user|group]",

    flags: cmd_flag::CMD_CLIENT_CANFAIL,
    exec: cmd_server_access_exec,
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// Looks up a user, or with `group` a group, by name. Returns its ACL ID
/// and its name as the system spells it.
///
/// # Safety
/// Calls getpwnam and getgrnam, whose results are copied before the
/// next call.
unsafe fn cmd_server_access_lookup(name: &CStr, group: bool) -> Option<(AclId, String)> {
    unsafe {
        if group {
            let gr = getgrnam(name.as_ptr());
            if gr.is_null() {
                return None;
            }
            let found = CStr::from_ptr((*gr).gr_name).to_string_lossy().into_owned();
            Some((AclId::Group((*gr).gr_gid), found))
        } else {
            let pw = getpwnam(name.as_ptr());
            if pw.is_null() {
                return None;
            }
            let found = CStr::from_ptr((*pw).pw_name).to_string_lossy().into_owned();
            Some((AclId::User((*pw).pw_uid), found))
        }
    }
}

/// Adds, removes or changes the access of a user or group (-g), or lists
/// the access control list (-l).
unsafe fn cmd_server_access_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let c = cmdq_get_target_client(item);

        if args_has(args, 'l') {
            server_acl_display(item);
            return cmd_retval::CMD_RETURN_NORMAL;
        }
        if args_count(args) == 0 {
            cmdq_error!(item, "missing user or group argument");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        let arg = format_single(
            item,
            cstr_to_str(args_string(args, 0)),
            c,
            null_mut(),
            null_mut(),
            null_mut(),
        );
        let arg_name = CStr::from_ptr(arg.cast()).to_owned();
        free_(arg);

        let group = args_has(args, 'g');
        let type_ = if group { "group" } else { "user" };
        let found = if arg_name.is_empty() {
            None
        } else {
            cmd_server_access_lookup(&arg_name, group)
        };
        let Some((id, name)) = found else {
            cmdq_error!(item, "unknown {type_}: {}", arg_name.to_string_lossy());
            return cmd_retval::CMD_RETURN_ERROR;
        };

        if let AclId::User(uid) = id
            && (uid == 0 || uid == getuid())
        {
            cmdq_error!(item, "{name} owns the server, can't change access");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        if args_has(args, 'a') && args_has(args, 'd') {
            cmdq_error!(item, "-a and -d cannot be used together");
            return cmd_retval::CMD_RETURN_ERROR;
        }
        if args_has(args, 'w') && args_has(args, 'r') {
            cmdq_error!(item, "-r and -w cannot be used together");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        if args_has(args, 'd') {
            if !server_acl_find(id) {
                cmdq_error!(item, "{type_} {name} not found");
                return cmd_retval::CMD_RETURN_ERROR;
            }
            server_acl_deny(id);
            return cmd_retval::CMD_RETURN_NORMAL;
        }
        if args_has(args, 'a') {
            if server_acl_find(id) {
                cmdq_error!(item, "{type_} {name} is already added");
                return cmd_retval::CMD_RETURN_ERROR;
            }
            server_acl_allow(id);
            // Do not return, -r or -w may come with -a.
        } else if args_has(args, 'r') || args_has(args, 'w') {
            // -r or -w adds the entry if there is none.
            server_acl_allow(id);
        }

        if args_has(args, 'w') {
            server_acl_allow_write(id);
        } else if args_has(args, 'r') {
            server_acl_deny_write(id);
        }

        cmd_retval::CMD_RETURN_NORMAL
    }
}
