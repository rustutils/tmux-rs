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
//! show-environment, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use crate::cmd_::cmd_set_environment::{EnvironError, environ_session};
use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult};
use crate::environ::EnvironEntry;
use crate::{CmdExec, args_parse, cmd_entry, cmd_entry_flag, cmd_find_flags, cmd_find_type, cmd_flag};

pub static CMD_SHOW_ENVIRONMENT_ENTRY: cmd_entry = cmd_entry {
    name: "show-environment",
    alias: Some("showenv"),

    args: args_parse::new("hgst:", 0, 1, None),
    usage: "[-hgs] [-t target-session] [variable]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_SESSION,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_show_environment_exec),
    source: cmd_entry_flag::zeroed(),
};

/// Escapes a value for double quotes in a POSIX shell: `$`, `` ` ``, `"`
/// and `\` get a backslash.
fn escape(value: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(value.len() * 2);
    for &c in value {
        if matches!(c, b'$' | b'`' | b'"' | b'\\') {
            out.push(b'\\');
        }
        out.push(c);
    }
    out
}

/// The line show-environment prints for `entry`, None when -h does not
/// match whether it is hidden. With `shell` (-s) it is a shell command.
fn line(entry: &EnvironEntry, hidden: bool, shell: bool) -> Option<Vec<u8>> {
    if hidden != entry.is_hidden() {
        return None;
    }
    let name = entry.name();
    let mut out = Vec::new();
    match (entry.value(), shell) {
        (Some(value), false) => {
            out.extend_from_slice(name);
            out.push(b'=');
            out.extend_from_slice(value);
        }
        (None, false) => {
            out.push(b'-');
            out.extend_from_slice(name);
        }
        (Some(value), true) => {
            out.extend_from_slice(name);
            out.extend_from_slice(b"=\"");
            out.extend_from_slice(&escape(value));
            out.extend_from_slice(b"\"; export ");
            out.extend_from_slice(name);
            out.push(b';');
        }
        (None, true) => {
            out.extend_from_slice(b"unset ");
            out.extend_from_slice(name);
            out.push(b';');
        }
    }
    Some(out)
}

/// show-environment: print one variable or all of the global (-g) or the
/// target session's environment, hidden ones only with -h.
fn cmd_show_environment_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let args = ctx.args();
    if let Some(t) = args.get('t')
        && ctx.target_session().is_none()
    {
        return Err(EnvironError::NoSuchSession(String::from_utf8_lossy(t).into_owned()).into());
    }
    let env = match environ_session(ctx)? {
        None => ctx.global_environ(),
        Some(_) => {
            let s = ctx.target_session().expect("environ_session found it");
            (*s.environ).clone()
        }
    };
    let (hidden, shell) = (args.has('h'), args.has('s'));

    if let Some(name) = args.string(0) {
        let entry = env.find(name).ok_or_else(|| {
            EnvironError::UnknownVariable(String::from_utf8_lossy(name).into_owned())
        })?;
        if let Some(out) = line(entry, hidden, shell) {
            ctx.print(out);
        }
        return Ok(CmdFlow::Done);
    }
    for entry in env.iter() {
        if let Some(out) = line(entry, hidden, shell) {
            ctx.print(out);
        }
    }
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::SessionId;
    use crate::cmd_::context::testing::{args, ctx, parts};
    use crate::cmd_::context::{CmdEffect, SessionInfo};
    use crate::environ::{Environ, EnvironFlags};

    fn show(flags: &[(char, Option<&str>)], values: &[&str]) -> Result<Vec<String>, String> {
        let mut env = Environ::new();
        env.set(b"A", "a$b`c\"d\\e", EnvironFlags::empty());
        env.set(b"H", "hidden", EnvironFlags::HIDDEN);
        env.clear(b"R");
        env.set(b"N\xff", b"\xfe".as_slice(), EnvironFlags::empty());
        let mut p = parts();
        p.target_session = Some(SessionInfo {
            id: SessionId(1),
            environ: Rc::new(env),
        });
        let a = args(flags, values);
        let mut c = ctx(&a, p);
        cmd_show_environment_exec(&mut c).map_err(|e| e.to_string())?;
        Ok(c.take_effects()
            .into_iter()
            .map(|e| match e {
                CmdEffect::Print(p) => String::from_utf8_lossy(&p).into_owned(),
                _ => panic!("unexpected effect"),
            })
            .collect())
    }

    #[test]
    fn lists_and_finds() {
        assert_eq!(
            show(&[], &[]).unwrap(),
            ["A=a$b`c\"d\\e", "N\u{fffd}=\u{fffd}", "-R"]
        );
        assert_eq!(show(&[('h', None)], &[]).unwrap(), ["H=hidden"]);
        assert_eq!(show(&[], &["H"]).unwrap(), Vec::<String>::new());
        assert_eq!(show(&[], &["NOPE"]).unwrap_err(), "unknown variable: NOPE");
    }

    #[test]
    fn shell_format() {
        assert_eq!(
            show(&[('s', None)], &["A"]).unwrap(),
            [r#"A="a\$b\`c\"d\\e"; export A;"#]
        );
        assert_eq!(show(&[('s', None)], &["R"]).unwrap(), ["unset R;"]);
    }

    #[test]
    fn missing_session() {
        let a = args(&[('t', Some("x")), ('g', None)], &[]);
        let result = cmd_show_environment_exec(&mut ctx(&a, parts()));
        assert_eq!(result.err().unwrap().to_string(), "no such session: x");
        let a = args(&[], &[]);
        let result = cmd_show_environment_exec(&mut ctx(&a, parts()));
        assert_eq!(result.err().unwrap().to_string(), "no current session");
    }
}
