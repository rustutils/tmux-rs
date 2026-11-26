//! The server message log and show-messages, checked against tmux 3.8.

use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux
}

/// The message texts show-messages prints, newest first.
fn messages(tmux: &TmuxTestHarness) -> Vec<String> {
    let result = tmux.cmd().args(["show-messages"]).run();
    result.assert_success();
    result
        .stdout_lines()
        .into_iter()
        .map(|line| {
            line.split_once(": ")
                .map(|(_, text)| text.to_string())
                .unwrap_or(line)
        })
        .collect()
}

#[test]
fn message_limit_keeps_that_many_messages() {
    let tmux = server();
    tmux.cmd()
        .args(["set-option", "-g", "message-limit", "3"])
        .run()
        .assert_success();
    for i in 0..5 {
        tmux.cmd()
            .args(["display-message", "-p", &format!("m{i}")])
            .run()
            .assert_success();
    }
    let log = messages(&tmux);
    assert_eq!(log.len(), 3, "{log:?}");
    assert!(log[0].ends_with("command: show-messages"), "{log:?}");
    assert!(
        log[1].ends_with("command: display-message -p m4"),
        "{log:?}"
    );
    assert!(
        log[2].ends_with("command: display-message -p m3"),
        "{log:?}"
    );
}

#[test]
fn message_limit_zero_keeps_no_messages() {
    let tmux = server();
    tmux.cmd()
        .args(["set-option", "-g", "message-limit", "0"])
        .run()
        .assert_success();
    let log = messages(&tmux);
    assert_eq!(log.len(), 0, "{log:?}");
}

#[test]
fn show_messages_without_a_client() {
    let tmux = server();
    tmux.cmd()
        .args(["display-message", "-p", "hello"])
        .run()
        .assert_success();
    // The target client does not exist, which is not an error in 3.8.
    let result = tmux
        .cmd()
        .args(["show-messages", "-t", "/dev/no-such-client"])
        .run();
    result.assert_success();
    let out = result.stdout_str();
    assert!(out.contains("command: display-message -p hello"), "{out}");
}

/// show-messages -J lists the running jobs.
#[test]
fn show_messages_lists_jobs() {
    let tmux = server();
    tmux.cmd()
        .args(["run-shell", "-b", "sleep 30"])
        .run()
        .assert_success();
    let out = tmux.cmd().args(["show-messages", "-J"]).run();
    out.assert_success();
    let lines = out.stdout_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("Job 0: sleep 30 [fd=") && lines[0].ends_with(", status=0]"),
        "{lines:?}"
    );
}

/// show-messages -T describes each terminal, and with -J a blank line
/// separates the terminals from the jobs. -t limits it to the terminal of
/// one client.
#[test]
fn show_messages_lists_terminals() {
    let tmux = server();
    let _client = harness::PtyClient::attach(&tmux, 80, 24);
    tmux.wait_for("#{session_attached}", |n| n == "1", Duration::from_secs(5));
    let tty = tmux.query("#{client_tty}");
    let out = tmux.cmd().args(["show-messages", "-T"]).run();
    out.assert_success();
    let lines = out.stdout_lines();
    let header = format!("Terminal 0: screen for {tty}, flags=0x");
    assert!(lines[0].starts_with(&header), "{lines:?}");
    assert!(lines.len() > 100, "{}", lines.len());
    assert!(
        lines
            .iter()
            .any(|l| l.trim_start().starts_with("0: acsc: ")),
        "{lines:?}"
    );

    tmux.cmd()
        .args(["run-shell", "-b", "sleep 30"])
        .run()
        .assert_success();
    let both = tmux
        .cmd()
        .args(["show-messages", "-TJ"])
        .run()
        .stdout_lines();
    assert_eq!(both.len(), lines.len() + 2, "{both:?}");
    assert_eq!(both[lines.len()], "");
    assert!(both[lines.len() + 1].starts_with("Job 0: sleep 30"));

    let only = tmux
        .cmd()
        .args(["show-messages", "-T", "-t", &tty])
        .run()
        .stdout_lines();
    assert_eq!(only, lines);
}
