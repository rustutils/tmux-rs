//! Hooks and the event payloads they see, checked against tmux 3.8.
//!
//! Each test sets a hook that stores what it sees in a user option with
//! `set -gF`, then waits for the option. Hooks are queued commands, so
//! they run a moment after the command that fired the event.

use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

const T: Duration = Duration::from_secs(5);

fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "main", "-n", "first", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(T);
    tmux
}

fn run(tmux: &TmuxTestHarness, args: &[&str]) {
    tmux.cmd().args(args).run().assert_success();
}

/// Sets a global hook that stores `format`, expanded, in @seen.
fn record(tmux: &TmuxTestHarness, hook: &str, format: &str) {
    run(
        tmux,
        &["set-hook", "-g", hook, &format!("set -gF @seen '{format}'")],
    );
}

/// Waits until @seen is set and returns it.
fn seen(tmux: &TmuxTestHarness) -> String {
    tmux.wait_for("#{@seen}", |v| !v.is_empty(), T)
}

#[test]
fn window_renamed_has_old_and_new_name() {
    let tmux = server();
    record(
        &tmux,
        "window-renamed",
        "#{hook} #{hook_event} #{hook_window_name} #{hook_old_name} #{hook_new_name}",
    );
    run(&tmux, &["rename-window", "second"]);
    assert_eq!(
        seen(&tmux),
        "window-renamed window-renamed second first second"
    );
    let id = tmux.query("#{window_id}");
    run(&tmux, &["set", "-gu", "@seen"]);
    record(&tmux, "window-renamed", "#{hook_window}");
    run(&tmux, &["rename-window", "third"]);
    assert_eq!(seen(&tmux), id);
}

#[test]
fn session_renamed_has_old_and_new_name() {
    let tmux = server();
    record(
        &tmux,
        "session-renamed",
        "#{hook_session_name} #{hook_old_name} #{hook_new_name}",
    );
    run(&tmux, &["rename-session", "other"]);
    assert_eq!(seen(&tmux), "other main other");
}

#[test]
fn after_hook_has_arguments_and_flags() {
    let tmux = server();
    record(
        &tmux,
        "after-new-window",
        "#{hook} #{hook_flag_n} #{hook_flag_d} #{hook_argument_0} [#{hook_arguments}]",
    );
    run(&tmux, &["new-window", "-d", "-n", "extra", "sleep 1000"]);
    assert_eq!(
        seen(&tmux),
        "after-new-window extra 1 sleep 1000 [-d -n extra \"sleep 1000\"]"
    );
}

#[test]
fn window_pane_changed_has_new_and_old_pane() {
    let tmux = server();
    let first = tmux.query("#{pane_id}");
    run(&tmux, &["split-window", "-d", "sleep 1000"]);
    let second = tmux
        .cmd()
        .args(["display", "-p", "-t", ":.1", "#{pane_id}"])
        .run()
        .stdout_trimmed();
    record(
        &tmux,
        "window-pane-changed",
        "#{hook_pane} #{hook_new_pane} #{hook_old_pane}",
    );
    run(&tmux, &["select-pane", "-t", &second]);
    assert_eq!(seen(&tmux), format!("{second} {second} {first}"));
}

#[test]
fn pane_mode_hooks_name_the_modes() {
    let tmux = server();
    run(
        &tmux,
        &[
            "set-hook",
            "-g",
            "pane-mode-entered",
            "set -gF @entered '#{hook_current_mode} #{hook_mode_entered}'",
        ],
    );
    run(
        &tmux,
        &[
            "set-hook",
            "-g",
            "pane-mode-exited",
            "set -gF @exited '#{hook_previous_mode} #{hook_mode_entered}'",
        ],
    );
    run(
        &tmux,
        &[
            "set-hook",
            "-g",
            "pane-mode-changed",
            "set -gaF @changed '#{hook_mode_entered}'",
        ],
    );
    run(&tmux, &["copy-mode"]);
    assert_eq!(
        tmux.wait_for("#{@entered}", |v| !v.is_empty(), T),
        "copy-mode 1"
    );
    run(&tmux, &["send-keys", "-X", "cancel"]);
    assert_eq!(
        tmux.wait_for("#{@exited}", |v| !v.is_empty(), T),
        "copy-mode 0"
    );
    assert_eq!(tmux.wait_for("#{@changed}", |v| v.len() == 2, T), "10");
}

#[test]
fn pane_hook_names_are_spelled_right() {
    let tmux = server();
    for hook in [
        "pane-focus-in",
        "pane-focus-out",
        "pane-mode-changed",
        "pane-set-clipboard",
        "pane-title-changed",
    ] {
        run(&tmux, &["set-hook", "-g", hook, "display hi"]);
    }
}

#[test]
fn pane_title_changed_has_the_title() {
    let tmux = server();
    record(&tmux, "pane-title-changed", "#{hook_new_title}");
    run(&tmux, &["select-pane", "-T", "my title"]);
    assert_eq!(seen(&tmux), "my title");
}

/// With remain-on-exit the pane waits for its exit status before
/// pane-died fires. pane-exited can fire before the status is known, in C
/// too, so it is not tested here.
#[test]
fn pane_died_has_the_exit_status() {
    let tmux = server();
    run(&tmux, &["set", "-g", "remain-on-exit", "on"]);
    record(
        &tmux,
        "pane-died",
        "#{hook} #{hook_exit_status} #{hook_exit_success}",
    );
    run(&tmux, &["new-window", "-d", "exit 3"]);
    assert_eq!(seen(&tmux), "pane-died 3 0");
}

#[test]
fn hook_commands_run_in_order() {
    let tmux = server();
    run(
        &tmux,
        &["set-hook", "-g", "window-renamed[0]", "set -ga @order a"],
    );
    run(
        &tmux,
        &["set-hook", "-g", "window-renamed[1]", "set -ga @order b"],
    );
    run(
        &tmux,
        &["set-hook", "-g", "after-rename-window", "set -ga @order c"],
    );
    run(&tmux, &["rename-window", "x"]);
    assert_eq!(tmux.wait_for("#{@order}", |v| v.len() == 3, T), "cab");
}

#[test]
fn hooks_do_not_fire_hooks() {
    let tmux = server();
    // The hook renames the window again, which must not fire it again.
    run(
        &tmux,
        &[
            "set-hook",
            "-g",
            "window-renamed",
            "set -ga @count x ; rename-window again",
        ],
    );
    run(&tmux, &["rename-window", "x"]);
    tmux.wait_for("#{window_name}", |v| v == "again", T);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(tmux.query("#{@count}"), "x");
}

#[test]
fn set_hook_run_fires_now() {
    let tmux = server();
    record(&tmux, "window-renamed", "#{hook}");
    run(&tmux, &["set-hook", "-R", "window-renamed"]);
    assert_eq!(seen(&tmux), "window-renamed");
}

#[test]
fn user_hook_runs_its_command() {
    let tmux = server();
    run(&tmux, &["set-hook", "-g", "@mine", "set -g @seen ran"]);
    run(&tmux, &["set-hook", "-R", "@mine"]);
    assert_eq!(seen(&tmux), "ran");
}
