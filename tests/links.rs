//! Sessions, winlinks and session groups, checked against tmux 3.8.
//!
//! These cover how windows are linked into sessions: linking and
//! unlinking, the current and last windows, moving and renumbering,
//! grouped sessions staying in step, and session names.

use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

const T: Duration = Duration::from_secs(5);

/// A server with session `a` holding windows 0, 1, 2 and 5.
fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "a", "-x", "80", "-y", "24", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(T);
    for idx in ["a:1", "a:2", "a:5"] {
        run(&tmux, &["new-window", "-d", "-t", idx, "sleep 1000"]);
    }
    tmux
}

fn run(tmux: &TmuxTestHarness, args: &[&str]) {
    tmux.cmd().args(args).run().assert_success();
}

/// The windows of `target` as index and flags, one string.
fn windows(tmux: &TmuxTestHarness, target: &str) -> String {
    tmux.cmd()
        .args([
            "list-windows",
            "-t",
            target,
            "-F",
            "#{window_index}#{window_flags}",
        ])
        .run()
        .stdout_lines()
        .join(" ")
}

/// Every window of every session, as session:index.
fn all_windows(tmux: &TmuxTestHarness) -> String {
    tmux.cmd()
        .args([
            "list-windows",
            "-a",
            "-F",
            "#{session_name}:#{window_index}",
        ])
        .run()
        .stdout_lines()
        .join(" ")
}

fn session_names(tmux: &TmuxTestHarness) -> Vec<String> {
    tmux.cmd()
        .args(["list-sessions", "-F", "[#{session_name}|#{session_group}]"])
        .run()
        .stdout_lines()
}

#[test]
fn linked_window_formats() {
    let tmux = server();
    run(&tmux, &["new-session", "-d", "-s", "b", "sleep 1000"]);
    run(&tmux, &["link-window", "-s", "a:1", "-t", "b:3"]);
    let f = "#{session_name}:#{window_index} #{window_linked} \
             #{window_linked_sessions} #{window_linked_sessions_list}";
    let lines = tmux
        .cmd()
        .args(["list-windows", "-a", "-F", f])
        .run()
        .stdout_lines();
    assert_eq!(
        lines,
        [
            "a:0 0 1 a",
            "a:1 1 2 a,b",
            "a:2 0 1 a",
            "a:5 0 1 a",
            "b:0 0 1 b",
            "b:3 1 2 a,b",
        ]
    );
}

/// In a group every window is in at least two sessions, so
/// `window_linked` is 1. `window_linked_sessions` counts a group once
/// (upstream stopped using the window's reference count for both).
#[test]
fn linked_window_formats_in_a_group() {
    let tmux = server();
    run(&tmux, &["new-session", "-d", "-s", "b", "sleep 1000"]);
    run(&tmux, &["link-window", "-s", "a:1", "-t", "b:3"]);
    run(&tmux, &["new-session", "-d", "-s", "g", "-t", "a"]);
    let f = "#{session_name}:#{window_index} #{window_linked} \
             #{window_linked_sessions}";
    let lines = tmux
        .cmd()
        .args(["list-windows", "-a", "-F", f])
        .run()
        .stdout_lines();
    assert_eq!(
        lines,
        [
            "a:0 1 1", "a:1 1 2", "a:2 1 1", "a:5 1 1", "b:0 0 1", "b:3 1 2", "g:0 1 1", "g:1 1 2",
            "g:2 1 1", "g:5 1 1",
        ]
    );
}

#[test]
fn unlink_last_window_of_a_session() {
    let tmux = server();
    run(&tmux, &["new-session", "-d", "-s", "b", "sleep 1000"]);
    let r = tmux.cmd().args(["unlink-window", "-t", "b:0"]).run();
    r.assert_failure();
    assert_eq!(r.stderr_str().trim(), "window only linked to one session");
    run(&tmux, &["unlink-window", "-k", "-t", "b:0"]);
    assert_eq!(session_names(&tmux), ["[a|]"]);
}

/// Killing a session's last window closes the session before the window
/// is unlinked (3.8 leaves the last window to `session_destroy`).
#[test]
fn kill_last_window_closes_session_first() {
    let tmux = server();
    run(&tmux, &["new-session", "-d", "-s", "b", "sleep 1000"]);
    run(&tmux, &["set", "-g", "@seen", ""]);
    run(
        &tmux,
        &["set-hook", "-g", "window-unlinked", "set -ga @seen 'u '"],
    );
    run(
        &tmux,
        &["set-hook", "-g", "session-closed", "set -ga @seen 'c '"],
    );
    run(&tmux, &["kill-window", "-t", "b:0"]);
    let seen = tmux.wait_for("#{@seen}", |v| v.len() >= 3, T);
    assert_eq!(seen, "c u");
}

