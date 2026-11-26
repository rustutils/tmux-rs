//! Jobs: run-shell, if-shell and #() commands, checked against tmux 3.8.

use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

const T: Duration = Duration::from_secs(5);

fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "s", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(T);
    tmux
}

fn out(tmux: &TmuxTestHarness, args: &[&str]) -> String {
    let r = tmux.cmd().args(args).run();
    if r.success() {
        r.stdout_trimmed()
    } else {
        format!("error: {}", r.stderr_str().trim())
    }
}

/// show-messages -J lists the running jobs, newest first.
#[test]
fn running_jobs_are_listed() {
    let tmux = server();
    out(&tmux, &["run-shell", "-b", "sleep 30"]);
    out(&tmux, &["run-shell", "-b", "sleep 31"]);
    let lines: Vec<String> = out(&tmux, &["show-messages", "-J"])
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("Job 0: sleep 31 ["), "{lines:?}");
    assert!(lines[1].starts_with("Job 1: sleep 30 ["), "{lines:?}");
}

/// run-shell uses /bin/sh whatever default-shell says, and runs in the
/// directory from -c with PWD set to it.
#[test]
fn run_shell_shell_and_directory() {
    let tmux = server();
    out(&tmux, &["set", "-g", "default-shell", "/bin/false"]);
    assert_eq!(out(&tmux, &["run-shell", "echo out"]), "out");
    assert_eq!(
        out(&tmux, &["run-shell", "-c", "/tmp", "echo $PWD; pwd"]),
        "/tmp\n/tmp"
    );
}

/// run-shell -E shows the command's standard error too.
#[test]
fn run_shell_shows_stderr_with_dash_e() {
    let tmux = server();
    assert_eq!(out(&tmux, &["run-shell", "echo out; echo err >&2"]), "out");
    assert_eq!(
        out(&tmux, &["run-shell", "-E", "echo out; echo err >&2"]),
        "out\nerr"
    );
}

/// A job whose pane goes away first finishes without harm.
#[test]
fn job_outlives_its_pane() {
    let tmux = server();
    let pane = out(
        &tmux,
        &["split-window", "-d", "-P", "-F", "#{pane_id}", "sleep 1000"],
    );
    out(
        &tmux,
        &["run-shell", "-b", "-t", &pane, "sleep 0.3; echo late"],
    );
    out(&tmux, &["kill-pane", "-t", &pane]);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(out(&tmux, &["display", "-p", "ok"]), "ok");
    assert_eq!(out(&tmux, &["show-messages", "-J"]), "");
}

/// A #() job in the status line whose client goes away first finishes
/// without harm.
#[test]
fn format_job_outlives_its_client() {
    let tmux = server();
    out(
        &tmux,
        &["set", "-g", "status-right", "#(sleep 0.3; echo late)"],
    );
    {
        let _client = harness::PtyClient::attach(&tmux, 80, 24);
        tmux.wait_for("#{session_attached}", |v| v == "1", T);
    }
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(out(&tmux, &["display", "-p", "ok"]), "ok");
}

/// run-shell leaves SHELL as the environment has it: only popups set it
/// to default-shell.
#[test]
fn run_shell_keeps_shell_variable() {
    let tmux = server();
    out(&tmux, &["set-environment", "-g", "SHELL", "/not/a/shell"]);
    assert_eq!(out(&tmux, &["run-shell", "echo $SHELL"]), "/not/a/shell");
}
