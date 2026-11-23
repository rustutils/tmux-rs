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
use std::rc::Rc;

use crate::*;

pub static CMD_SET_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "set-buffer",
    alias: Some("setb"),

    args: args_parse::new("ab:t:n:w", 0, 1, None),
    usage: "[-aw] [-b buffer-name] [-n new-buffer-name] [-t target-client] data",

    flags: cmd_flag::CMD_AFTERHOOK
        .union(cmd_flag::CMD_CLIENT_TFLAG)
        .union(cmd_flag::CMD_CLIENT_CANFAIL),
    exec: cmd_set_buffer_exec,
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

pub static CMD_DELETE_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "delete-buffer",
    alias: Some("deleteb"),

    args: args_parse::new("b:", 0, 0, None),
    usage: "[-b buffer-name]",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: cmd_set_buffer_exec,
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

unsafe fn cmd_set_buffer_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let tc = cmdq_get_target_client(item);

        // Names can be any bytes when they come from a client, but only
        // valid UTF-8 can name an existing buffer.
        let bufname = cstr_to_bytes(args_get_(args, 'b'));
        let mut pb = bufname.and_then(paste_get_name_bytes);

        if std::ptr::eq(cmd_get_entry(self_), &CMD_DELETE_BUFFER_ENTRY) {
            if pb.is_none() {
                if let Some(bufname) = bufname {
                    cmdq_error!(item, "unknown buffer: {}", String::from_utf8_lossy(bufname));
                    return cmd_retval::CMD_RETURN_ERROR;
                }
                pb = paste_get_top();
            }
            let Some(pb) = pb else {
                cmdq_error!(item, "no buffer");
                return cmd_retval::CMD_RETURN_ERROR;
            };
            paste_free(pb.name());
            return cmd_retval::CMD_RETURN_NORMAL;
        }

        if args_has(args, 'n') {
            if pb.is_none() {
                if let Some(bufname) = bufname {
                    cmdq_error!(item, "unknown buffer: {}", String::from_utf8_lossy(bufname));
                    return cmd_retval::CMD_RETURN_ERROR;
                }
                pb = paste_get_top();
            }
            let Some(pb) = pb else {
                cmdq_error!(item, "no buffer");
                return cmd_retval::CMD_RETURN_ERROR;
            };
            let newname = cstr_to_bytes(args_get_(args, 'n')).unwrap_or_default();
            if let Err(cause) = paste_rename(pb.name(), newname) {
                cmdq_error!(item, "{}", cause);
                return cmd_retval::CMD_RETURN_ERROR;
            }
            return cmd_retval::CMD_RETURN_NORMAL;
        }

        if args_count(args) != 1 {
            cmdq_error!(item, "no data specified");
            return cmd_retval::CMD_RETURN_ERROR;
        }
        let newdata = cstr_to_bytes(args_string(args, 0)).unwrap_or_default();
        if newdata.is_empty() {
            return cmd_retval::CMD_RETURN_NORMAL;
        }

        let mut bufdata = Vec::new();
        if let Some(pb) = &pb
            && args_has(args, 'a')
        {
            bufdata.extend_from_slice(pb.data());
        }
        bufdata.extend_from_slice(newdata);
        let bufdata: Rc<[u8]> = bufdata.into();

        if let Err(cause) = paste_set(bufdata.clone(), bufname) {
            cmdq_error!(item, "{}", cause);
            return cmd_retval::CMD_RETURN_ERROR;
        }
        if args_has(args, 'w') && !tc.is_null() {
            tty_set_selection(&raw mut (*tc).tty, c!(""), bufdata.as_ptr(), bufdata.len());
        }

        cmd_retval::CMD_RETURN_NORMAL
    }
}
