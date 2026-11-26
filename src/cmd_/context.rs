// Copyright (c) 2026 tmux-rs contributors
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
//! The command context: what a converted command works with.
//!
//! A converted command is a safe function, [`CmdExecFn`], that takes a
//! [`CmdCtx`] and returns a [`CmdResult`]. The context gives it its
//! arguments ([`CmdArgs`]), its client and targets as plain values
//! ([`ClientInfo`], [`PaneInfo`], [`SessionInfo`]), the paste buffers
//! ([`PasteAccess`]) and a copy of the global environment.
//!
//! Everything a command wants done to unconverted parts of tmux (printing,
//! writing to a pane, setting a client's clipboard, raising notifications
//! for changed buffers) is recorded as a [`CmdEffect`]. The dispatcher in
//! `run.rs` carries the effects out in order once the command returns,
//! whether it succeeded or not, since in C what happened before an error
//! stays done. An error is printed as `cmdq_error` prints it, so its
//! Display text is the C message.
//!
//! A command that has to wait for a client file returns a [`CmdFlow`]
//! naming the file and what to do with the result. The dispatcher starts
//! the read or write and calls the continuation with a new context when
//! the file is done.

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::environ::Environ;
use crate::global::{with_global_environ, with_message_log, with_paste, with_prompt_history};
use crate::message_log::Message;
use crate::paste::{PasteBuffer, PasteError, PasteEvent};
use crate::prompt_history::PromptType;
use crate::sort::SortCriteria;
use crate::sort_get::sort_buffers;
use crate::{ClientId, PaneId, SessionId, args, args_value};

/// What a converted command returns.
pub type CmdResult = Result<CmdFlow, Box<dyn std::error::Error>>;

/// The exec function of a converted command.
pub type CmdExecFn = fn(&mut CmdCtx<'_>) -> CmdResult;

/// What runs when a client file is done: the context of that moment and
/// the outcome.
pub type FileThen = Box<dyn FnOnce(&mut CmdCtx<'_>, FileDone) -> CmdResult>;

/// How a command goes on after its exec function returns.
pub enum CmdFlow {
    /// The command is finished.
    Done,
    /// Read a client file, then call `then` with its contents. The
    /// command waits until then.
    ReadFile { path: FilePath, then: FileThen },
    /// Write `data` to a client file, appending or truncating, then call
    /// `then`. The command waits until then.
    WriteFile {
        path: FilePath,
        append: bool,
        data: Rc<[u8]>,
        then: FileThen,
    },
}

/// The path of a client file.
pub enum FilePath {
    /// Used as it is ("-" is the client's standard input or output).
    Literal(Vec<u8>),
    /// A format, expanded for the command's target first.
    Format(Vec<u8>),
}

/// The outcome of reading or writing a client file: what was read
/// (empty after a write), or the error.
pub type FileDone = Result<Vec<u8>, FileError>;

/// A client file that could not be read or written. Shown as tmux 3.8
/// does, the error first and the path last.
#[derive(Debug, thiserror::Error)]
#[error("{message}: {path}")]
pub struct FileError {
    /// The strerror text.
    pub message: String,
    /// The path the client used.
    pub path: String,
}

/// Work for unconverted parts of tmux, done by the dispatcher after the
/// command returns, in the order it was recorded.
pub enum CmdEffect {
    /// A paste buffer changed or was deleted, which raises a
    /// notification.
    Paste(PasteEvent),
    /// Output for the command's client, shown as it is.
    Print(Vec<u8>),
    /// Expand `template` for `subject` and print it, if `filter` is
    /// absent or expands to true.
    PrintFormat {
        template: Vec<u8>,
        filter: Option<Vec<u8>>,
        subject: FormatSubject,
    },
    /// Set the clipboard of a client's terminal, if the client is still
    /// attached.
    SetSelection { client: ClientId, data: Rc<[u8]> },
    /// Write bytes to a pane's program.
    WritePane { pane: PaneId, data: Vec<u8> },
    /// Print the terminals in use (show-messages -T), only the one of
    /// `terminal_client` if given, then the jobs (-J), with a blank line
    /// between when both print something.
    PrintSummaries {
        terminals: bool,
        terminal_client: Option<ClientId>,
        jobs: bool,
    },
    /// Change a variable of the global environment (`session` None) or
    /// of a session's.
    Environ {
        session: Option<SessionId>,
        change: EnvironChange,
    },
}

/// A change to one environment variable.
pub enum EnvironChange {
    /// Set `name`. With `format` the value is a format, expanded for the
    /// command's target first (set-environment -F).
    Set {
        name: Vec<u8>,
        value: Vec<u8>,
        format: bool,
        hidden: bool,
    },
    /// Mark `name` as removed (-r).
    Clear { name: Vec<u8> },
    /// Delete `name` (-u).
    Unset { name: Vec<u8> },
}

/// What a format is expanded for, which decides the format variables.
pub enum FormatSubject {
    /// A paste buffer (the buffer_* variables).
    PasteBuffer(PasteBuffer),
    /// A logged message (the message_* variables), with the command's
    /// target as well.
    Message(Message),
}

/// A client, as a command sees it.
#[derive(Clone, Copy, Debug)]
pub struct ClientInfo {
    pub id: ClientId,
    /// Whether it is attached to a session.
    pub attached: bool,
    /// Whether it is a control mode client.
    pub control: bool,
}

/// A pane, as a command sees it.
#[derive(Clone, Copy, Debug)]
pub struct PaneInfo {
    pub id: PaneId,
    /// Whether its program has exited.
    pub exited: bool,
    /// Whether input to it is turned off (select-pane -d).
    pub input_off: bool,
    /// Whether its program asked for bracketed paste.
    pub bracket_paste: bool,
}

/// A session, as a command sees it.
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: SessionId,
    /// Its environment, shared with the session until either changes.
    pub environ: Rc<Environ>,
}

/// A command's parsed arguments.
#[derive(Clone, Copy)]
pub struct CmdArgs<'a> {
    args: &'a args,
}

