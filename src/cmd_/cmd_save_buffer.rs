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
//! save-buffer and show-buffer, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult, FileDone, FilePath};
use crate::paste::PasteBuffer;
use crate::{CmdExec, args_parse, cmd_entry, cmd_entry_flag, cmd_flag};

pub static CMD_SAVE_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "save-buffer",
    alias: Some("saveb"),

    args: args_parse::new("ab:", 1, 1, None),
    usage: "[-a] [-b buffer-name] path",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_save_buffer_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

pub static CMD_SHOW_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "show-buffer",
    alias: Some("showb"),

    args: args_parse::new("b:", 0, 0, None),
    usage: "[-b buffer-name]",

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_show_buffer_exec),
    source: cmd_entry_flag::zeroed(),
    target: cmd_entry_flag::zeroed(),
};

/// Errors of save-buffer and show-buffer, with C's messages.
#[derive(Debug, thiserror::Error)]
pub enum SaveBufferError {
    /// -b names a buffer that does not exist.
    #[error("no buffer {0}")]
    NoBuffer(String),
    /// There is no automatic buffer to use without -b.
    #[error("no buffers")]
    NoBuffers,
}

/// The buffer named with -b, or the newest automatic one.
fn named_or_top(ctx: &mut CmdCtx<'_>) -> Result<PasteBuffer, SaveBufferError> {
    match ctx.args().get('b') {
        Some(name) => ctx
            .paste()
            .get(name)
            .ok_or_else(|| SaveBufferError::NoBuffer(String::from_utf8_lossy(name).into())),
        None => ctx.paste().top().ok_or(SaveBufferError::NoBuffers),
    }
}

/// After writing the file: report an error writing it.
fn write_done(_ctx: &mut CmdCtx<'_>, done: FileDone) -> CmdResult {
    done?;
    Ok(CmdFlow::Done)
}

/// show-buffer: show the buffer to the client. An attached or control
/// client gets it as command output, any other client on its standard
/// output.
fn cmd_show_buffer_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let pb = named_or_top(ctx)?;
    if let Some(c) = ctx.client()
        && (c.attached || c.control)
    {
        ctx.print(pb.data().to_vec());
        return Ok(CmdFlow::Done);
    }
    Ok(CmdFlow::WriteFile {
        path: FilePath::Literal(b"-".to_vec()),
        append: false,
        data: pb.data().clone(),
        then: Box::new(write_done),
    })
}

/// save-buffer: write the buffer to a client file, the path expanded as
/// a format. -a appends to the file.
fn cmd_save_buffer_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let pb = named_or_top(ctx)?;
    let args = ctx.args();
    Ok(CmdFlow::WriteFile {
        path: FilePath::Format(args.string(0).unwrap_or_default().to_vec()),
        append: args.has('a'),
        data: pb.data().clone(),
        then: Box::new(write_done),
    })
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::ClientId;
    use crate::cmd_::context::testing::{args, ctx, parts};
    use crate::cmd_::context::{ClientInfo, CmdEffect, FileError};

    fn add(name: &str, data: &[u8]) {
        let a = args(&[], &[]);
        let mut c = ctx(&a, parts());
        c.paste().set(Some(name.as_bytes()), Rc::from(data)).unwrap();
    }

    fn error(result: CmdResult) -> String {
        result.err().expect("an error").to_string()
    }

    #[test]
    fn errors() {
        let a = args(&[], &["/x"]);
        assert_eq!(error(cmd_save_buffer_exec(&mut ctx(&a, parts()))), "no buffers");
        assert_eq!(error(cmd_show_buffer_exec(&mut ctx(&a, parts()))), "no buffers");
        let a = args(&[('b', Some("nope"))], &["/x"]);
        assert_eq!(error(cmd_save_buffer_exec(&mut ctx(&a, parts()))), "no buffer nope");
        assert_eq!(error(cmd_show_buffer_exec(&mut ctx(&a, parts()))), "no buffer nope");
    }

    #[test]
    fn save_writes_the_buffer() {
        add("n", b"data");
        let a = args(&[('a', None), ('b', Some("n"))], &["#{pane_id}.txt"]);
        let mut c = ctx(&a, parts());
        let Ok(CmdFlow::WriteFile {
            path: FilePath::Format(path),
            append,
            data,
            then,
        }) = cmd_save_buffer_exec(&mut c)
        else {
            panic!("expected a write");
        };
        assert_eq!(path, b"#{pane_id}.txt");
        assert!(append);
        assert_eq!(&data[..], b"data");

        // An error writing is the command's error.
        let failed = Err(FileError {
            message: "No space left on device".into(),
            path: "/dev/full".into(),
        });
        assert_eq!(error(then(&mut c, failed)), "No space left on device: /dev/full");
    }

    #[test]
    fn show_prints_to_attached_and_control_clients() {
        add("n", b"line 1\nline 2");
        for (attached, control) in [(true, false), (false, true)] {
            let a = args(&[('b', Some("n"))], &[]);
            let mut p = parts();
            p.client = Some(ClientInfo {
                id: ClientId(1),
                attached,
                control,
            });
            let mut c = ctx(&a, p);
            assert!(matches!(cmd_show_buffer_exec(&mut c), Ok(CmdFlow::Done)));
            assert!(matches!(
                c.take_effects().as_slice(),
                [CmdEffect::Print(data)] if data == b"line 1\nline 2"
            ));
        }

        // Other clients get it on standard output.
        let a = args(&[('b', Some("n"))], &[]);
        let result = cmd_show_buffer_exec(&mut ctx(&a, parts()));
        assert!(matches!(
            result,
            Ok(CmdFlow::WriteFile { path: FilePath::Literal(ref p), append: false, .. }) if p == b"-"
        ));
    }
}
