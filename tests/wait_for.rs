//! wait-for on channels and events, checked against tmux 3.8.
//!
//! A waiting command blocks its client, so each test runs it on a thread
//! and waits until `wait-for -l` lists it before going on.

use std::time::{Duration, Instant};

use harness::{TmuxResult, TmuxTestHarness};

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

fn run(tmux: &TmuxTestHarness, args: &[&str]) -> TmuxResult {
    tmux.cmd().args(args).run()
}

/// The client names `wait-for -l` lists for `list_args`, once there are
/// `n` of them.
fn waiters(tmux: &TmuxTestHarness, list_args: &[&str], n: usize) -> Vec<String> {
    let start = Instant::now();
    loop {
        let out = run(tmux, list_args);
        out.assert_success();
        let lines = out.stdout_lines();
        if lines.len() == n {
            return lines;
        }
        assert!(start.elapsed() < T, "waiters: {lines:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn event_wait_ends_when_the_event_fires() {
    let tmux = server();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| run(&tmux, &["wait-for", "-E", "-v", "window-renamed"]));
        waiters(&tmux, &["wait-for", "-E", "-l", "window-renamed"], 1);
        run(&tmux, &["rename-window", "second"]).assert_success();
        let result = waiter.join().unwrap();
        result.assert_success();
        let lines = result.stdout_lines();
        let id = tmux.query("#{window_id}");
        for expected in [
            "event=window-renamed".to_string(),
            "new_name=second".to_string(),
            "old_name=first".to_string(),
            format!("window={id}"),
        ] {
            assert!(lines.contains(&expected), "{expected} in {lines:?}");
        }
        let mut sorted = lines.clone();
        sorted.sort();
        assert_eq!(lines, sorted, "payload lines are sorted by name");
    });
}

#[test]
fn event_filter_waits_until_true() {
    let tmux = server();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| {
            run(
                &tmux,
                &[
                    "wait-for",
                    "-E",
                    "-F",
                    "#{==:#{new_name},b}",
                    "window-renamed",
                ],
            )
        });
        waiters(&tmux, &["wait-for", "-E", "-l", "window-renamed"], 1);
        run(&tmux, &["rename-window", "a"]).assert_success();
        // Still waiting: the name was not b.
        waiters(&tmux, &["wait-for", "-E", "-l", "window-renamed"], 1);
        run(&tmux, &["rename-window", "b"]).assert_success();
        waiter.join().unwrap().assert_success();
        waiters(&tmux, &["wait-for", "-E", "-l", "window-renamed"], 0);
    });
}

#[test]
fn user_event_from_set_hook() {
    let tmux = server();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| run(&tmux, &["wait-for", "-E", "-v", "@ping"]));
        waiters(&tmux, &["wait-for", "-E", "-l", "@ping"], 1);
        run(&tmux, &["set-hook", "-E", "@ping"]).assert_success();
        let result = waiter.join().unwrap();
        result.assert_success();
        let lines = result.stdout_lines();
        assert!(lines.contains(&"event=@ping".to_string()), "{lines:?}");
        assert!(
            lines.contains(&format!("session={}", tmux.query("#{session_id}"))),
            "{lines:?}"
        );
    });
}

#[test]
fn wake_one_event_waiter() {
    let tmux = server();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| run(&tmux, &["wait-for", "-E", "pane-bell"]));
        let names = waiters(&tmux, &["wait-for", "-E", "-l", "pane-bell"], 1);
        assert!(!names[0].is_empty());
        run(&tmux, &["wait-for", "-E", "-w", &names[0], "pane-bell"]).assert_success();
        waiter.join().unwrap().assert_success();
    });
}

#[test]
fn wake_one_channel_waiter() {
    let tmux = server();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| run(&tmux, &["wait-for", "chan"]));
        let names = waiters(&tmux, &["wait-for", "-l", "chan"], 1);
        run(&tmux, &["wait-for", "-w", &names[0], "chan"]).assert_success();
        waiter.join().unwrap().assert_success();
        // Nothing waits on the channel any more.
        waiters(&tmux, &["wait-for", "-l", "chan"], 0);
    });
}

#[test]
fn event_errors() {
    let tmux = server();
    let r = run(&tmux, &["wait-for", "-E", "no-such-event"]);
    r.assert_failure();
    assert_eq!(r.stderr_str().trim(), "invalid event: no-such-event");

    let r = run(&tmux, &["wait-for", "-E", "-w", "nobody", "pane-bell"]);
    r.assert_failure();
    assert_eq!(r.stderr_str().trim(), "waiter nobody not found");

    let r = run(&tmux, &["set-hook", "-E", "ping"]);
    r.assert_failure();
    assert_eq!(r.stderr_str().trim(), "event name must start with @");
}

#[test]
fn user_hooks_show_in_show_hooks() {
    let tmux = server();
    run(&tmux, &["set-hook", "-g", "@mine", "display hi"]).assert_success();
    run(&tmux, &["set", "-g", "@plain", "x"]).assert_success();
    let hooks = run(&tmux, &["show-hooks", "-g"]).stdout_lines();
    assert!(hooks.iter().any(|l| l.starts_with("@mine")), "{hooks:?}");
    assert!(!hooks.iter().any(|l| l.starts_with("@plain")), "{hooks:?}");
    let options = run(&tmux, &["show-options", "-g"]).stdout_lines();
    assert!(
        !options.iter().any(|l| l.starts_with("@mine")),
        "{options:?}"
    );
    assert!(
        options.iter().any(|l| l.starts_with("@plain")),
        "{options:?}"
    );
    let all = run(&tmux, &["show-options", "-gH"]).stdout_lines();
    assert!(all.iter().any(|l| l.starts_with("@mine")), "{all:?}");
}