impl<'a> CmdArgs<'a> {
    /// Wraps parsed arguments. Command blocks among the values must have
    /// been printed already (see `args_value_as_string`), or they read as
    /// missing.
    pub fn new(args: &'a args) -> Self {
        Self { args }
    }

    /// Whether `flag` was given.
    pub fn has(self, flag: char) -> bool {
        debug_assert!(flag.is_ascii());
        self.args
            .tree
            .get(&(flag as u8))
            .is_some_and(|entry| entry.count != 0)
    }

    /// The last value given with `flag`, as `args_get`.
    pub fn get(self, flag: char) -> Option<&'a [u8]> {
        debug_assert!(flag.is_ascii());
        match self.args.tree.get(&(flag as u8))?.values.last()? {
            args_value::String { string } => Some(string.to_bytes()),
            _ => None,
        }
    }

    /// The number of arguments after the flags.
    pub fn count(self) -> usize {
        self.args.values.len()
    }

    /// Argument `idx` after the flags, as `args_string`.
    pub fn string(self, idx: usize) -> Option<&'a [u8]> {
        match self.args.values.get(idx)? {
            args_value::None => Some(b""),
            args_value::String { string } => Some(string.to_bytes()),
            args_value::Commands { cached, .. } => {
                debug_assert!(cached.get().is_some(), "command block not printed");
                cached.get().map(|c| c.to_bytes())
            }
        }
    }
}

/// The context a converted command runs in.
pub struct CmdCtx<'a> {
    args: CmdArgs<'a>,
    client: Option<ClientInfo>,
    target_client: Option<ClientInfo>,
    target_pane: Option<PaneInfo>,
    target_session: Option<SessionInfo>,
    now: i64,
    buffer_limit: u32,
    effects: Vec<CmdEffect>,
}

/// The values a context is built from, collected by the dispatcher.
pub struct CmdCtxParts {
    /// The client the command came from.
    pub client: Option<ClientInfo>,
    /// The client found for -t or -c, or the best client.
    pub target_client: Option<ClientInfo>,
    /// The pane of the command's target.
    pub target_pane: Option<PaneInfo>,
    /// The session of the command's target.
    pub target_session: Option<SessionInfo>,
    /// Seconds since the epoch.
    pub now: i64,
    /// The buffer-limit option.
    pub buffer_limit: u32,
}

impl<'a> CmdCtx<'a> {
    pub fn new(args: CmdArgs<'a>, parts: CmdCtxParts) -> Self {
        Self {
            args,
            client: parts.client,
            target_client: parts.target_client,
            target_pane: parts.target_pane,
            target_session: parts.target_session,
            now: parts.now,
            buffer_limit: parts.buffer_limit,
            effects: Vec::new(),
        }
    }

