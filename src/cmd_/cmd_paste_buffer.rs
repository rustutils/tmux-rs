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

pub static CMD_PASTE_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "paste-buffer",
    alias: Some("pasteb"),

    args: args_parse::new("db:prs:t:", 0, 0, None),
    usage: "[-dpr] [-s separator] [-b buffer-name] [-t target-pane]",

    target: cmd_entry_flag::new(b't', cmd_find_type::CMD_FIND_PANE, cmd_find_flags::empty()),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Old(cmd_paste_buffer_exec),
    source: cmd_entry_flag::zeroed(),
};

unsafe fn cmd_paste_buffer_exec(self_: *mut cmd, item: *mut cmdq_item) -> cmd_retval {
    unsafe {
        let args = cmd_get_args(self_);
        let target = cmdq_get_target(item);
        let wp = (*target).wp.and_then(|id| pane_from_id(id)).unwrap_or(null_mut());
        let bracket = args_has(args, 'p');

        if window_pane_exited(wp) {
            cmdq_error!(item, "target pane has exited");
            return cmd_retval::CMD_RETURN_ERROR;
        }

        let mut bufname = None;
        if args_has(args, 'b') {
            bufname = cstr_to_bytes(args_get(args, b'b'));
        }

        let pb;
        if let Some(bufname) = bufname {
            pb = paste_get_name_bytes(bufname);
            if pb.is_none() {
                cmdq_error!(item, "no buffer {}", String::from_utf8_lossy(bufname));
                return cmd_retval::CMD_RETURN_ERROR;
            }
        } else {
            pb = paste_get_top();
        }

        if let Some(pb) = &pb
            && !(*wp).flags.intersects(window_pane_flags::PANE_INPUTOFF)
        {
            let mut sepstr = args_get(args, b's');
            if sepstr.is_null() {
                if args_has(args, 'r') {
                    sepstr = c!("\n");
                } else {
                    sepstr = c!("\r");
                }
            }
            let seplen = strlen(sepstr);

            if bracket
                && (*(*wp).screen)
                    .mode
                    .intersects(mode_flag::MODE_BRACKETPASTE)
            {
                window_pane_write_to_pty(wp, c!("\x1b[200~").cast(), 6);
            }

            // Each line is written followed by the separator instead of
            // its newline.
            let mut rest: &[u8] = pb.data();
            while let Some(end) = memchr::memchr(b'\n', rest) {
                window_pane_write_to_pty(wp, rest.as_ptr().cast(), end);
                window_pane_write_to_pty(wp, sepstr.cast(), seplen);
                rest = &rest[end + 1..];
            }
            if !rest.is_empty() {
                window_pane_write_to_pty(wp, rest.as_ptr().cast(), rest.len());
            }

            if bracket
                && (*(*wp).screen)
                    .mode
                    .intersects(mode_flag::MODE_BRACKETPASTE)
            {
                window_pane_write_to_pty(wp, c!("\x1b[201~").cast(), 6);
            }
        }

        if let Some(pb) = pb
            && args_has(args, 'd')
        {
            paste_free(pb.name());
        }

        cmd_retval::CMD_RETURN_NORMAL
    }
}