/// Killing the current window makes the last window current, and the
/// last flag moves down the stack.
#[test]
fn kill_current_window_selects_last() {
    let tmux = server();
    for w in ["a:2", "a:5", "a:1"] {
        run(&tmux, &["select-window", "-t", w]);
    }
    assert_eq!(windows(&tmux, "a"), "0 1* 2 5-");
    run(&tmux, &["kill-window", "-t", "a:1"]);
    assert_eq!(windows(&tmux, "a"), "0 2- 5*");
    run(&tmux, &["last-window", "-t", "a"]);
    assert_eq!(windows(&tmux, "a"), "0 2* 5-");
}

#[test]
fn next_and_previous_with_alerts() {
    let tmux = server();
    run(&tmux, &["set", "-g", "bell-action", "any"]);
    run(
        &tmux,
        &[
            "new-window",
            "-d",
            "-t",
            "a:3",
            "sh -c 'sleep 0.3; printf \"\\a\"; sleep 1000'",
        ],
    );
    tmux.wait_for("#{W:#{window_bell_flag}}", |v| v.contains('1'), T);
    run(&tmux, &["next-window", "-a", "-t", "a"]);
    assert_eq!(tmux.query("#{window_index}"), "3");
    let r = tmux.cmd().args(["next-window", "-a", "-t", "a"]).run();
    r.assert_failure();
    assert_eq!(r.stderr_str().trim(), "no next window");
    let r = tmux.cmd().args(["previous-window", "-a", "-t", "a"]).run();
    r.assert_failure();
    assert_eq!(r.stderr_str().trim(), "no previous window");
}

#[test]
fn move_window_renumber_and_insert() {
    let tmux = server();
    run(&tmux, &["kill-window", "-t", "a:1"]);
    run(&tmux, &["select-window", "-t", "a:5"]);
    run(&tmux, &["select-window", "-t", "a:2"]);
    run(&tmux, &["move-window", "-r", "-t", "a"]);
    assert_eq!(windows(&tmux, "a"), "0 1* 2-");
    // -a after 0 shuffles the others up, -b before 0 too.
    run(
        &tmux,
        &["new-window", "-d", "-a", "-t", "a:0", "sleep 1000"],
    );
    assert_eq!(windows(&tmux, "a"), "0 1 2* 3-");
    run(
        &tmux,
        &["new-window", "-d", "-b", "-t", "a:0", "sleep 1000"],
    );
    assert_eq!(windows(&tmux, "a"), "0 1 2 3* 4-");
}

#[test]
fn grouped_sessions_stay_in_step() {
    let tmux = server();
    run(&tmux, &["new-session", "-d", "-s", "g1", "sleep 1000"]);
    run(&tmux, &["new-session", "-d", "-s", "g2", "-t", "g1"]);
    run(&tmux, &["new-window", "-d", "-t", "g2:4", "sleep 1000"]);
    run(&tmux, &["link-window", "-s", "a:0", "-t", "g1:6"]);
    run(&tmux, &["move-window", "-s", "g1:4", "-t", "g1:9"]);
    let all = all_windows(&tmux);
    assert!(all.ends_with("g1:0 g1:6 g1:9 g2:0 g2:6 g2:9"), "got: {all}");
    run(&tmux, &["select-window", "-t", "g1:9"]);
    run(&tmux, &["kill-window", "-t", "g2:6"]);
    assert_eq!(windows(&tmux, "g1"), "0- 9*");
    assert_eq!(windows(&tmux, "g2"), "0* 9");
}

/// new-session -t with a name that is not a session starts a new group
/// with that name. tmux-rs crashed synchronizing the new session with
/// itself.
#[test]
fn new_group_by_name() {
    let tmux = server();
    run(&tmux, &["new-session", "-d", "-s", "s1", "-t", "grp"]);
    run(&tmux, &["new-session", "-d", "-s", "s2", "-t", "grp"]);
    run(&tmux, &["new-window", "-d", "-t", "s2:3", "sleep 1000"]);
    assert_eq!(session_names(&tmux), ["[a|]", "[s1|grp]", "[s2|grp]"]);
    assert_eq!(windows(&tmux, "s1"), "0* 3");
}

