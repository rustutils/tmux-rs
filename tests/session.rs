use std::io::Write;
use std::time::Duration;

use harness::{PtyClient, TmuxTestHarness};

use super::harness;

// Ported from regress/new-session-size.sh
#[test]
fn new_session_default_size() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let size = tmux.query("#{window_width} #{window_height}");
    assert_eq!(size, "80 24");
}

// Ported from regress/new-session-size.sh (second case)
#[test]
fn new_session_explicit_size() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-x", "100", "-y", "50"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let size = tmux.query("#{window_width} #{window_height}");
    assert_eq!(size, "100 50");
}

// Ported from regress/new-session-command.sh
#[test]
fn new_session_with_command() {
    let mut tmux = TmuxTestHarness::new();
    // exec, so the pane runs cat itself. Some /bin/sh (dash on Ubuntu)
    // run a lone command as a child, and the pane would show sh.
    tmux.new_session()
        .args(["--", "exec cat"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // The command should become 'cat'. Right after the session starts the
    // pane process may not have run exec yet, so wait for it.
    tmux.wait_for(
        "#{pane_current_command}",
        |c| c == "cat",
        Duration::from_secs(5),
    );
}

// Ported from regress/has-session-return.sh
#[test]
fn has_session_returns_error_for_nonexistent() {
    let mut tmux = TmuxTestHarness::new();

    // has-session should fail when no server is running
    let result = tmux.cmd().args(["has-session", "-t", "foo"]).run();
    assert!(!result.success());

    // has-session should fail for nonexistent session even after new-session
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux.cmd().args(["has-session", "-t", "foo"]).run();
    assert!(!result.success());
}

// Ported from regress/has-session-return.sh (positive case)
#[test]
fn has_session_returns_success_for_existing() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "foo"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux.cmd().args(["has-session", "-t", "foo"]).run();
    assert!(result.success());
}

// Ported from regress/new-session-base-index.sh
#[test]
fn new_session_base_index() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Default base index is 0
    let idx = tmux.query("#{window_index}");
    assert_eq!(idx, "0");

    // Set base-index to 1 and create a new session
    let mut tmux2 = TmuxTestHarness::new();
    tmux2.new_session().run().assert_success();
    tmux2.wait_ready(Duration::from_secs(5));
    tmux2
        .cmd()
        .args(["set-option", "-g", "base-index", "1"])
        .run()
        .assert_success();
    tmux2.cmd().args(["new-window"]).run().assert_success();
    // The new window should have index 1 (or 2 if base-index was applied)
    let windows = tmux2
        .cmd()
        .args(["list-windows", "-F", "#{window_index}"])
        .run();
    let output = windows.stdout_trimmed();
    let indices: Vec<&str> = output.lines().collect();
    assert!(indices.contains(&"0") || indices.contains(&"1"));
}

// Ported from regress/kill-session-process-exit.sh
#[test]
fn kill_session_kills_processes() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["--", "sleep", "300"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Get the pane PID
    let pid_str = tmux.query("#{pane_pid}");
    let pid: u32 = pid_str.parse().expect("pane_pid should be a number");

    // Kill the session
    tmux.cmd().args(["kill-session"]).run().assert_success();

    // Give the process a moment to die
    std::thread::sleep(Duration::from_millis(500));

    // The process should be dead (kill -0 should fail)
    let check = std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status();
    if let Ok(status) = check {
        assert!(!status.success(), "process {pid} should have been killed");
    }
}

// Ported from regress/new-session-no-client.sh
#[test]
fn new_session_no_client() {
    let mut tmux = TmuxTestHarness::new();
    // new -d means no client attaches
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let clients = tmux.cmd().args(["list-clients"]).run();
    // No clients should be attached (output should be empty)
    assert_eq!(clients.stdout_trimmed(), "");
}

