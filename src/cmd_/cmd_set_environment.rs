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
//! set-environment, converted to run in a [`CmdCtx`].

#![forbid(unsafe_code)]

use crate::cmd_::context::{CmdCtx, CmdFlow, CmdResult, EnvironChange};
use crate::{
    CmdExec, SessionId, args_parse, cmd_entry, cmd_entry_flag, cmd_find_flags, cmd_find_type,
    cmd_flag,
};

pub static CMD_SET_ENVIRONMENT_ENTRY: cmd_entry = cmd_entry {
    name: "set-environment",
    alias: Some("setenv"),

    args: args_parse::new("Fhgrt:u", 1, 2, None),
    usage: "[-Fhgru] [-t target-session] variable [value]",

    target: cmd_entry_flag::new(
        b't',
        cmd_find_type::CMD_FIND_SESSION,
        cmd_find_flags::CMD_FIND_CANFAIL,
    ),

    flags: cmd_flag::CMD_AFTERHOOK,
    exec: CmdExec::Ctx(cmd_set_environment_exec),
    source: cmd_entry_flag::zeroed(),
};

/// Errors of set-environment and show-environment, with C's messages.
#[derive(Debug, thiserror::Error)]
pub enum EnvironError {
    #[error("empty variable name")]
    EmptyName,
    #[error("variable name contains =")]
    NameHasEquals,
    #[error("can't specify a value with -{0}")]
    ValueWith(char),
    #[error("no value specified")]
    NoValue,
    /// -t names no session.
    #[error("no such session: {0}")]
    NoSuchSession(String),
    #[error("no current session")]
    NoCurrentSession,
    #[error("unknown variable: {0}")]
    UnknownVariable(String),
}

/// The session whose environment a command changes or shows: None with
/// -g for the global environment, otherwise the target session.
pub fn environ_session(ctx: &CmdCtx<'_>) -> Result<Option<SessionId>, EnvironError> {
    if ctx.args().has('g') {
        return Ok(None);
    }
    match (ctx.target_session(), ctx.args().get('t')) {
        (Some(s), _) => Ok(Some(s.id)),
        (None, Some(t)) => Err(EnvironError::NoSuchSession(
            String::from_utf8_lossy(t).into_owned(),
        )),
        (None, None) => Err(EnvironError::NoCurrentSession),
    }
}

/// set-environment: set, remove (-r) or delete (-u) a variable of the
/// global (-g) or the target session's environment.
fn cmd_set_environment_exec(ctx: &mut CmdCtx<'_>) -> CmdResult {
    let args = ctx.args();
    let name = args.string(0).unwrap_or_default().to_vec();
    if name.is_empty() {
        return Err(EnvironError::EmptyName.into());
    }
    if name.contains(&b'=') {
        return Err(EnvironError::NameHasEquals.into());
    }
    let value = args.string(1);
    let session = environ_session(ctx)?;

    let change = if args.has('u') {
        if value.is_some() {
            return Err(EnvironError::ValueWith('u').into());
        }
        EnvironChange::Unset { name }
    } else if args.has('r') {
        if value.is_some() {
            return Err(EnvironError::ValueWith('r').into());
        }
        EnvironChange::Clear { name }
    } else {
        let value = value.ok_or(EnvironError::NoValue)?;
        EnvironChange::Set {
            name,
            value: value.to_vec(),
            format: args.has('F'),
            hidden: args.has('h'),
        }
    };
    ctx.change_environ(session, change);
    Ok(CmdFlow::Done)
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use super::*;
    use crate::cmd_::context::testing::{args, ctx, parts};
    use crate::cmd_::context::{CmdEffect, SessionInfo};
    use crate::environ::Environ;

    fn session_parts() -> crate::cmd_::context::CmdCtxParts {
        let mut p = parts();
        p.target_session = Some(SessionInfo {
            id: SessionId(3),
            environ: Rc::new(Environ::new()),
        });
        p
    }

    fn run(
        flags: &[(char, Option<&str>)],
        values: &[&str],
        p: crate::cmd_::context::CmdCtxParts,
    ) -> Result<(Option<SessionId>, EnvironChange), String> {
        let a = args(flags, values);
        let mut c = ctx(&a, p);
        cmd_set_environment_exec(&mut c).map_err(|e| e.to_string())?;
        match c.take_effects().pop() {
            Some(CmdEffect::Environ { session, change }) => Ok((session, change)),
            _ => panic!("no change recorded"),
        }
    }

    #[test]
    fn errors() {
        let err = |flags: &[(char, Option<&str>)], values: &[&str], p| run(flags, values, p).err().unwrap();
        assert_eq!(err(&[], &["", "x"], session_parts()), "empty variable name");
        assert_eq!(err(&[], &["A=B", "x"], session_parts()), "variable name contains =");
        assert_eq!(err(&[('u', None)], &["A", "x"], session_parts()), "can't specify a value with -u");
        assert_eq!(err(&[('r', None)], &["A", "x"], session_parts()), "can't specify a value with -r");
        assert_eq!(err(&[], &["A"], session_parts()), "no value specified");
        assert_eq!(err(&[], &["A", "x"], parts()), "no current session");
        assert_eq!(err(&[('t', Some("nosuch"))], &["A", "x"], parts()), "no such session: nosuch");
    }

    #[test]
    fn changes() {
        let (s, change) = run(&[('h', None), ('F', None)], &["A", "#{x}"], session_parts()).unwrap();
        assert_eq!(s, Some(SessionId(3)));
        assert!(matches!(
            change,
            EnvironChange::Set { name, value, format: true, hidden: true }
                if name == b"A" && value == b"#{x}"
        ));
        let (s, change) = run(&[('g', None), ('u', None)], &["A"], parts()).unwrap();
        assert_eq!(s, None);
        assert!(matches!(change, EnvironChange::Unset { name } if name == b"A"));
        let (_, change) = run(&[('r', None)], &["A"], session_parts()).unwrap();
        assert!(matches!(change, EnvironChange::Clear { name } if name == b"A"));
    }
}
