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

//! unbind-key, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult, QuietError};
use crate::key_string::key_string_lookup_bytes;
use crate::{CmdExec, KEYC_MASK_FLAGS, args_parse, cmd_entry, cmd_entry_flag, cmd_flag};

pub static CMD_UNBIND_KEY_ENTRY: cmd_entry = cmd_entry {
    name: "unbind-key",
    alias: Some("unbind"),

    args: args_parse::new("anqT:", 0, 1, None),
    usage: "[-anq] [-T key-table] key",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_unbind_key_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// Errors of unbind-key, with C's messages.
#[derive(Debug, thiserror::Error)]
pub enum UnbindKeyError {
    #[error("key given with -a")]
    KeyWithAll,
    #[error("missing key")]
    MissingKey,
    #[error("unknown key: {0}")]
    UnknownKey(String),
    #[error("table {0} doesn't exist")]
    NoTable(String),
}

/// unbind-key: remove one binding, or a whole table with -a. With -q an
/// error prints nothing, but still fails.
fn cmd_unbind_key_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let args = ctx.args();
    let fail = |error: UnbindKeyError| -> CmdResult {
        if args.has('q') {
            Err(QuietError.into())
        } else {
            Err(error.into())
        }
    };
    let lossy = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
    let default_table: &[u8] = if args.has('n') { b"root" } else { b"prefix" };
    let keystr = args.string(0);

    if args.has('a') {
        if keystr.is_some() {
            return fail(UnbindKeyError::KeyWithAll);
        }
        let table = args.get('T').unwrap_or(default_table);
        if !ctx.has_key_table(table) {
            return fail(UnbindKeyError::NoTable(lossy(table)));
        }
        ctx.remove_key_table(table);
        return Ok(CmdFlow::Done);
    }

    let Some(keystr) = keystr else {
        return fail(UnbindKeyError::MissingKey);
    };
    let Some(key) = key_string_lookup_bytes(keystr) else {
        return fail(UnbindKeyError::UnknownKey(lossy(keystr)));
    };
    let table = match args.get('T') {
        Some(table) => {
            if !ctx.has_key_table(table) {
                return fail(UnbindKeyError::NoTable(lossy(table)));
            }
            table
        }
        None => default_table,
    };
    ctx.unbind_key(table, key & !KEYC_MASK_FLAGS);
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd_::context::testing::{args, ctx, parts};

    fn run(flags: &[(char, Option<&str>)], values: &[&str]) -> Result<(), String> {
        let a = args(flags, values);
        let mut c = ctx(&a, parts());
        cmd_unbind_key_exec(&mut c).map(drop).map_err(|e| e.to_string())
    }

    #[test]
    fn errors() {
        assert_eq!(run(&[('a', None)], &["x"]).unwrap_err(), "key given with -a");
        assert_eq!(run(&[], &[]).unwrap_err(), "missing key");
        assert_eq!(run(&[('T', Some("none"))], &["x"]).unwrap_err(), "table none doesn't exist");
        assert_eq!(run(&[('a', None), ('T', Some("none"))], &[]).unwrap_err(), "table none doesn't exist");
        // -q fails without a message.
        assert_eq!(run(&[('q', None)], &[]).unwrap_err(), "");
    }
}
