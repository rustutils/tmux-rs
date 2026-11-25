// Copyright (c) 2009 Tiago Cunha <me@tiagocunha.org>
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
//! load-buffer, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult, FileDone, FilePath};
use crate::{ClientId, CmdExec, args_parse, cmd_entry, cmd_entry_flag, cmd_flag};

pub static CMD_LOAD_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "load-buffer",
    alias: Some("loadb"),

    args: args_parse::new("b:t:w", 1, 1, None),
    usage: "[-b buffer-name] [-t target-client] path",

    flags: cmd_flag::CMD_AFTERHOOK
        .union(cmd_flag::CMD_CLIENT_TFLAG)
        .union(cmd_flag::CMD_CLIENT_CANFAIL),
    exec: CmdExec::Ctx(cmd_load_buffer_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// load-buffer: read a client file (the path expanded as a format) into
/// the buffer named with -b, or a new automatic one. -w also sends it to
/// the target client's clipboard.
fn cmd_load_buffer_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let args = ctx.args();
    let name = args.get('b').map(<[u8]>::to_vec);
    let client = if args.has('w') {
        ctx.target_client().map(|tc| tc.id)
    } else {
        None
    };
    let path = args.string(0).unwrap_or_default().to_vec();
    Ok(CmdFlow::ReadFile {
        path: FilePath::Format(path),
        then: Box::new(move |ctx, done| load_done(ctx, done, name.as_deref(), client)),
    })
}

/// After reading the file: set the buffer to what was read, unless it
/// was empty.
fn load_done(
    ctx: &mut CmdCtx<'_>,
    done: FileDone,
    name: Option<&[u8]>,
    client: Option<ClientId>,
) -> CmdResult {
    let data = done?;
    if data.is_empty() {
        return Ok(CmdFlow::Done);
    }
    let data: Rc<[u8]> = data.into();
    ctx.paste().set(name, data.clone())?;
    if let Some(client) = client {
        ctx.set_selection(client, data);
    }
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd_::context::testing::{args, ctx, parts, paste_events};
    use crate::cmd_::context::{ClientInfo, CmdEffect, FileError};
    use crate::paste::PasteEvent;

    /// Runs load-buffer with `flags` and hands its continuation `done`.
    fn load(flags: &[(char, Option<&str>)], done: FileDone) -> (CmdResult, Vec<CmdEffect>) {
        let a = args(flags, &["~/f"]);
        let mut p = parts();
        p.target_client = Some(ClientInfo {
            id: ClientId(4),
            attached: true,
            control: false,
        });
        let mut c = ctx(&a, p);
        let Ok(CmdFlow::ReadFile {
            path: FilePath::Format(path),
            then,
        }) = cmd_load_buffer_exec(&mut c)
        else {
            panic!("expected a read");
        };
        assert_eq!(path, b"~/f");
        assert!(c.take_effects().is_empty(), "nothing happens before the read");
        let result = then(&mut c, done);
        (result, c.take_effects())
    }

    #[test]
    fn sets_the_buffer_and_selection() {
        let (result, effects) = load(&[('b', Some("f")), ('w', None)], Ok(b"text".to_vec()));
        assert!(matches!(result, Ok(CmdFlow::Done)));
        assert_eq!(paste_events(&effects), [PasteEvent::Changed("f".into())]);
        assert!(matches!(
            effects.last(),
            Some(CmdEffect::SetSelection { client: ClientId(4), data }) if &data[..] == b"text"
        ));

        let (_, effects) = load(&[], Ok(b"text".to_vec()));
        assert_eq!(paste_events(&effects), [PasteEvent::Changed("buffer0".into())]);
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, CmdEffect::SetSelection { .. })),
            "no selection without -w"
        );
    }

    #[test]
    fn empty_files_and_errors() {
        let (result, effects) = load(&[('w', None)], Ok(Vec::new()));
        assert!(matches!(result, Ok(CmdFlow::Done)));
        assert!(effects.is_empty());

        let failed = Err(FileError {
            message: "No such file or directory".into(),
            path: "/x".into(),
        });
        let (result, _) = load(&[], failed);
        assert_eq!(
            result.err().unwrap().to_string(),
            "No such file or directory: /x"
        );

        let (result, effects) = load(&[('b', Some("a\tb")), ('w', None)], Ok(b"x".to_vec()));
        assert_eq!(result.err().unwrap().to_string(), "invalid buffer name: a\tb");
        assert!(effects.is_empty(), "no selection when the buffer was not set");
    }
}
