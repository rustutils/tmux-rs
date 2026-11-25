//! Parity tests against C tmux 3.8.
//!
//! Each test starts a fresh tmux-rs server and a fresh C tmux server, runs
//! the same command on both and compares the output. Differences that are
//! known and not fixed yet are listed in the `known` tables in
//! `parity_known.rs`, each with the roadmap item that will remove it. The
//! tests fail on any difference that is not listed, and also on a listed
//! difference that has gone away, so the tables only ever shrink.
//!
//! The C binary is /usr/bin/tmux, or `TMUX_REFERENCE_BIN` if set. The tests
//! are skipped when it is missing or not tmux 3.8. They always use the
//! cargo-built tmux-rs, whatever `TMUX_SERVER_BIN` and `TMUX_CLIENT_BIN` say.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "parity_known.rs"]
mod known;

static SERVER_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The C tmux 3.8 binary to compare against, or None to skip.
fn reference_bin() -> Option<PathBuf> {
    let bin = std::env::var_os("TMUX_REFERENCE_BIN")
        .map_or_else(|| PathBuf::from("/usr/bin/tmux"), PathBuf::from);
    let out = Command::new(&bin).arg("-V").output().ok()?;
    let version = String::from_utf8_lossy(&out.stdout);
    if version.trim() == "tmux 3.8" {
        Some(bin)
    } else {
        eprintln!(
            "skipping parity test: {} is {:?}, not tmux 3.8",
            bin.display(),
            version.trim()
        );
        None
    }
}

/// A server started with an empty config on its own socket, killed on drop.
struct Server {
    bin: PathBuf,
    socket: String,
}

impl Server {
    fn start(bin: PathBuf) -> Self {
        let id = SERVER_COUNTER.fetch_add(1, Ordering::Relaxed);
        let socket = format!("/tmp/tmux-parity-{}-{id}", std::process::id());
        let server = Self { bin, socket };
        let out = server.command(&["-f/dev/null", "new-session", "-d", "-x", "80", "-y", "24"]);
        assert!(
            out.status.success(),
            "{} failed to start",
            server.bin.display()
        );
        server
    }

    fn command(&self, args: &[&str]) -> std::process::Output {
        Command::new(&self.bin)
            .args(["-S", &self.socket])
            .args(args)
            .env("TERM", "screen")
            .env("SHELL", "/bin/sh")
            .env("HISTFILE", "/dev/null")
            .env_remove("ENV")
            .env_remove("TMUX")
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .env("LANG", "C.UTF-8")
            .env_remove("LC_ALL")
            .env_remove("LC_CTYPE")
            .env_remove("TMUX_CONF")
            .env_remove("TMUX_TMPDIR")
            .output()
            .expect("run tmux")
    }

    /// Run a command and return its standard output.
    fn run(&self, args: &[&str]) -> String {
        String::from_utf8_lossy(&self.command(args).stdout).into_owned()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.command(&["kill-server"]);
    }
}

/// Start a tmux-rs server and a C tmux server, or None if there is no C
/// tmux 3.8 to compare with.
fn servers() -> Option<(Server, Server)> {
    let reference = reference_bin()?;
    let rust = Server::start(env!("CARGO_BIN_EXE_tmux-rs").into());
    let c = Server::start(reference);
    Some((rust, c))
}

/// Compare two maps and describe each difference as a key: "missing:K"
/// when only C has K, "extra:K" when only tmux-rs has it and "differs:K"
/// when the values are not equal.
fn differences(rust: &BTreeMap<String, String>, c: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (k, v) in c {
        match rust.get(k) {
            None => {
                out.insert(format!("missing:{k}"));
            }
            Some(rv) if rv != v => {
                out.insert(format!("differs:{k}"));
            }
            Some(_) => {}
        }
    }
    for k in rust.keys() {
        if !c.contains_key(k) {
            out.insert(format!("extra:{k}"));
        }
    }
    out
}

