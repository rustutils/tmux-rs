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

pub static CMD_BIND_KEY_ENTRY: cmd_entry = cmd_entry {
    name: "bind-key",
    alias: Some("bind"),

    args: args_parse::new("nrN:T:", 1, -1, Some(cmd_bind_key_args_parse)),
    usage: "[-nr] [-T key-table] [-N note] key [command [arguments]]",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Old(cmd_bind_key_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

unsafe fn cmd_bind_key_args_parse(_args: *mut args, _idx: u32) -> args_parse_type {
    args_parse_type::ARGS_PARSE_COMMANDS_OR_STRING
}

unsafe fn cmd_bind_key_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args: *mut args = cmd_get_args(self_);
        let note = args_get(args, b'N');

        let count: u32 = args_count(args);

        let key: key_code = key_string_lookup_string(args_string(args, 0));
        if key == KEYC_NONE || key == KEYC_UNKNOWN {
            cmdq_error!(item, "unknown key: {}", _s(args_string(args, 0)));
            return cmd_retval::CMD_RETURN_ERROR;
        }

        let tablename: *const u8 = if args_has(args, 'T') {
            args_get(args, b'T')
        } else if args_has(args, 'n') {
            c!("root")
        } else {
            c!("prefix")
        };
        let repeat = args_has(args, 'r');
        let tablename = CStr::from_ptr(tablename.cast()).to_bytes();
        let note = (!note.is_null()).then(|| CStr::from_ptr(note.cast()).to_bytes());

        if count == 1 {
            key_bindings_add(tablename, key, note, repeat, None);
            return cmd_retval::CMD_RETURN_NORMAL;
        }

        if count == 2
            && let Some(args_value::Commands { cmdlist, .. }) = args_value(args, 1)
        {
            // SAFETY: the argument holds a live list. The reference taken
            // here goes to the binding.
            (**cmdlist).references += 1;
            let cmds = CmdList::from_raw(NonNull::new(*cmdlist).expect("parsed commands"));
            key_bindings_add(tablename, key, note, repeat, Some(cmds));
            return cmd_retval::CMD_RETURN_NORMAL;
        }

        let pr = if count == 2 {
            cmd_parse_from_string(cstr_to_str(args_string(args, 1)), None)
        } else {
            cmd_parse_from_arguments(&args_values(args)[1..], None)
        };

        match pr {
            Err(error) => {
                cmdq_error!(item, "{}", error.to_string_lossy());
                cmd_retval::CMD_RETURN_ERROR
            }
            Ok(cmdlist) => {
                // SAFETY: the parser returns a new list with one reference,
                // which goes to the binding.
                let cmds = CmdList::from_raw(NonNull::new(cmdlist).expect("parsed commands"));
                key_bindings_add(tablename, key, note, repeat, Some(cmds));
                cmd_retval::CMD_RETURN_NORMAL
            }
        }
    }
}
