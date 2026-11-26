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

//! Environments, environ.c from tmux 3.8 in safe Rust.
//!
//! An [`Environ`] is a set of variables in name order. The global
//! environment, each session and each client have one, and spawning a
//! process builds one from them ([`Environ::for_process`]).
//!
//! An entry either has a value or is removed: a removed entry (set with
//! `set-environment -r`, or by `update-environment` for a variable the
//! client does not have) masks the variable when environments are copied
//! over each other, and never reaches a process.
//!
//! Names and values are bytes. A client sends its whole environment and
//! POSIX allows any bytes but `=` and NUL in a name, so nothing here
//! assumes UTF-8. Names compare bytewise, which is C's strcmp order.
//!
//! The store reads no options and no process state. The update-environment
//! patterns, the pattern matcher and the values tmux adds for a new
//! process come in as arguments.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

bitflags::bitflags! {
    /// Flags of an environment entry.
    #[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
    pub struct EnvironFlags: u32 {
        /// Hidden from show-environment without -h and from processes
        /// (set-environment -h, %hidden in a config file).
        const HIDDEN = 0x1;
    }
}

/// One variable of an [`Environ`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvironEntry {
    name: Vec<u8>,
    value: Option<Vec<u8>>,
    flags: EnvironFlags,
}

impl EnvironEntry {
    /// The variable name.
    pub fn name(&self) -> &[u8] {
        &self.name
    }

    /// The value, or None for a removed variable.
    pub fn value(&self) -> Option<&[u8]> {
        self.value.as_deref()
    }

    /// Whether the entry is hidden.
    pub fn is_hidden(&self) -> bool {
        self.flags.contains(EnvironFlags::HIDDEN)
    }
}

/// What tmux adds to the environment of a new process, besides the global
/// and session environments ([`Environ::for_process`]).
#[derive(Clone, Debug)]
pub struct ProcessVars<'a> {
    /// The default-terminal option for TERM, or None to leave TERM and the
    /// variables that go with it out (commands run while the config file
    /// is loading).
    pub term: Option<&'a [u8]>,
    /// The tmux version, for `TERM_PROGRAM_VERSION`.
    pub version: &'a [u8],
    /// The value of TMUX: socket path, server PID and session ID.
    pub tmux: &'a [u8],
}

/// A set of environment variables in name order (environ.c's struct
/// environ).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Environ {
    entries: BTreeMap<Vec<u8>, EnvironEntry>,
}

impl Environ {
    /// An empty environment.
    pub fn new() -> Self {
        Self::default()
    }

    /// The entries in name order.
    pub fn iter(&self) -> impl Iterator<Item = &EnvironEntry> {
        self.entries.values()
    }

    /// The entry called `name`.
    pub fn find(&self, name: &[u8]) -> Option<&EnvironEntry> {
        self.entries.get(name)
    }

    /// The value of `name`, None when it is missing or removed.
    pub fn value(&self, name: &[u8]) -> Option<&[u8]> {
        self.find(name)?.value()
    }

    /// Sets `name` to `value` with `flags` (`environ_set`).
    pub fn set(&mut self, name: &[u8], value: impl Into<Vec<u8>>, flags: EnvironFlags) {
        let value = Some(value.into());
        match self.entries.get_mut(name) {
            Some(entry) => {
                entry.value = value;
                entry.flags = flags;
            }
            None => {
                self.entries.insert(
                    name.to_vec(),
                    EnvironEntry {
                        name: name.to_vec(),
                        value,
                        flags,
                    },
                );
            }
        }
    }

    /// Marks `name` as removed, keeping its flags if it exists
    /// (`environ_clear`, set-environment -r).
    pub fn clear(&mut self, name: &[u8]) {
        match self.entries.get_mut(name) {
            Some(entry) => entry.value = None,
            None => {
                self.entries.insert(
                    name.to_vec(),
                    EnvironEntry {
                        name: name.to_vec(),
                        value: None,
                        flags: EnvironFlags::empty(),
                    },
                );
            }
        }
    }

    /// Deletes `name` (`environ_unset`, set-environment -u).
    pub fn unset(&mut self, name: &[u8]) {
        self.entries.remove(name);
    }

    /// Sets a variable from `NAME=VALUE` (`environ_put`). The name ends at
    /// the first `=`. Without `=` nothing is set.
    pub fn put(&mut self, var: &[u8], flags: EnvironFlags) {
        if let Some(eq) = var.iter().position(|&b| b == b'=') {
            self.set(&var[..eq], &var[eq + 1..], flags);
        }
    }

