use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use harness::TmuxTestHarness;

use super::harness;

/// Spawn a control-mode client attached to the given harness.
/// Returns the child process with piped stdin/stdout/stderr.
fn spawn_control_client(tmux: &TmuxTestHarness) -> std::process::Child {
    let bin = harness::client_bin();
    Command::new(&bin)
        .args(["-S", tmux.socket_path(), "-C", "attach"])
        .env("TERM", "screen")
        .env_remove("TMUX")
        .env_remove("TMUX_CONF")
        .env_remove("TMUX_TMPDIR")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn control mode client")
}

/// Read lines from a `BufReader` until `predicate` returns true for a line,
/// or until `timeout` elapses. Returns all lines collected.
fn read_until<R: std::io::Read>(
    reader: &mut BufReader<R>,
    timeout: Duration,
    predicate: impl Fn(&str) -> bool,
) -> Vec<String> {
    let start = Instant::now();
    let mut lines = Vec::new();
    let mut line_buf = String::new();

    loop {
        line_buf.clear();
        // We rely on the child eventually producing output or EOF.
        // Use a thread with a timeout approach.
        match reader.read_line(&mut line_buf) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = line_buf.trim_end_matches('\n').to_string();
                let matched = predicate(&trimmed);
                lines.push(trimmed);
                if matched {
                    break;
                }
            }
            Err(_) => break,
        }
        if start.elapsed() > timeout {
            break;
        }
    }
    lines
}

/// Send a command to the control mode client and collect output lines until
/// a line matching the predicate appears or timeout.
fn send_and_collect(
    stdin: &mut impl Write,
    reader: &mut BufReader<impl std::io::Read>,
    cmd: &str,
    timeout: Duration,
    predicate: impl Fn(&str) -> bool,
) -> Vec<String> {
    writeln!(stdin, "{cmd}").expect("failed to write command");
    stdin.flush().expect("failed to flush stdin");
    read_until(reader, timeout, predicate)
}

/// Helper to set up a harness with a detached session and a control mode client.
/// Returns (harness, child, stdin, `stdout_reader`).
fn setup_control() -> (
    TmuxTestHarness,
    std::process::Child,
    std::process::ChildStdin,
    BufReader<std::process::ChildStdout>,
) {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-x", "80", "-y", "24"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let mut child = spawn_control_client(&tmux);
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    (tmux, child, stdin, reader)
}