    /// The command's arguments.
    pub fn args(&self) -> CmdArgs<'a> {
        self.args
    }

    /// The client the command came from, if any (commands from the
    /// configuration have none).
    pub fn client(&self) -> Option<ClientInfo> {
        self.client
    }

    /// The target client, for commands that have one.
    pub fn target_client(&self) -> Option<ClientInfo> {
        self.target_client
    }

    /// The target pane, for commands that target one.
    pub fn target_pane(&self) -> Option<PaneInfo> {
        self.target_pane
    }

    /// Show `data` to the command's client.
    pub fn print(&mut self, data: impl Into<Vec<u8>>) {
        self.effects.push(CmdEffect::Print(data.into()));
    }

    /// Show `template` expanded for `subject`, if `filter` is `None` or
    /// expands to true.
    pub fn print_format(&mut self, template: &[u8], filter: Option<&[u8]>, subject: FormatSubject) {
        self.effects.push(CmdEffect::PrintFormat {
            template: template.to_vec(),
            filter: filter.map(<[u8]>::to_vec),
            subject,
        });
    }

    /// Set the clipboard of `client`'s terminal to `data`.
    pub fn set_selection(&mut self, client: ClientId, data: Rc<[u8]>) {
        self.effects.push(CmdEffect::SetSelection { client, data });
    }

    /// Write `data` to the program in `pane`.
    pub fn write_pane(&mut self, pane: PaneId, data: Vec<u8>) {
        self.effects.push(CmdEffect::WritePane { pane, data });
    }

    /// The paste buffers.
    pub fn paste(&mut self) -> PasteAccess<'_> {
        PasteAccess {
            limit: self.buffer_limit,
            now: self.now,
            effects: &mut self.effects,
        }
    }

    /// The prompt history.
    pub fn prompt_history(&self) -> PromptHistoryAccess {
        PromptHistoryAccess
    }

    /// The session of the command's target, if it has one.
    pub fn target_session(&self) -> Option<&SessionInfo> {
        self.target_session.as_ref()
    }

    /// The logged messages, newest first.
    pub fn messages(&self) -> Vec<Message> {
        with_message_log(|log| log.iter().cloned().collect())
    }

    /// A copy of the global environment.
    pub fn global_environ(&self) -> Environ {
        with_global_environ(|env| env.clone())
    }

    /// Changes a variable of the global environment (`session` None) or
    /// of a session's environment, once the command returns.
    pub fn change_environ(&mut self, session: Option<SessionId>, change: EnvironChange) {
        self.effects.push(CmdEffect::Environ { session, change });
    }

    /// Prints the terminals in use (`terminals`, only the one of
    /// `terminal_client` if given) and the jobs, once the command returns.
    pub fn print_summaries(&mut self, terminals: bool, terminal_client: Option<ClientId>, jobs: bool) {
        self.effects.push(CmdEffect::PrintSummaries {
            terminals,
            terminal_client,
            jobs,
        });
    }

    /// Takes the recorded effects, oldest first.
    pub fn take_effects(&mut self) -> Vec<CmdEffect> {
        std::mem::take(&mut self.effects)
    }
}

/// The prompt history, for a command.
pub struct PromptHistoryAccess;

impl PromptHistoryAccess {
    /// The entries of `ty`, oldest first.
    pub fn entries(&self, ty: PromptType) -> Vec<Vec<u8>> {
        with_prompt_history(|h| h.entries(ty).to_vec())
    }

    /// Delete all entries of `ty`.
    pub fn clear(&self, ty: PromptType) {
        with_prompt_history(|h| h.clear(ty));
    }
}

/// The paste buffers, for a command. Changes are recorded as effects so
/// the dispatcher raises their notifications.
pub struct PasteAccess<'c> {
    limit: u32,
    now: i64,
    effects: &'c mut Vec<CmdEffect>,
}

impl PasteAccess<'_> {
    /// The buffer called `name`. Names that are not UTF-8 find nothing,
    /// no buffer can have one.
    pub fn get(self, name: &[u8]) -> Option<PasteBuffer> {
        let name = std::str::from_utf8(name).ok()?;
        with_paste(|store| store.get(name).cloned())
    }

    /// The newest automatic buffer.
    pub fn top(&self) -> Option<PasteBuffer> {
        with_paste(|store| store.top().cloned())
    }

    /// All buffers, sorted by `criteria` (newest first without -O).
    pub fn sorted(&self, criteria: &SortCriteria) -> Vec<PasteBuffer> {
        let buffers = with_paste(|store| store.iter().cloned().collect());
        sort_buffers(buffers, criteria)
    }

    /// Set buffer `name` to `data`, or add an automatic buffer if `name`
    /// is `None`.
    pub fn set(&mut self, name: Option<&[u8]>, data: Rc<[u8]>) -> Result<(), PasteError> {
        let (limit, now) = (self.limit, self.now);
        let events = with_paste(|store| store.set(name, data, limit, now))?;
        self.effects.extend(events.into_iter().map(CmdEffect::Paste));
        Ok(())
    }

    /// Rename buffer `old` to `new`.
    pub fn rename(&mut self, old: &str, new: &[u8]) -> Result<(), PasteError> {
        let events = with_paste(|store| store.rename(old, new))?;
        self.effects.extend(events.into_iter().map(CmdEffect::Paste));
        Ok(())
    }

    /// Delete buffer `name`. Returns false if there is none.
    pub fn delete(&mut self, name: &str) -> bool {
        let event = with_paste(|store| store.delete(name));
        let found = event.is_some();
        self.effects.extend(event.map(CmdEffect::Paste));
        found
    }
}

