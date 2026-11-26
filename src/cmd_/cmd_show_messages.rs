// Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
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
//! show-messages, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult, FormatSubject};
use crate::{CmdExec, args_parse, cmd_entry, cmd_entry_flag, cmd_flag};

/// How each message is shown.
const SHOW_MESSAGES_TEMPLATE: &[u8] = b"#{t/p:message_time}: #{message_text}";

pub static CMD_SHOW_MESSAGES_ENTRY: cmd_entry = cmd_entry {
    name: "show-messages",
    alias: Some("showmsgs"),

    args: args_parse::new("JTt:", 0, 0, None),
    usage: "[-JT] [-t target-client]",

    flags: cmd_flag::CMD_AFTERHOOK
        .union(cmd_flag::CMD_CLIENT_TFLAG)
        .union(cmd_flag::CMD_CLIENT_CANFAIL),
    exec: CmdExec::Ctx(cmd_show_messages_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// show-messages: the terminals in use (-T) and the jobs (-J), or else
/// the message log, newest first.
fn cmd_show_messages_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let args = ctx.args();
    let (terminals, jobs) = (args.has('T'), args.has('J'));
    if terminals || jobs {
        // -t limits -T to the terminal of the target client.
        let terminal_client = if args.has('t') {
            ctx.target_client().map(|c| c.id)
        } else {
            None
        };
        ctx.print_summaries(terminals, terminal_client, jobs);
        return Ok(CmdFlow::Done);
    }
    for msg in ctx.messages() {
        ctx.print_format(SHOW_MESSAGES_TEMPLATE, None, FormatSubject::Message(msg));
    }
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ClientId;
    use crate::cmd_::context::{ClientInfo, CmdEffect};
    use crate::cmd_::context::testing::{args, ctx, parts};
    use crate::global::with_message_log;

    #[test]
    fn summaries() {
        let mut p = parts();
        p.target_client = Some(ClientInfo {
            id: ClientId(4),
            attached: true,
            control: false,
        });
        let a = args(&[('T', None), ('J', None), ('t', Some("/dev/x"))], &[]);
        let mut c = ctx(&a, p);
        cmd_show_messages_exec(&mut c).unwrap();
        let effects = c.take_effects();
        assert!(matches!(
            effects.as_slice(),
            [CmdEffect::PrintSummaries {
                terminals: true,
                terminal_client: Some(ClientId(4)),
                jobs: true
            }]
        ));
    }

    #[test]
    fn messages_newest_first() {
        with_message_log(|log| {
            log.add("one".into(), std::time::UNIX_EPOCH, 10);
            log.add("two".into(), std::time::UNIX_EPOCH, 10);
        });
        let a = args(&[], &[]);
        let mut c = ctx(&a, parts());
        cmd_show_messages_exec(&mut c).unwrap();
        let texts: Vec<String> = c
            .take_effects()
            .into_iter()
            .map(|e| match e {
                CmdEffect::PrintFormat {
                    subject: FormatSubject::Message(m),
                    ..
                } => m.text().to_string(),
                _ => panic!("unexpected effect"),
            })
            .collect();
        assert_eq!(texts, ["two", "one"]);
    }
}