    /// Copies every entry of `src` over this one (`environ_copy`). Removed
    /// entries remove, the others keep their flags.
    pub fn copy_from(&mut self, src: &Environ) {
        for entry in src.iter() {
            match &entry.value {
                Some(value) => self.set(&entry.name, value.as_slice(), entry.flags),
                None => self.clear(&entry.name),
            }
        }
    }

    /// Applies the update-environment patterns (`environ_update`): every
    /// variable of `src` whose name matches a pattern is set here, and a
    /// pattern that matches nothing is marked removed under its own name.
    /// `matches(pattern, name)` is the caller's fnmatch.
    pub fn update<'p>(
        &mut self,
        patterns: impl IntoIterator<Item = &'p [u8]>,
        src: &Environ,
        matches: impl Fn(&[u8], &[u8]) -> bool,
    ) {
        for pattern in patterns {
            let mut found = false;
            for entry in src.iter() {
                if matches(pattern, &entry.name) {
                    if let Some(value) = &entry.value {
                        self.set(&entry.name, value.as_slice(), EnvironFlags::empty());
                    }
                    found = true;
                }
            }
            if !found {
                self.clear(pattern);
            }
        }
    }

    /// The environment of a new process (`environ_for_session`): `global`,
    /// then `session` over it, then TERM, `TERM_PROGRAM`,
    /// `TERM_PROGRAM_VERSION` and COLORTERM unless `vars.term` is None, then
    /// TMUX.
    pub fn for_process(global: &Environ, session: Option<&Environ>, vars: &ProcessVars<'_>) -> Self {
        let mut env = global.clone();
        if let Some(session) = session {
            env.copy_from(session);
        }
        if let Some(term) = vars.term {
            env.set(b"TERM", term, EnvironFlags::empty());
            env.set(b"TERM_PROGRAM", b"tmux".as_slice(), EnvironFlags::empty());
            env.set(b"TERM_PROGRAM_VERSION", vars.version, EnvironFlags::empty());
            env.set(b"COLORTERM", b"truecolor".as_slice(), EnvironFlags::empty());
        }
        #[cfg(feature = "systemd")]
        {
            env.clear(b"LISTEN_PID");
            env.clear(b"LISTEN_FDS");
            env.clear(b"LISTEN_FDNAMES");
        }
        env.set(b"TMUX", vars.tmux, EnvironFlags::empty());
        env
    }

    /// The variables a process gets (what `environ_push` sets): the entries
    /// with a value and a name, except hidden ones, in name order. The
    /// process environment must hold exactly these.
    pub fn process_vars(&self) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.iter().filter_map(|entry| match &entry.value {
            Some(value) if !entry.name.is_empty() && !entry.is_hidden() => {
                Some((entry.name.as_slice(), value.as_slice()))
            }
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vals(env: &Environ) -> Vec<(String, Option<String>)> {
        env.iter()
            .map(|e| {
                (
                    String::from_utf8_lossy(e.name()).into_owned(),
                    e.value().map(|v| String::from_utf8_lossy(v).into_owned()),
                )
            })
            .collect()
    }

    fn s(name: &str, value: Option<&str>) -> (String, Option<String>) {
        (name.to_string(), value.map(str::to_string))
    }

    #[test]
    fn set_find_and_replace() {
        let mut env = Environ::new();
        env.set(b"B", "1", EnvironFlags::HIDDEN);
        env.set(b"A", "2", EnvironFlags::empty());
        env.set(b"B", "3", EnvironFlags::empty());
        assert_eq!(vals(&env), [s("A", Some("2")), s("B", Some("3"))]);
        assert!(!env.find(b"B").unwrap().is_hidden());
        assert_eq!(env.value(b"A"), Some(b"2".as_slice()));
        assert!(env.find(b"C").is_none());
    }

    #[test]
    fn clear_keeps_flags_and_masks() {
        let mut env = Environ::new();
        env.set(b"H", "x", EnvironFlags::HIDDEN);
        env.clear(b"H");
        env.clear(b"NEW");
        let h = env.find(b"H").unwrap();
        assert_eq!((h.value(), h.is_hidden()), (None, true));
        assert!(!env.find(b"NEW").unwrap().is_hidden());
        assert_eq!(env.value(b"H"), None);
    }

    #[test]
    fn unset_removes() {
        let mut env = Environ::new();
        env.set(b"A", "1", EnvironFlags::empty());
        env.unset(b"A");
        env.unset(b"MISSING");
        assert_eq!(env, Environ::new());
    }

    #[test]
    fn put_splits_at_the_first_equals() {
        let mut env = Environ::new();
        env.put(b"A=b=c", EnvironFlags::empty());
        env.put(b"NOEQUALS", EnvironFlags::empty());
        env.put(b"E=", EnvironFlags::HIDDEN);
        env.put(b"BAD\xffNAME=\xfe", EnvironFlags::empty());
        assert_eq!(env.value(b"A"), Some(b"b=c".as_slice()));
        assert!(env.find(b"NOEQUALS").is_none());
        assert!(env.find(b"E").unwrap().is_hidden());
        assert_eq!(env.value(b"BAD\xffNAME"), Some(b"\xfe".as_slice()));
    }

    #[test]
    fn names_sort_bytewise() {
        let mut env = Environ::new();
        for name in [b"b".as_slice(), b"B", b"\xff", b"a_"] {
            env.set(name, "", EnvironFlags::empty());
        }
        let names: Vec<&[u8]> = env.iter().map(EnvironEntry::name).collect();
        assert_eq!(names, [b"B".as_slice(), b"a_", b"b", b"\xff"]);
    }

    #[test]
    fn copy_from_sets_and_masks() {
        let mut src = Environ::new();
        src.set(b"A", "src", EnvironFlags::HIDDEN);
        src.clear(b"B");
        let mut dst = Environ::new();
        dst.set(b"A", "dst", EnvironFlags::empty());
        dst.set(b"B", "dst", EnvironFlags::empty());
        dst.set(b"C", "dst", EnvironFlags::empty());
        dst.copy_from(&src);
        assert_eq!(
            vals(&dst),
            [s("A", Some("src")), s("B", None), s("C", Some("dst"))]
        );
        assert!(dst.find(b"A").unwrap().is_hidden());
    }

    #[test]
    fn update_copies_matches_and_removes_the_rest() {
        let mut client = Environ::new();
        client.set(b"DISPLAY", ":1", EnvironFlags::HIDDEN);
        client.set(b"SSH_AUTH_SOCK", "/s", EnvironFlags::empty());
        client.set(b"SSH_AGENT_PID", "7", EnvironFlags::empty());
        let mut session = Environ::new();
        session.set(b"KEEP", "k", EnvironFlags::empty());
        let patterns: [&[u8]; 3] = [b"DISPLAY", b"SSH_*", b"XAUTHORITY"];
        // A prefix matcher stands in for fnmatch.
        let matches = |p: &[u8], n: &[u8]| match p.strip_suffix(b"*") {
            Some(prefix) => n.starts_with(prefix),
            None => p == n,
        };
        session.update(patterns, &client, matches);
        assert_eq!(
            vals(&session),
            [
                s("DISPLAY", Some(":1")),
                s("KEEP", Some("k")),
                s("SSH_AGENT_PID", Some("7")),
                s("SSH_AUTH_SOCK", Some("/s")),
                s("XAUTHORITY", None),
            ]
        );
        // Copied entries lose the hidden flag, as C sets them with 0.
        assert!(!session.find(b"DISPLAY").unwrap().is_hidden());
    }

    #[test]
    fn process_environment() {
        let mut global = Environ::new();
        global.set(b"G", "g", EnvironFlags::empty());
        global.set(b"BOTH", "global", EnvironFlags::empty());
        global.set(b"TERM", "old", EnvironFlags::empty());
        let mut session = Environ::new();
        session.set(b"BOTH", "session", EnvironFlags::empty());
        session.clear(b"G");
        let vars = ProcessVars {
            term: Some(b"tmux-256color"),
            version: b"3.8",
            tmux: b"/tmp/s,1,0",
        };
        let env = Environ::for_process(&global, Some(&session), &vars);
        assert_eq!(
            vals(&env),
            [
                s("BOTH", Some("session")),
                s("COLORTERM", Some("truecolor")),
                s("G", None),
                s("TERM", Some("tmux-256color")),
                s("TERM_PROGRAM", Some("tmux")),
                s("TERM_PROGRAM_VERSION", Some("3.8")),
                s("TMUX", Some("/tmp/s,1,0")),
            ]
        );
        let no_term = ProcessVars { term: None, ..vars };
        let env = Environ::for_process(&global, None, &no_term);
        assert_eq!(env.value(b"TERM"), Some(b"old".as_slice()));
        assert!(env.find(b"COLORTERM").is_none());
        assert!(env.find(b"TERM_PROGRAM").is_none());
    }

    #[test]
    fn process_vars_skip_removed_hidden_and_empty_names() {
        let mut env = Environ::new();
        env.set(b"A", "1", EnvironFlags::empty());
        env.set(b"H", "2", EnvironFlags::HIDDEN);
        env.clear(b"R");
        env.put(b"=x", EnvironFlags::empty());
        let vars: Vec<_> = env.process_vars().collect();
        assert_eq!(vars, [(b"A".as_slice(), b"1".as_slice())]);
    }
}