/// Helpers for testing converted commands without a server.
#[cfg(test)]
pub(crate) mod testing {
    use std::collections::BTreeMap;
    use std::ffi::CString;

    use super::{CmdArgs, CmdCtx, CmdCtxParts, CmdEffect};
    use crate::paste::PasteEvent;
    use crate::{args, args_entry, args_value};

    /// Parsed arguments: `flags` are flags with an optional value, then
    /// `values` after them.
    pub fn args(flags: &[(char, Option<&str>)], values: &[&str]) -> args {
        let mut tree = BTreeMap::new();
        for &(flag, value) in flags {
            let entry = tree.entry(flag as u8).or_insert_with(|| {
                Box::new(args_entry {
                    flag: flag as u8,
                    values: Vec::new(),
                    count: 0,
                    flags: 0,
                })
            });
            entry.count += 1;
            if let Some(value) = value {
                entry.values.push(args_value::String {
                    string: CString::new(value).unwrap(),
                });
            }
        }
        let values = values
            .iter()
            .map(|v| args_value::String {
                string: CString::new(*v).unwrap(),
            })
            .collect();
        args { tree, values }
    }

    /// Context parts with no client or target, buffer-limit 50 and time
    /// 1000.
    pub fn parts() -> CmdCtxParts {
        CmdCtxParts {
            client: None,
            target_client: None,
            target_pane: None,
            target_session: None,
            now: 1000,
            buffer_limit: 50,
        }
    }

    /// A context for `args`.
    pub fn ctx(args: &args, parts: CmdCtxParts) -> CmdCtx<'_> {
        CmdCtx::new(CmdArgs::new(args), parts)
    }

    /// The paste events among `effects`.
    pub fn paste_events(effects: &[CmdEffect]) -> Vec<PasteEvent> {
        effects
            .iter()
            .filter_map(|e| match e {
                CmdEffect::Paste(event) => Some(event.clone()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{args, ctx, parts, paste_events};
    use super::*;

    #[test]
    fn args_accessors() {
        let a = args(&[('a', None), ('b', Some("x")), ('b', Some("y"))], &["one", ""]);
        let args = CmdArgs::new(&a);
        assert!(args.has('a'));
        assert!(args.has('b'));
        assert!(!args.has('c'));
        assert_eq!(args.get('b'), Some(&b"y"[..]), "the last value counts");
        assert_eq!(args.get('a'), None);
        assert_eq!(args.get('c'), None);
        assert_eq!(args.count(), 2);
        assert_eq!(args.string(0), Some(&b"one"[..]));
        assert_eq!(args.string(1), Some(&b""[..]));
        assert_eq!(args.string(2), None);
    }

    #[test]
    fn paste_changes_are_recorded_in_order() {
        let a = args(&[], &[]);
        let mut ctx = ctx(&a, parts());
        ctx.paste().set(Some(b"one"), Rc::from(&b"1"[..])).unwrap();
        ctx.print("between");
        ctx.paste().rename("one", b"two").unwrap();
        assert!(ctx.paste().delete("two"));
        assert!(!ctx.paste().delete("two"));
        let effects = ctx.take_effects();
        assert!(matches!(effects[1], CmdEffect::Print(ref p) if p == b"between"));
        assert_eq!(
            paste_events(&effects),
            [
                PasteEvent::Changed("one".into()),
                PasteEvent::Deleted("one".into()),
                PasteEvent::Changed("two".into()),
                PasteEvent::Deleted("two".into()),
            ]
        );
    }

    #[test]
    fn paste_uses_the_limit_and_time_given() {
        let a = args(&[], &[]);
        let mut p = parts();
        p.buffer_limit = 1;
        p.now = 77;
        let mut ctx = ctx(&a, p);
        ctx.paste().set(None, Rc::from(&b"1"[..])).unwrap();
        ctx.paste().set(None, Rc::from(&b"2"[..])).unwrap();
        let all = ctx.paste().sorted(&SortCriteria::default());
        assert_eq!(all.len(), 1, "buffer-limit 1 keeps one");
        assert_eq!(&all[0].data()[..], b"2");
        assert_eq!(all[0].created(), 77);
        assert_eq!(ctx.paste().get(b"buffer1").map(|pb| pb.order()), Some(all[0].order()));
        assert_eq!(ctx.paste().get(b"bad\xff"), None);
    }

    #[test]
    fn file_errors_put_the_path_last() {
        let e = FileError {
            message: "No such file or directory".into(),
            path: "/x".into(),
        };
        assert_eq!(e.to_string(), "No such file or directory: /x");
    }
}
