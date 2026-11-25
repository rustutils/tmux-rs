//! Control mode format subscriptions (refresh-client -B), checked against
//! tmux 3.8.
//!
//! The server checks subscriptions once a second and writes
//! `%subscription-changed name $session @window index %pane : value`
//! when a value changes. The control client here reads on a thread, so
//! tests can also check that nothing arrived within a time.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, channel};
use std::time::{Duration, Instant};

use harness::TmuxTestHarness;

use super::harness;

/// How long to wait for a subscription tick, with room for slow runners.
const TICK: Duration = Duration::from_secs(4);

struct Control {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Control {
    fn attach(tmux: &TmuxTestHarness) -> Self {
        let mut child = Command::new(harness::client_bin())
            .args(["-S", tmux.socket_path(), "-C", "attach"])
            .env("TERM", "screen")
            .env_remove("TMUX")
            .env_remove("VISUAL")
            .env_remove("EDITOR")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn control client");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        let control = Self {
            child,
            stdin,
            lines: rx,
        };
        control.until(|l| l.starts_with("%end"), TICK);
        control
    }

    fn send(&mut self, command: &str) {
        writeln!(self.stdin, "{command}").unwrap();
        self.stdin.flush().unwrap();
    }

    /// Lines until one matches, which is included. Panics on timeout.
    fn until(&self, pred: impl Fn(&str) -> bool, timeout: Duration) -> Vec<String> {
        let start = Instant::now();
        let mut out = Vec::new();
        loop {
            let left = timeout.saturating_sub(start.elapsed());
            match self.lines.recv_timeout(left) {
                Ok(line) => {
                    let done = pred(&line);
                    out.push(line);
                    if done {
                        return out;
                    }
                }
                Err(RecvTimeoutError::Timeout) => panic!("timed out, got:\n{}", out.join("\n")),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("client exited, got:\n{}", out.join("\n"))
                }
            }
        }
    }

    /// Runs a command and waits for its %end.
    fn run(&mut self, command: &str) -> Vec<String> {
        self.send(command);
        self.until(|l| l.starts_with("%end") || l.starts_with("%error"), TICK)
    }

    /// The subscription lines that arrive within `time`.
    fn changes_for(&self, time: Duration) -> Vec<String> {
        let start = Instant::now();
        let mut out = Vec::new();
        while let Some(left) = time.checked_sub(start.elapsed()) {
            match self.lines.recv_timeout(left) {
                Ok(line) if line.starts_with("%subscription-changed") => out.push(line),
                Ok(_) => {}
                Err(_) => break,
            }
        }
        out
    }

    /// Waits for the next subscription line.
    fn next_change(&self) -> String {
        self.until(|l| l.starts_with("%subscription-changed"), TICK)
            .pop()
            .unwrap()
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "detach");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args([
            "-s",
            "main",
            "-n",
            "main",
            "-x",
            "80",
            "-y",
            "24",
            "sleep 1000",
        ])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args(["set", "-g", "automatic-rename", "off"])
        .run()
        .assert_success();
    tmux
}

fn ids(tmux: &TmuxTestHarness) -> (String, String, String) {
    (
        tmux.query("#{session_id}"),
        tmux.query("#{window_id}"),
        tmux.query("#{pane_id}"),
    )
}

#[test]
fn session_subscription_reports_first_value_and_changes() {
    let tmux = server();
    let (s, _, _) = ids(&tmux);
    let mut c = Control::attach(&tmux);
    c.run("refresh-client -B 'name::#{session_name}'");
    assert_eq!(
        c.next_change(),
        format!("%subscription-changed name {s} - - - : main")
    );
    c.run("rename-session other");
    assert_eq!(
        c.next_change(),
        format!("%subscription-changed name {s} - - - : other")
    );
    // No change, no line.
    assert!(c.changes_for(Duration::from_millis(2500)).is_empty());
}