/// Cleanup helper: send detach and kill child.
fn cleanup(mut stdin: std::process::ChildStdin, mut child: std::process::Child) {
    let _ = writeln!(stdin, "detach");
    let _ = stdin.flush();
    drop(stdin);
    // Give it a moment then kill
    std::thread::sleep(Duration::from_millis(200));
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn control_mode_list_sessions() {
    let (tmux, child, mut stdin, mut reader) = setup_control();

    // Wait briefly for initial output from control mode
    let initial = read_until(&mut reader, Duration::from_secs(3), |line| {
        // The initial greeting ends with a %end line or similar.
        // Just consume until we see a %begin/%end pair or timeout.
        line.starts_with("%end")
    });
    assert!(!initial.is_empty(), "expected initial control mode output");

    // Send list-sessions command
    let lines = send_and_collect(
        &mut stdin,
        &mut reader,
        "list-sessions",
        Duration::from_secs(5),
        |line| line.starts_with("%end"),
    );

    // We should see output containing a session entry (the default session name
    // or "0:") between %begin and %end.
    let output = lines.join("\n");
    assert!(
        output.contains("%begin") && output.contains("%end"),
        "expected %%begin/%%end framing in control mode output, got:\n{output}"
    );
    // The session should appear in the output
    assert!(
        lines.iter().any(|l| !l.starts_with('%') && !l.is_empty()),
        "expected session listing in output, got:\n{output}"
    );

    cleanup(stdin, child);
    drop(tmux);
}

#[test]
fn control_mode_new_window_notification() {
    let (tmux, child, mut stdin, mut reader) = setup_control();

    // Drain initial output (attach response + session-changed notification)
    let _initial = read_until(&mut reader, Duration::from_secs(3), |line| {
        line.starts_with("%end")
    });

    // Send new-window and wait for the %window-add notification specifically,
    // since it arrives after the command's %begin/%end pair.
    let lines = send_and_collect(
        &mut stdin,
        &mut reader,
        "new-window",
        Duration::from_secs(5),
        |line| line.starts_with("%window-add"),
    );

    let output = lines.join("\n");
    assert!(
        lines.iter().any(|l| l.starts_with("%window-add")),
        "expected %%window-add notification, got:\n{output}"
    );

    cleanup(stdin, child);
    drop(tmux);
}

#[test]
fn control_mode_split_window_layout_change() {
    let (tmux, child, mut stdin, mut reader) = setup_control();

    // Drain initial output (attach response + session-changed notification)
    let _initial = read_until(&mut reader, Duration::from_secs(3), |line| {
        line.starts_with("%end")
    });

    // Send split-window and wait for the %layout-change notification,
    // which arrives after the command's %begin/%end pair.
    let lines = send_and_collect(
        &mut stdin,
        &mut reader,
        "split-window",
        Duration::from_secs(5),
        |line| line.starts_with("%layout-change"),
    );

    let output = lines.join("\n");
    assert!(
        lines.iter().any(|l| l.starts_with("%layout-change")),
        "expected %%layout-change notification, got:\n{output}"
    );

    cleanup(stdin, child);
    drop(tmux);
}

#[test]
fn control_mode_rename_session() {
    let (tmux, child, mut stdin, mut reader) = setup_control();

    // Drain initial output (attach response + session-changed notification)
    let _initial = read_until(&mut reader, Duration::from_secs(3), |line| {
        line.starts_with("%end")
    });

    // Send rename-session and wait for a line containing "newname",
    // which appears in the %session-renamed or %session-changed notification
    // after the command's %begin/%end pair.
    let lines = send_and_collect(
        &mut stdin,
        &mut reader,
        "rename-session newname",
        Duration::from_secs(5),
        |line| line.contains("newname"),
    );

    let output = lines.join("\n");
    assert!(
        lines.iter().any(|l| l.contains("newname")),
        "expected 'newname' in output, got:\n{output}"
    );

    cleanup(stdin, child);
    drop(tmux);
}

#[test]
fn control_mode_multiple_commands() {
    let (tmux, child, mut stdin, mut reader) = setup_control();

    // Drain initial output
    let _initial = read_until(&mut reader, Duration::from_secs(3), |line| {
        line.starts_with("%end")
    });

    // Send first command
    let lines1 = send_and_collect(
        &mut stdin,
        &mut reader,
        "list-sessions",
        Duration::from_secs(5),
        |line| line.starts_with("%end"),
    );
    assert!(
        lines1.iter().any(|l| l.starts_with("%begin")),
        "first command should produce %%begin, got:\n{}",
        lines1.join("\n")
    );

    // Send second command
    let lines2 = send_and_collect(
        &mut stdin,
        &mut reader,
        "list-windows",
        Duration::from_secs(5),
        |line| line.starts_with("%end"),
    );
    assert!(
        lines2.iter().any(|l| l.starts_with("%begin")),
        "second command should produce %%begin, got:\n{}",
        lines2.join("\n")
    );

    // Send third command
    let lines3 = send_and_collect(
        &mut stdin,
        &mut reader,
        "list-panes",
        Duration::from_secs(5),
        |line| line.starts_with("%end"),
    );
    assert!(
        lines3.iter().any(|l| l.starts_with("%begin")),
        "third command should produce %%begin, got:\n{}",
        lines3.join("\n")
    );

    cleanup(stdin, child);
    drop(tmux);
}

#[test]
fn control_mode_display_message() {
    let (tmux, child, mut stdin, mut reader) = setup_control();

    // Drain initial output
    let _initial = read_until(&mut reader, Duration::from_secs(3), |line| {
        line.starts_with("%end")
    });

    // Send display-message to query the session name
    let lines = send_and_collect(
        &mut stdin,
        &mut reader,
        "display-message -p \"#{session_name}\"",
        Duration::from_secs(5),
        |line| line.starts_with("%end"),
    );

    let output = lines.join("\n");
    // Should have %begin/%end framing
    assert!(
        output.contains("%begin") && output.contains("%end"),
        "expected %%begin/%%end framing, got:\n{output}"
    );
    // The session name should appear between %begin and %end.
    // Default session name is "0".
    let data_lines: Vec<&String> = lines
        .iter()
        .filter(|l| !l.starts_with('%') && !l.is_empty())
        .collect();
    assert!(
        !data_lines.is_empty(),
        "expected session name in output, got:\n{output}"
    );
    // The default session name is typically "0"
    assert!(
        data_lines.iter().any(|l| l.trim() == "0"),
        "expected default session name '0', got data lines: {data_lines:?}"
    );

    cleanup(stdin, child);
    drop(tmux);
}

/// Wait up to `timeout` for a child to exit. Returns whether it did.
fn wait_for_exit(child: &mut std::process::Child, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if child.try_wait().expect("try_wait").is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

/// With the wait-exit flag a control client prints %exit and then waits
/// for an empty line before it exits.
#[test]
fn control_mode_wait_exit_waits_for_empty_line() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let mut child = spawn_control_client(&tmux);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    stdin
        .write_all(b"refresh-client -f wait-exit\ndetach-client\n")
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        let line = rx.recv_timeout(left).expect("no %exit from control client");
        if line.starts_with("%exit") {
            break;
        }
    }
    assert!(
        !wait_for_exit(&mut child, Duration::from_millis(500)),
        "control client exited without waiting for an empty line"
    );
    stdin.write_all(b"\n").unwrap();
    assert!(
        wait_for_exit(&mut child, Duration::from_secs(5)),
        "control client did not exit"
    );
}

