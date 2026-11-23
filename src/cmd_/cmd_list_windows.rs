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

const LIST_WINDOWS_TEMPLATE: *const u8 = c!(
    "#{window_index}: #{window_name}#{window_raw_flags} (#{window_panes} panes) [#{window_width}x#{window_height}] [layout #{window_layout}] #{window_id}#{?window_active, (active),}"
);
const LIST_WINDOWS_WITH_SESSION_TEMPLATE: *const u8 = c!(
    "#{session_name}:#{window_index}: #{window_name}#{window_raw_flags} (#{window_panes} panes) [#{window_width}x#{window_height}] "
);

pub static CMD_LIST_WINDOWS_ENTRY: cmd_entry = cmd_entry {
    name: "list-windows",
    alias: Some("lsw"),

    args: args_parse::new("aF:f:O:rt:", 0, 0, None),
    usage: "[-ar] [-F format] [-f filter] [-O order][-t target-session]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_SESSION,
        cmd_find_flags::empty(),
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: cmd_list_windows_exec,
    source: cmd_entry_flag::zeroed(),
};

/// Lists the windows of the target session, or with -a of every session,
/// sorted by -O and -r.
unsafe fn cmd_list_windows_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let target = cmdq_get_target(item);
        let c = cmdq_get_client(item);

        let mut template = args_get(args, b'F');
        let filter = args_get(args, b'f');

        let criteria = match args_sort_criteria(args) {
            Ok(criteria) => criteria,
            Err(e) => {
                cmdq_error!(item, "{e}");
                return cmd_retval::CMD_RETURN_ERROR;
            }
        };

        let l = if args_has(args, 'a') {
            if template.is_null() {
                template = LIST_WINDOWS_WITH_SESSION_TEMPLATE;
            }
            sort_get_winlinks(&criteria)
        } else {
            if template.is_null() {
                template = LIST_WINDOWS_TEMPLATE;
            }
            let s = (*target).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
            debug_assert!(!s.is_null(), "list-windows target has a session");
            if s.is_null() {
                return cmd_retval::CMD_RETURN_NORMAL;
            }
            sort_get_winlinks_session(s, &criteria)
        };

        let n = l.len();
        for wl in l {
            let s = (*wl).session.and_then(|id| session_from_id(id));
            let ft = format_create(
                cmdq_get_client(item),
                item,
                FORMAT_NONE,
                format_flags::empty(),
            );
            // C sets line to the number of windows, not the line number.
            format_add!(ft, "line", "{n}");
            format_defaults(ft, c, s.and_then(NonNull::new), NonNull::new(wl), None);

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
