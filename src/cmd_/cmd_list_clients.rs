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

const LIST_CLIENTS_TEMPLATE: *const u8 = c!(
    "#{client_name}: #{session_name} [#{client_width}x#{client_height} #{client_termname}] #{?#{!=:#{client_uid},#{uid}},[user #{?client_user,#{client_user},#{client_uid},}] ,}#{?client_flags,(,}#{client_flags}#{?client_flags,),}"
);

pub static CMD_LIST_CLIENTS_ENTRY: cmd_entry = cmd_entry {
    name: "list-clients",
    alias: Some("lsc"),

    args: args_parse::new("F:f:O:rt:", 0, 0, None),
    usage: "[-F format] [-f filter] [-O order][-t target-session]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_SESSION,
        cmd_find_flags::empty(),
    ),

    flags: cmd_flag::CMD_READONLY.union(cmd_flag::CMD_AFTERHOOK),
    exec: CmdExec::Old(cmd_list_clients_exec),
    source: cmd_entry_flag::zeroed(),
};

unsafe fn cmd_list_clients_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let target = cmdq_get_target(item);

        let s = if args_has(args, 't') {
            (*target).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut())
        } else {
            null_mut()
        };

        let mut template = args_get(args, b'F');
        if template.is_null() {
            template = LIST_CLIENTS_TEMPLATE;
        }
        let filter = args_get(args, b'f');

        let criteria = match args_sort_criteria(args) {
            Ok(criteria) => criteria,
            Err(e) => {
                cmdq_error!(item, "{e}");
                return cmd_retval::CMD_RETURN_ERROR;
            }
        };

        // The line number counts the clients skipped by -t, as in C.
        for (idx, c) in sort_get_clients(&criteria).into_iter().enumerate() {
            if client_get_session(c).is_null() || (!s.is_null() && s != client_get_session(c)) {
                continue;
            }

            let ft = format_create(
                cmdq_get_client(item),
                item,
                FORMAT_NONE,
                format_flags::empty(),
            );
            format_add!(ft, "line", "{idx}");
            format_defaults(ft, c, None, None, None);

            let flag;
            if !filter.is_null() {
                let expanded = format_expand(ft, filter);
                flag = format_true(expanded);
                free_(expanded);
            } else {
                flag = true;
            }
            if flag {
                let line = format_expand(ft, template);
                cmdq_print!(item, "{}", _s(line));
                free_(line);
            }

            format_free(ft);
        }

        cmd_retval::CMD_RETURN_NORMAL
    }
}