/// 3.8 keeps ':' and '.' in session names and allows an empty name.
#[test]
#[ignore = "session names not checked as in 3.8, run 17 task 5"]
fn session_names_keep_colon_and_dot() {
    let tmux = server();
    run(&tmux, &["new-session", "-d", "-s", "x:y.z", "sleep 1000"]);
    run(&tmux, &["new-session", "-d", "-s", "", "sleep 1000"]);
    run(&tmux, &["new-session", "-d", "-s", "b", "sleep 1000"]);
    run(&tmux, &["rename-session", "-t", "b", "p.q"]);
    run(&tmux, &["new-session", "-d", "-s", "s3", "-t", "q.r"]);
    assert_eq!(
        session_names(&tmux),
        ["[|]", "[a|]", "[p.q|]", "[s3|q.r]", "[x:y.z|]"]
    );
}

/// Names with control characters or bytes that are not UTF-8 are
/// rejected. tmux-rs accepted control characters and crashed on the
/// rest.
#[test]
#[ignore = "session names not checked as in 3.8, run 17 task 5"]
fn bad_session_names_are_rejected() {
    use std::os::unix::ffi::OsStrExt;
    let tmux = server();
    let bad: [&[u8]; 3] = [b"c\x01d", b"c\td", b"c\xffd"];
    for name in bad {
        let name = std::ffi::OsStr::from_bytes(name);
        let r = tmux
            .cmd()
            .args([
                "new-session".into(),
                "-d".into(),
                "-s".into(),
                name.to_os_string(),
            ])
            .run();
        r.assert_failure();
        assert!(
            r.stderr_str().starts_with("invalid session name: "),
            "{:?}: {}",
            name,
            r.stderr_str()
        );
        let r = tmux
            .cmd()
            .args([
                "rename-session".into(),
                "-t".into(),
                "a".into(),
                name.to_os_string(),
            ])
            .run();
        r.assert_failure();
        assert!(
            r.stderr_str().starts_with("invalid session name: "),
            "{:?}: {}",
            name,
            r.stderr_str()
        );
        let r = tmux
            .cmd()
            .args([
                "new-session".into(),
                "-d".into(),
                "-s".into(),
                "g".into(),
                "-t".into(),
                name.to_os_string(),
            ])
            .run();
        r.assert_failure();
        assert!(
            r.stderr_str().starts_with("invalid session group name: "),
            "{:?}: {}",
            name,
            r.stderr_str()
        );
    }
    assert_eq!(session_names(&tmux), ["[a|]"]);
}

/// `window_stack_index` is a window's place on the last window stack, 0
/// for one that is not on it. tmux-rs gave the stack's length.
#[test]
fn window_stack_index() {
    let tmux = server();
    run(&tmux, &["kill-window", "-t", "a:5"]);
    run(&tmux, &["select-window", "-t", "a:1"]);
    let f = "#{window_index}:#{window_stack_index}";
    let lines = tmux
        .cmd()
        .args(["list-windows", "-t", "a", "-F", f])
        .run()
        .stdout_lines();
    assert_eq!(lines, ["0:1", "1:0", "2:0"]);
}

/// The marked pane moves with its window when swap-window swaps it.
#[test]
fn swap_window_keeps_the_marked_pane() {
    let tmux = server();
    run(&tmux, &["select-pane", "-m", "-t", "a:1"]);
    run(&tmux, &["swap-window", "-s", "a:1", "-t", "a:2"]);
    let f = "#{window_index}:#{window_marked_flag}";
    let lines = tmux
        .cmd()
        .args(["list-windows", "-t", "a", "-F", f])
        .run()
        .stdout_lines();
    assert_eq!(lines, ["0:0", "1:0", "2:1", "5:0"]);
}

/// run-shell -t keeps working when its session is killed while the
/// command runs.
#[test]
fn run_shell_outlives_its_session() {
    let tmux = server();
    run(&tmux, &["new-session", "-d", "-s", "v", "sleep 1000"]);
    run(&tmux, &["run-shell", "-b", "-t", "v", "sleep 0.3; echo hi"]);
    run(&tmux, &["kill-session", "-t", "v"]);
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(session_names(&tmux), ["[a|]"]);
}
