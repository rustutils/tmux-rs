// Copyright (c) 2021 Anindya Mukherjee <anindya49@hotmail.com>
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
//! show-prompt-history and clear-prompt-history, converted to run in a
//! [`CmdCtx`].

#![forbid(unsafe_code)]

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult};
use crate::prompt_history::PromptType;
use crate::{CmdExec, args_parse, cmd_entry, cmd_entry_flag, cmd_flag};

pub static CMD_SHOW_PROMPT_HISTORY_ENTRY: cmd_entry = cmd_entry {
    name: "show-prompt-history",
    alias: Some("showphist"),

    args: args_parse::new("T:", 0, 0, None),
    usage: "[-T prompt-type]",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_show_prompt_history_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

pub static CMD_CLEAR_PROMPT_HISTORY_ENTRY: cmd_entry = cmd_entry {
    name: "clear-prompt-history",
    alias: Some("clearphist"),

    args: args_parse::new("T:", 0, 0, None),
    usage: "[-T prompt-type]",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_clear_prompt_history_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// Errors of the prompt history commands, with C's messages.
#[derive(Debug, thiserror::Error)]
pub enum PromptHistoryError {
    /// -T names no prompt type.
    #[error("invalid type: {0}")]
    InvalidType(String),
}

/// The types -T names: one, or all without -T.
fn types(ctx: &CmdCtx<'_>) -> Result<Vec<PromptType>, PromptHistoryError> {
    match ctx.args().get('T') {
        None => Ok(PromptType::ALL.to_vec()),
        Some(name) => PromptType::from_name(name)
            .map(|ty| vec![ty])
            .ok_or_else(|| PromptHistoryError::InvalidType(String::from_utf8_lossy(name).into())),
    }
}

/// show-prompt-history: list the history of each type, numbered from 1.
fn cmd_show_prompt_history_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    for ty in types(ctx)? {
        ctx.print(format!("History for {}:\n", ty.name()));
        for (i, line) in ctx.prompt_history().entries(ty).into_iter().enumerate() {
            let mut out = format!("{}: ", i + 1).into_bytes();
            out.extend_from_slice(&line);
            ctx.print(out);
        }
        ctx.print("");
    }
    Ok(CmdFlow::Done)
}

/// clear-prompt-history: delete the history of one type, or all.
fn cmd_clear_prompt_history_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    for ty in types(ctx)? {
        ctx.prompt_history().clear(ty);
    }
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd_::context::CmdEffect;
    use crate::cmd_::context::testing::{args, ctx, parts};
    use crate::global::with_prompt_history;

    fn printed(effects: Vec<CmdEffect>) -> Vec<String> {
        effects
            .into_iter()
            .filter_map(|e| match e {
                CmdEffect::Print(p) => Some(String::from_utf8(p).unwrap()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn show_and_clear() {
        with_prompt_history(|h| {
            h.add(PromptType::Command, b"one", 10);
            h.add(PromptType::Search, b"two", 10);
        });
        let a = args(&[], &[]);
        let mut c = ctx(&a, parts());
        cmd_show_prompt_history_exec(&mut c).unwrap();
        assert_eq!(
            printed(c.take_effects()),
            [
                "History for command:\n",
                "1: one",
                "",
                "History for search:\n",
                "1: two",
                ""
            ]
        );

        let a = args(&[('T', Some("search"))], &[]);
        cmd_clear_prompt_history_exec(&mut ctx(&a, parts())).unwrap();
        assert!(with_prompt_history(|h| h.entries(PromptType::Search).is_empty()));
        assert_eq!(with_prompt_history(|h| h.entries(PromptType::Command).len()), 1);
        let a = args(&[], &[]);
        cmd_clear_prompt_history_exec(&mut ctx(&a, parts())).unwrap();
        assert!(with_prompt_history(|h| h.entries(PromptType::Command).is_empty()));
    }

    #[test]
    fn invalid_type() {
        for exec in [cmd_show_prompt_history_exec, cmd_clear_prompt_history_exec] {
            let a = args(&[('T', Some("target"))], &[]);
            let result = exec(&mut ctx(&a, parts()));
            assert_eq!(result.err().unwrap().to_string(), "invalid type: target");
        }
    }
}
