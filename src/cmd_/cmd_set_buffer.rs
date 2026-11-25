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
//! set-buffer and delete-buffer, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult};
use crate::paste::PasteBuffer;
use crate::{CmdExec, args_parse, cmd_entry, cmd_entry_flag, cmd_flag};

pub static CMD_SET_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "set-buffer",
    alias: Some("setb"),

    args: args_parse::new("ab:t:n:w", 0, 1, None),
    usage: "[-aw] [-b buffer-name] [-n new-buffer-name] [-t target-client] [data]",

    flags: cmd_flag::CMD_AFTERHOOK
        .union(cmd_flag::CMD_CLIENT_TFLAG)
        .union(cmd_flag::CMD_CLIENT_CANFAIL),
    exec: CmdExec::Ctx(cmd_set_buffer_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

pub static CMD_DELETE_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "delete-buffer",
    alias: Some("deleteb"),

    args: args_parse::new("b:", 0, 0, None),
    usage: "[-b buffer-name]",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_delete_buffer_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// Errors of set-buffer and delete-buffer, with C's messages. Errors of
/// the paste store itself are passed on as they are.
#[derive(Debug, thiserror::Error)]
pub enum SetBufferError {
    /// -b names a buffer that does not exist.
    #[error("unknown buffer: {0}")]
    UnknownBuffer(String),
    /// There is no automatic buffer to use without -b.
    #[error("no buffer")]
    NoBuffer,
    /// set-buffer without data (and without -n).
    #[error("no data specified")]
    NoData,
}

/// The buffer named `name`, or the newest automatic buffer without one.
fn named_or_top(ctx: &mut CmdCtx<'_>, name: Option<&[u8]>) -> Result<PasteBuffer, SetBufferError> {
    match name {
        // Names can be any bytes when they come from a client, but only
        // valid UTF-8 can name an existing buffer.
        Some(name) => ctx
            .paste()
            .get(name)
            .ok_or_else(|| SetBufferError::UnknownBuffer(String::from_utf8_lossy(name).into())),
        None => ctx.paste().top().ok_or(SetBufferError::NoBuffer),
    }
}

/// delete-buffer: delete the buffer named with -b, or the newest
/// automatic one.
fn cmd_delete_buffer_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let pb = named_or_top(ctx, ctx.args().get('b'))?;
    ctx.paste().delete(pb.name());
    Ok(CmdFlow::Done)
}