/// Toggling no-output while a pane produces output used to leave queued
/// output blocks behind, so the client never finished exiting (upstream
/// issue 5049).
#[test]
fn control_mode_exits_after_toggling_no_output() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sh -c 'while :; do echo 0123456789abcdef; done'"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let mut child = spawn_control_client(&tmux);
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    // Do not read yet: the pipe fills up and pane output queues up as
    // blocks in the server. Then toggle no-output, which resets the panes.
    std::thread::sleep(Duration::from_millis(500));
    stdin.write_all(b"refresh-client -f no-output\n").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    stdin.write_all(b"refresh-client -f !no-output\n").unwrap();
    std::thread::sleep(Duration::from_millis(200));
    std::thread::spawn(move || {
        let mut sink = Vec::new();
        let _ = std::io::Read::read_to_end(&mut BufReader::new(stdout), &mut sink);
    });
    std::thread::sleep(Duration::from_millis(200));
    drop(stdin);

    let exited = wait_for_exit(&mut child, Duration::from_secs(5));
    if !exited {
        let _ = child.kill();
        let _ = child.wait();
    }
    tmux.kill_server();
    assert!(exited, "control client did not exit after its input closed");
}

/// A control client that never reads its output must not keep the server
/// alive forever. tmux 3.8 gives up on it after 10 seconds.
#[test]
fn server_exits_despite_stuck_control_client() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sh -c 'while :; do echo 0123456789abcdef; done'"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    #[expect(
        clippy::literal_string_with_formatting_args,
        reason = "tmux format string, not a Rust one"
    )]
    let pid: i32 = tmux.query("#{pid}").parse().expect("server pid");

    // Keep stdout open but never read it, so the pipe fills up.
    let mut child = spawn_control_client(&tmux);
    let _stdout = child.stdout.take().unwrap();
    std::thread::sleep(Duration::from_secs(1));

    // A server that is shutting down refuses new clients, so watch the
    // process itself rather than asking it with has-session.
    tmux.cmd().args(["kill-server"]).run();
    let start = Instant::now();
    let mut gone = false;
    while start.elapsed() < Duration::from_secs(25) {
        if unsafe { libc::kill(pid, 0) } != 0 {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(gone, "server still running 25 seconds after kill-server");
}

/// refresh-client -f sets and clears client flags. The flag parser used to
/// never run its loop, so every flag was silently ignored.
#[test]
fn refresh_client_sets_client_flags() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let out = tmux.control_mode(
        "refresh-client -f ignore-size,no-output,read-only\n\
         display-message -p 'set:#{client_flags}'\n\
         refresh-client -f !ignore-size,!no-output\n\
         display-message -p 'cleared:#{client_flags}'\n",
    );
    let stdout = out.stdout_str();
    let line = |prefix: &str| {
        stdout
            .lines()
            .find(|l| l.starts_with(prefix))
            .unwrap_or_else(|| panic!("no {prefix} line in {stdout:?}"))
            .to_string()
    };
    let set = line("set:");
    for flag in ["ignore-size", "no-output", "read-only"] {
        assert!(set.contains(flag), "{flag} missing in {set:?}");
    }
    let cleared = line("cleared:");
    assert!(
        !cleared.contains("ignore-size") && !cleared.contains("no-output"),
        "{cleared:?}"
    );
    assert!(
        cleared.contains("read-only"),
        "read-only should stay set: {cleared:?}"
    );
}

