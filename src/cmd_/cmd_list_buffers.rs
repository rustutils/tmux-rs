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

//! list-buffers, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult, FormatSubject};
use crate::sort::SortCriteria;
use crate::{CmdExec, args_parse, cmd_entry, cmd_entry_flag, cmd_flag};

pub static CMD_LIST_BUFFERS_ENTRY: cmd_entry = cmd_entry {
    name: "list-buffers",
    alias: Some("lsb"),

    args: args_parse::new("F:f:O:r", 0, 0, None),
    usage: "[-F format] [-f filter] [-O order]",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_list_buffers_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// The line printed for each buffer without -F.
const LIST_BUFFERS_TEMPLATE: &[u8] = b"#{buffer_name}: #{buffer_size} bytes: \"#{buffer_sample}\"";

/// list-buffers: print a line for each buffer that passes the -f filter,
/// newest first or in the -O order (-r reverses it).
fn cmd_list_buffers_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let args = ctx.args();
    let template = args.get('F').unwrap_or(LIST_BUFFERS_TEMPLATE);
    let filter = args.get('f');
    // An order that is not UTF-8 is not an order name.
    let order = args.get('O').map(|o| std::str::from_utf8(o).unwrap_or(""));
    let criteria = SortCriteria::from_args(order, args.has('r'))?;

    for pb in ctx.paste().sorted(&criteria) {
        ctx.print_format(template, filter, FormatSubject::PasteBuffer(pb));
    }
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::cmd_::context::CmdEffect;
    use crate::cmd_::context::testing::{args, ctx, parts};

    /// The buffer names and templates printed.
    fn printed(effects: Vec<CmdEffect>) -> Vec<(String, Vec<u8>, Option<Vec<u8>>)> {
        effects
            .into_iter()
            .filter_map(|e| match e {
                CmdEffect::PrintFormat {
                    template,
                    filter,
                    subject: FormatSubject::PasteBuffer(pb),
                } => Some((pb.name().to_string(), template, filter)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn lists_in_order_with_the_template() {
        let a = args(&[], &[]);
        let mut c = ctx(&a, parts());
        c.paste().set(Some(b"b"), Rc::from(&b"long"[..])).unwrap();
        c.paste().set(Some(b"a"), Rc::from(&b"x"[..])).unwrap();
        c.take_effects();

        let a = args(&[], &[]);
        let mut c = ctx(&a, parts());
        cmd_list_buffers_exec(&mut c).unwrap();
        let lines = printed(c.take_effects());
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, "a", "newest first");
        assert_eq!(lines[0].1, LIST_BUFFERS_TEMPLATE);
        assert_eq!(lines[0].2, None);

        let a = args(
            &[('O', Some("name")), ('r', None), ('F', Some("#{buffer_size}")), ('f', Some("1"))],
            &[],
        );
        let mut c = ctx(&a, parts());
        cmd_list_buffers_exec(&mut c).unwrap();
        let lines = printed(c.take_effects());
        let names: Vec<_> = lines.iter().map(|l| l.0.as_str()).collect();
        assert_eq!(names, ["b", "a"]);
        assert_eq!(lines[0].1, b"#{buffer_size}");
        assert_eq!(lines[0].2.as_deref(), Some(&b"1"[..]));
    }

    #[test]
    fn invalid_order() {
        let a = args(&[('O', Some("bogus"))], &[]);
        let result = cmd_list_buffers_exec(&mut ctx(&a, parts()));
        assert_eq!(result.err().map(|e| e.to_string()).as_deref(), Some("invalid sort order"));
    }
}