/// set-buffer: set (-a append to) a buffer, or rename one with -n. -w
/// also sends the data to the target client's clipboard.
fn cmd_set_buffer_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let args = ctx.args();
    let bufname = args.get('b');

    if args.has('n') {
        let pb = named_or_top(ctx, bufname)?;
        let newname = args.get('n').unwrap_or_default();
        ctx.paste().rename(pb.name(), newname)?;
        return Ok(CmdFlow::Done);
    }

    if args.count() != 1 {
        return Err(SetBufferError::NoData.into());
    }
    let newdata = args.string(0).unwrap_or_default();
    if newdata.is_empty() {
        return Ok(CmdFlow::Done);
    }

    let mut data = Vec::new();
    if args.has('a')
        && let Some(pb) = bufname.and_then(|name| ctx.paste().get(name))
    {
        data.extend_from_slice(pb.data());
    }
    data.extend_from_slice(newdata);
    let data: Rc<[u8]> = data.into();

    ctx.paste().set(bufname, data.clone())?;
    if args.has('w')
        && let Some(tc) = ctx.target_client()
    {
        ctx.set_selection(tc.id, data);
    }
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd_::context::testing::{args, ctx, parts, paste_events};
    use crate::cmd_::context::{ClientInfo, CmdEffect};
    use crate::paste::PasteEvent;
    use crate::ClientId;

    fn error(result: CmdResult) -> String {
        match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected an error"),
        }
    }

    fn data(ctx: &mut CmdCtx<'_>, name: &str) -> Option<Vec<u8>> {
        ctx.paste().get(name.as_bytes()).map(|pb| pb.data().to_vec())
    }

    #[test]
    fn set_adds_and_appends() {
        let a = args(&[('b', Some("x"))], &["one"]);
        let mut c = ctx(&a, parts());
        assert!(matches!(cmd_set_buffer_exec(&mut c), Ok(CmdFlow::Done)));
        let a = args(&[('a', None), ('b', Some("x"))], &[" two"]);
        let mut c = ctx(&a, parts());
        cmd_set_buffer_exec(&mut c).unwrap();
        assert_eq!(data(&mut c, "x").as_deref(), Some(&b"one two"[..]));
        // Replacing a buffer deletes the old one first, as in C.
        assert_eq!(
            paste_events(&c.take_effects()),
            [PasteEvent::Deleted("x".into()), PasteEvent::Changed("x".into())]
        );

        // Empty data adds nothing.
        let a = args(&[], &[""]);
        let mut c = ctx(&a, parts());
        cmd_set_buffer_exec(&mut c).unwrap();
        assert!(c.take_effects().is_empty());
    }

    #[test]
    fn rename_with_n() {
        let a = args(&[], &["data"]);
        cmd_set_buffer_exec(&mut ctx(&a, parts())).unwrap();
        let a = args(&[('n', Some("new"))], &[]);
        let mut c = ctx(&a, parts());
        cmd_set_buffer_exec(&mut c).unwrap();
        assert_eq!(data(&mut c, "new").as_deref(), Some(&b"data"[..]));
        assert_eq!(data(&mut c, "buffer0"), None);
    }

    #[test]
    fn errors_have_the_c_messages() {
        type Case<'a> = (&'a [(char, Option<&'a str>)], &'a [&'a str], &'a str);
        let cases: [Case<'_>; 4] = [
            (&[], &[], "no data specified"),
            (&[('n', Some("x"))], &[], "no buffer"),
            (&[('b', Some("nope")), ('n', Some("x"))], &[], "unknown buffer: nope"),
            (&[('b', Some("a\tb"))], &["x"], "invalid buffer name: a\tb"),
        ];
        for (flags, values, message) in cases {
            let a = args(flags, values);
            assert_eq!(error(cmd_set_buffer_exec(&mut ctx(&a, parts()))), message);
        }
        let a = args(&[], &[]);
        assert_eq!(error(cmd_delete_buffer_exec(&mut ctx(&a, parts()))), "no buffer");
        let a = args(&[('b', Some("nope"))], &[]);
        assert_eq!(
            error(cmd_delete_buffer_exec(&mut ctx(&a, parts()))),
            "unknown buffer: nope"
        );
    }

    #[test]
    fn delete_named_or_top() {
        let a = args(&[('b', Some("k"))], &["1"]);
        cmd_set_buffer_exec(&mut ctx(&a, parts())).unwrap();
        let a = args(&[], &["2"]);
        cmd_set_buffer_exec(&mut ctx(&a, parts())).unwrap();

        // Without -b the newest automatic buffer goes, then -b names one.
        for (a, name) in [(args(&[], &[]), "buffer0"), (args(&[('b', Some("k"))], &[]), "k")] {
            let mut c = ctx(&a, parts());
            cmd_delete_buffer_exec(&mut c).unwrap();
            assert_eq!(
                paste_events(&c.take_effects()),
                [PasteEvent::Deleted(name.into())]
            );
            assert_eq!(data(&mut c, name), None);
        }
    }

    #[test]
    fn w_sets_the_target_clients_selection() {
        let mut p = parts();
        p.target_client = Some(ClientInfo {
            id: ClientId(7),
            attached: true,
            control: false,
        });
        let a = args(&[('w', None)], &["clip"]);
        let mut c = ctx(&a, p);
        cmd_set_buffer_exec(&mut c).unwrap();
        let effects = c.take_effects();
        assert!(matches!(
            effects.last(),
            Some(CmdEffect::SetSelection { client: ClientId(7), data }) if &data[..] == b"clip"
        ));

        // Without a target client there is nobody to send it to.
        let a = args(&[('w', None)], &["clip"]);
        let mut c = ctx(&a, parts());
        cmd_set_buffer_exec(&mut c).unwrap();
        assert!(
            !c.take_effects()
                .iter()
                .any(|e| matches!(e, CmdEffect::SetSelection { .. }))
        );
    }
}