/// tmux 3.8 removed the active-pane client flag, so it is ignored.
#[test]
fn active_pane_client_flag_is_ignored() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let out = tmux.control_mode(
        "refresh-client -f active-pane\ndisplay-message -p 'flags:#{client_flags}'\n",
    );
    let stdout = out.stdout_str();
    let flags = stdout
        .lines()
        .find(|l| l.starts_with("flags:"))
        .expect("flags line");
    assert!(!flags.contains("active-pane"), "{flags:?}");
}

/// select-window followed by kill-session in one command queues a
/// session-window-changed notification for a session that is gone by the
/// time it fires. With a control client attached elsewhere this read the
/// destroyed session's window and killed the server (upstream 035447b0).
#[test]
fn window_changed_notification_after_kill_session() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "keep", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args(["new-session", "-d", "-s", "x", "sleep 1000"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["new-window", "-t", "x", "sleep 1000"])
        .run()
        .assert_success();

    let mut child = spawn_control_client(&tmux);
    let _stdin = child.stdin.take().unwrap();
    std::thread::sleep(Duration::from_millis(300));
    tmux.cmd()
        .args(["select-window", "-t", "x:0", ";", "kill-session", "-t", "x"])
        .run();
    std::thread::sleep(Duration::from_millis(300));
    let alive = tmux
        .cmd()
        .args(["has-session", "-t", "keep"])
        .run()
        .success();
    let _ = child.kill();
    let _ = child.wait();
    assert!(alive, "server died");
}

/// Renaming the newest buffer without -b notifies the old name as deleted.
/// The old name used to be read after the rename had freed it (upstream
/// 7e50eb0e), so the notification showed the new name.
#[test]
fn rename_top_buffer_notifies_old_name() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let out = tmux.control_mode("set-buffer hello\nset-buffer -n renamed\n");
    let lines: Vec<String> = out
        .stdout_str()
        .lines()
        .filter(|l| l.starts_with("%paste-buffer"))
        .map(str::to_string)
        .collect();
    assert_eq!(
        lines,
        [
            "%paste-buffer-changed buffer0",
            "%paste-buffer-deleted buffer0",
            "%paste-buffer-changed renamed",
        ]
    );
}