/// Fail unless the actual differences are exactly the known ones. The
/// message lists new differences with both values, and listed ones that no
/// longer happen so they can be removed from the table.
fn check(
    what: &str,
    actual: &BTreeSet<String>,
    known: &[(&str, &str)],
    rust: &BTreeMap<String, String>,
    c: &BTreeMap<String, String>,
) {
    let listed: BTreeSet<String> = known.iter().map(|(k, _)| (*k).to_string()).collect();
    assert_eq!(
        listed.len(),
        known.len(),
        "{what}: duplicate entries in the known table"
    );
    let mut msg = String::new();
    for d in actual.difference(&listed) {
        let key = d.split_once(':').map_or(d.as_str(), |(_, k)| k);
        msg.push_str(&format!(
            "new difference {d:?}\n    tmux-rs: {:?}\n    tmux 3.8: {:?}\n",
            rust.get(key),
            c.get(key)
        ));
    }
    for d in listed.difference(actual) {
        msg.push_str(&format!("fixed, remove from the known table: {d:?}\n"));
    }
    assert!(msg.is_empty(), "{what} differ from tmux 3.8:\n{msg}");
}

/// Parse `show-options` output into option name and value.
fn parse_options(out: &str) -> BTreeMap<String, String> {
    out.lines()
        .filter_map(|l| l.split_once(' ').or(Some((l, ""))))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn options_match_tmux_3_8() {
    let Some((rust, c)) = servers() else { return };
    let mut r = BTreeMap::new();
    let mut cc = BTreeMap::new();
    for scope in ["-s", "-g", "-gw"] {
        for (k, v) in parse_options(&rust.run(&["show-options", scope])) {
            r.insert(format!("{scope} {k}"), v);
        }
        for (k, v) in parse_options(&c.run(&["show-options", scope])) {
            cc.insert(format!("{scope} {k}"), v);
        }
    }
    let actual = differences(&r, &cc);
    check("options", &actual, known::OPTIONS, &r, &cc);
}

/// The hook options, in order. The options test does not see them,
/// because show-options leaves hooks out.
#[test]
fn hooks_match_tmux_3_8() {
    let Some((rust, c)) = servers() else { return };
    for scope in ["-g", "-gw"] {
        assert_eq!(
            rust.run(&["show-hooks", scope]),
            c.run(&["show-hooks", scope]),
            "show-hooks {scope}"
        );
    }
}

#[test]
fn commands_match_tmux_3_8() {
    let Some((rust, c)) = servers() else { return };
    let parse = |out: String| -> BTreeMap<String, String> {
        out.lines()
            .map(|l| {
                let name = l.split_whitespace().next().unwrap_or("").to_string();
                (name, l.to_string())
            })
            .collect()
    };
    let r = parse(rust.run(&["list-commands"]));
    let cc = parse(c.run(&["list-commands"]));
    let actual = differences(&r, &cc);
    check("commands", &actual, known::COMMANDS, &r, &cc);
}

#[test]
fn key_bindings_match_tmux_3_8() {
    let Some((rust, c)) = servers() else { return };
    // "bind-key [-r] -T table key command": key the map by table and key,
    // and compare the repeat flag and command with whitespace collapsed.
    let parse = |out: String| -> BTreeMap<String, String> {
        out.lines()
            .filter_map(|l| {
                let mut words = l.split_whitespace().peekable();
                words.next()?; // bind-key
                let repeat = words.next_if_eq(&"-r").is_some();
                words.next()?; // -T
                let table = words.next()?;
                let key = words.next()?;
                let command: Vec<&str> = words.collect();
                let value = format!("{}{}", if repeat { "-r " } else { "" }, command.join(" "));
                Some((format!("{table} {key}"), value))
            })
            .collect()
    };
    let r = parse(rust.run(&["list-keys"]));
    let cc = parse(c.run(&["list-keys"]));
    let actual = differences(&r, &cc);
    check("key bindings", &actual, known::KEY_BINDINGS, &r, &cc);
}
