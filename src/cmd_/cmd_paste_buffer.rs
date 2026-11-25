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
//! paste-buffer, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult};
use crate::compat::vis_flags;
use crate::utf8::utf8_strvis_;
use crate::{
    CmdExec, args_parse, cmd_entry, cmd_entry_flag, cmd_find_flags, cmd_find_type, cmd_flag,
};

pub static CMD_PASTE_BUFFER_ENTRY: cmd_entry = cmd_entry {
    name: "paste-buffer",
    alias: Some("pasteb"),

    args: args_parse::new("db:prSs:t:", 0, 0, None),
    usage: "[-dprS] [-s separator] [-b buffer-name] [-t target-pane]",

    target: cmd_entry_flag::new(b't', cmd_find_type::CMD_FIND_PANE, cmd_find_flags::empty()),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_paste_buffer_exec),
    source: cmd_entry_flag::zeroed(),
};

/// Errors of paste-buffer, with C's messages.
#[derive(Debug, thiserror::Error)]
pub enum PasteBufferError {
    /// The target pane's program has exited.
    #[error("target pane has exited")]
    PaneExited,
    /// -b names a buffer that does not exist.
    #[error("no buffer {0}")]
    NoBuffer(String),
}

/// What paste-buffer writes to the pane for `data`: each line followed
/// by `separator` instead of its newline. Since 3.8 control characters
/// are escaped with vis (as ^X, tab and carriage return kept) unless
/// `raw` (-S). `bracket` wraps it all in bracketed paste markers.
fn paste_bytes(data: &[u8], separator: &[u8], raw: bool, bracket: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 12);
    let add = |out: &mut Vec<u8>, line: &[u8]| {
        if raw {
            out.extend_from_slice(line);
        } else {
            utf8_strvis_(out, line, vis_flags::VIS_SAFE | vis_flags::VIS_NOSLASH);
        }
    };

    if bracket {
        out.extend_from_slice(b"\x1b[200~");
    }
    let mut rest = data;
    while let Some(end) = memchr::memchr(b'\n', rest) {
        add(&mut out, &rest[..end]);
        out.extend_from_slice(separator);
        rest = &rest[end + 1..];
    }
    if !rest.is_empty() {
        add(&mut out, rest);
    }
    if bracket {
        out.extend_from_slice(b"\x1b[201~");
    }
    out
}

/// paste-buffer: write the buffer named with -b (or the newest automatic
/// one) to the target pane, then delete it with -d. -r and -s change the
/// line separator, -p brackets the paste if the program asked for it.
fn cmd_paste_buffer_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let args = ctx.args();
    // The queue only runs the command once it found the target pane.
    let Some(wp) = ctx.target_pane() else {
        debug_assert!(false, "paste-buffer without a target pane");
        return Err(PasteBufferError::PaneExited.into());
    };
    if wp.exited {
        return Err(PasteBufferError::PaneExited.into());
    }

    let pb = match args.get('b') {
        Some(name) => Some(
            ctx.paste()
                .get(name)
                .ok_or_else(|| PasteBufferError::NoBuffer(String::from_utf8_lossy(name).into()))?,
        ),
        None => ctx.paste().top(),
    };

    if let Some(pb) = &pb
        && !wp.input_off
    {
        let separator: &[u8] = match args.get('s') {
            Some(separator) => separator,
            None if args.has('r') => b"\n",
            None => b"\r",
        };
        let bracket = args.has('p') && wp.bracket_paste;
        let data = paste_bytes(pb.data(), separator, args.has('S'), bracket);
        if !data.is_empty() {
            ctx.write_pane(wp.id, data);
        }
    }

    if let Some(pb) = pb
        && args.has('d')
    {
        ctx.paste().delete(pb.name());
    }
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::PaneId;
    use crate::cmd_::context::testing::{args, ctx, parts, paste_events};
    use crate::cmd_::context::{CmdEffect, CmdCtxParts, PaneInfo};
    use crate::paste::PasteEvent;

    // Valid UTF-8 passes through only under a UTF-8 locale (mbtowc), so
    // these use ASCII and invalid bytes. tests/buffers.rs has the rest.
    #[test]
    fn escapes_unless_raw() {
        let data = b"a\x01b\x1bc\t\x7f\r\nd\xff";
        assert_eq!(paste_bytes(data, b"\r", false, false), b"a^Ab^[c\t^?\r\rdM^?");
        assert_eq!(paste_bytes(data, b"<>", true, false), b"a\x01b\x1bc\t\x7f\r<>d\xff");
        assert_eq!(paste_bytes(b"1\n2\n", b",", false, true), b"\x1b[200~1,2,\x1b[201~");
        assert_eq!(paste_bytes(b"", b",", false, false), b"");
    }

    fn pane_parts(pane: PaneInfo) -> CmdCtxParts {
        let mut p = parts();
        p.target_pane = Some(pane);
        p
    }

    fn pane() -> PaneInfo {
        PaneInfo {
            id: PaneId(3),
            exited: false,
            input_off: false,
            bracket_paste: true,
        }
    }

    fn written(effects: &[CmdEffect]) -> Vec<Vec<u8>> {
        effects
            .iter()
            .filter_map(|e| match e {
                CmdEffect::WritePane { pane: PaneId(3), data } => Some(data.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn pastes_and_deletes() {
        let a = args(&[], &[]);
        let mut c = ctx(&a, pane_parts(pane()));
        c.paste().set(Some(b"n"), Rc::from(&b"x\ny"[..])).unwrap();
        c.paste().set(None, Rc::from(&b"top"[..])).unwrap();
        c.take_effects();

        let a = args(&[('b', Some("n")), ('d', None), ('p', None), ('r', None)], &[]);
        let mut c = ctx(&a, pane_parts(pane()));
        cmd_paste_buffer_exec(&mut c).unwrap();
        let effects = c.take_effects();
        assert_eq!(written(&effects), [b"\x1b[200~x\ny\x1b[201~".to_vec()]);
        assert_eq!(paste_events(&effects), [PasteEvent::Deleted("n".into())]);

        // Without -b the newest automatic buffer. Input turned off pastes
        // nothing, but -d still deletes.
        let a = args(&[('d', None)], &[]);
        let mut p = pane();
        p.input_off = true;
        let mut c = ctx(&a, pane_parts(p));
        cmd_paste_buffer_exec(&mut c).unwrap();
        let effects = c.take_effects();
        assert!(written(&effects).is_empty());
        assert_eq!(paste_events(&effects), [PasteEvent::Deleted("buffer0".into())]);

        // No buffers at all is not an error.
        let a = args(&[], &[]);
        let mut c = ctx(&a, pane_parts(pane()));
        cmd_paste_buffer_exec(&mut c).unwrap();
        assert!(c.take_effects().is_empty());
    }

    #[test]
    fn errors() {
        let a = args(&[], &[]);
        let mut p = pane();
        p.exited = true;
        let result = cmd_paste_buffer_exec(&mut ctx(&a, pane_parts(p)));
        assert_eq!(result.err().unwrap().to_string(), "target pane has exited");
        let a = args(&[('b', Some("nope"))], &[]);
        let result = cmd_paste_buffer_exec(&mut ctx(&a, pane_parts(pane())));
        assert_eq!(result.err().unwrap().to_string(), "no buffer nope");
    }
}