#[test]
fn removed_subscription_stops() {
    let tmux = server();
    let mut c = Control::attach(&tmux);
    c.run("refresh-client -B 'name::#{session_name}'");
    c.next_change();
    c.run("refresh-client -B name");
    c.run("rename-session other");
    assert!(c.changes_for(Duration::from_millis(2500)).is_empty());
}

#[test]
fn pane_and_window_subscriptions() {
    let tmux = server();
    let (s, w, p) = ids(&tmux);
    let mut c = Control::attach(&tmux);
    c.run(&format!("refresh-client -B 'p:{p}:#{{pane_id}}'"));
    assert_eq!(
        c.next_change(),
        format!("%subscription-changed p {s} {w} 0 {p} : {p}")
    );
    c.run(&format!("refresh-client -B 'w:{w}:#{{window_name}}'"));
    assert_eq!(
        c.next_change(),
        format!("%subscription-changed w {s} {w} 0 - : main")
    );
}

#[test]
fn all_panes_and_all_windows() {
    let tmux = server();
    tmux.cmd()
        .args(["split-window", "-d", "sleep 1000"])
        .run()
        .assert_success();
    let mut c = Control::attach(&tmux);
    c.run("refresh-client -B 'ap:%*:#{pane_index}'");
    let mut lines = vec![c.next_change(), c.next_change()];
    lines.sort();
    assert!(
        lines[0].ends_with(": 0") && lines[1].ends_with(": 1"),
        "{lines:?}"
    );
    c.run("refresh-client -B 'aw:@*:#{window_index}'");
    assert!(c.next_change().ends_with("- : 0"));
}

/// 3.8 checks session subscriptions first, then single panes and windows,
/// then all panes, then all windows. tmux-rs checked in name order.
#[test]
fn check_order_is_by_type() {
    let tmux = server();
    let (_, w, _) = ids(&tmux);
    let mut c = Control::attach(&tmux);
    // One command line, so both are added before the next check.
    c.run(&format!(
        "refresh-client -B 'a:@*:#{{window_index}}' -B 'b:{w}:x' -B 'c::#{{session_name}}'"
    ));
    let names: Vec<String> = (0..3)
        .map(|_| c.next_change().split(' ').nth(1).unwrap().to_string())
        .collect();
    assert_eq!(names, ["c", "b", "a"]);
}

/// A value that does not parse is taken as the name of a subscription to
/// remove, so `foo` keeps working (3.8 passes the whole value to
/// `control_remove_sub`). tmux-rs made it a session subscription.
#[test]
fn unparsable_value_is_ignored() {
    let tmux = server();
    let mut c = Control::attach(&tmux);
    c.run("refresh-client -B 'foo::#{session_name}'");
    assert!(c.next_change().ends_with(": main"));
    c.run("refresh-client -B 'foo:bogus:other'");
    c.run("rename-session renamed");
    assert!(c.next_change().ends_with(": renamed"));
}

/// Monitor formats do not run jobs.
#[test]
fn formats_do_not_run_jobs() {
    let tmux = server();
    let mut c = Control::attach(&tmux);
    c.run("refresh-client -B 'job::[#(echo ran)]'");
    let first = c.next_change();
    assert!(first.ends_with(": []"), "{first}");
    // A job would finish and change the value a tick later.
    assert!(c.changes_for(Duration::from_millis(2500)).is_empty());
}

/// Entries for windows that are no longer seen are dropped, so a window
/// that moves away and back is reported again.
#[test]
fn window_moving_back_is_reported_again() {
    let tmux = server();
    let mut c = Control::attach(&tmux);
    c.run("refresh-client -B 'aw:@*:#{window_name}'");
    assert!(c.next_change().contains(" 0 - : main"));
    c.run("move-window -t 5");
    assert!(c.next_change().contains(" 5 - : main"));
    c.run("move-window -t 0");
    assert!(c.next_change().contains(" 0 - : main"));
}