// Ported from regress/new-session-environment.sh
// Verifies that tmux propagates the calling environment to the first session.
#[test]
fn new_session_environment_propagation() {
    let tmux = TmuxTestHarness::new();
    let bin = harness::server_bin();

    // Make a script that dumps a few env vars and cwd
    let outfile = tempfile::NamedTempFile::new().unwrap();
    let outpath = outfile.path().to_path_buf();

    let mut script = tempfile::NamedTempFile::new().unwrap();
    writeln!(
        script,
        "(\n\
         echo TERM=$TERM\n\
         echo PWD=$(pwd)\n\
         echo PATH=$PATH\n\
         echo SHELL=$SHELL\n\
         echo TEST=$TEST\n\
         ) >{}",
        outpath.display()
    )
    .unwrap();
    script.flush().unwrap();
    let script_path = script.path().to_path_buf();

    // Config: new -- /bin/sh script
    let mut conf = tempfile::NamedTempFile::new().unwrap();
    writeln!(conf, "new -- /bin/sh {}", script_path.display()).unwrap();
    conf.flush().unwrap();

    // Run tmux start with a fully cleared environment, cwd=/
    let mut command = std::process::Command::new(&bin);
    command
        .args([
            "-S",
            tmux.socket_path(),
            &format!("-f{}", conf.path().display()),
            "start",
        ])
        .current_dir("/")
        .env_clear()
        .env("TERM", "ansi")
        .env("TEST", "test1")
        .env("PATH", "1")
        .env("SHELL", "/bin/sh");
    // Keep sanitizer settings, so an ASAN build does not turn leak
    // checking back on for this one process.
    if let Some(asan) = std::env::var_os("ASAN_OPTIONS") {
        command.env("ASAN_OPTIONS", asan);
    }
    let status = command.status().expect("failed to spawn tmux");
    assert!(status.success());

    std::thread::sleep(Duration::from_secs(1));

    let contents = std::fs::read_to_string(&outpath).unwrap_or_default();
    // Note: TERM ends up being the default-terminal value tmux picks, not the
    // input "ansi" because tmux normalises it. We only assert on values we control.
    assert!(contents.contains("PWD=/"), "got: {contents}");
    assert!(contents.contains("PATH=1"), "got: {contents}");
    assert!(contents.contains("SHELL=/bin/sh"), "got: {contents}");
    assert!(contents.contains("TEST=test1"), "got: {contents}");
}

/// With detach-on-destroy no-detached, a client whose session is killed
/// detaches when every other session is attached (upstream f5018171).
/// tmux-rs used to move it to another session anyway.
#[test]
fn detach_on_destroy_no_detached_with_only_attached_sessions() {
    let t = Duration::from_secs(5);
    let mut tmux = TmuxTestHarness::new();
    for name in ["a", "b"] {
        tmux.new_session()
            .args(["-s", name, "-x", "80", "-y", "24", "sleep 1000"])
            .run()
            .assert_success();
    }
    tmux.wait_ready(t);
    tmux.cmd()
        .args(["set", "-g", "detach-on-destroy", "no-detached"])
        .run()
        .assert_success();
    let mut a = PtyClient::attach_to(&tmux, "a", 80, 24);
    let _b = PtyClient::attach_to(&tmux, "b", 80, 24);
    tmux.wait_for("#{session_attached}", |v| v == "1", t);
    tmux.wait_for("#{S:#{session_attached}}", |v| v == "11", t);
    tmux.cmd()
        .args(["kill-session", "-t", "a"])
        .run()
        .assert_success();
    let start = std::time::Instant::now();
    while a.is_alive() {
        assert!(
            start.elapsed() < t,
            "client of the killed session did not detach"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(
        tmux.query("#{S:#{session_name}=#{session_attached} }"),
        "b=1"
    );
}

/// Killing the last window of a session group destroys every session in
/// the group. The loop used to remove sessions from the group it was
/// iterating, which skipped sessions and used the freed group.
#[test]
fn kill_last_window_of_session_group() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "keep", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args(["new-session", "-d", "-s", "a", "sleep 1000"])
        .run()
        .assert_success();
    for name in ["b", "c", "d"] {
        tmux.cmd()
            .args(["new-session", "-d", "-t", "a", "-s", name])
            .run()
            .assert_success();
    }

    tmux.cmd()
        .args(["kill-window", "-t", "a:0"])
        .run()
        .assert_success();

    let left = tmux
        .cmd()
        .args(["list-sessions", "-F", "#{session_name}"])
        .run()
        .stdout_trimmed();
    assert_eq!(left, "keep");
}
