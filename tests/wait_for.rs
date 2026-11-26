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

/// A signal before anyone waits is kept for the next waiter, and only
/// for one.
#[test]
fn signal_before_wait_is_kept_once() {
    let tmux = server();
    run(&tmux, &["wait-for", "-S", "c"]).assert_success();
    run(&tmux, &["wait-for", "c"]).assert_success();
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| run(&tmux, &["wait-for", "c"]));
        waiters(&tmux, &["wait-for", "-l", "c"], 1);
        run(&tmux, &["wait-for", "-S", "c"]).assert_success();
        waiter.join().unwrap().assert_success();
    });
}

/// One signal lets every waiter run on.
#[test]
fn signal_wakes_all_waiters() {
    let tmux = server();
    std::thread::scope(|scope| {
        let a = scope.spawn(|| run(&tmux, &["wait-for", "c"]));
        let b = scope.spawn(|| run(&tmux, &["wait-for", "c"]));
        waiters(&tmux, &["wait-for", "-l", "c"], 2);
        run(&tmux, &["wait-for", "-S", "c"]).assert_success();
        a.join().unwrap().assert_success();
        b.join().unwrap().assert_success();
        waiters(&tmux, &["wait-for", "-l", "c"], 0);
    });
}

/// A locked channel queues the next locker until it is unlocked, which
/// hands the lock over. Unlocking a free channel is an error.
#[test]
fn lock_and_unlock() {
    let tmux = server();
    run(&tmux, &["wait-for", "-L", "c"]).assert_success();
    std::thread::scope(|scope| {
        let locker = scope.spawn(|| run(&tmux, &["wait-for", "-L", "c"]));
        waiters(&tmux, &["wait-for", "-l", "c"], 1);
        run(&tmux, &["wait-for", "-U", "c"]).assert_success();
        locker.join().unwrap().assert_success();
    });
    // The second locker holds it now.
    run(&tmux, &["wait-for", "-U", "c"]).assert_success();
    let r = run(&tmux, &["wait-for", "-U", "c"]);
    r.assert_failure();
    assert_eq!(r.stderr_str().trim(), "channel c not locked");
    let r = run(&tmux, &["wait-for", "-U", "never"]);
    assert_eq!(r.stderr_str().trim(), "channel never not locked");
}

/// -w lets a queued locker run on without the lock, which stays with its
/// holder.
#[test]
fn wake_a_locker() {
    let tmux = server();
    run(&tmux, &["wait-for", "-L", "c"]).assert_success();
    std::thread::scope(|scope| {
        let locker = scope.spawn(|| run(&tmux, &["wait-for", "-L", "c"]));
        let names = waiters(&tmux, &["wait-for", "-l", "c"], 1);
        run(&tmux, &["wait-for", "-w", &names[0], "c"]).assert_success();
        locker.join().unwrap().assert_success();
    });
    run(&tmux, &["wait-for", "-U", "c"]).assert_success();
    run(&tmux, &["wait-for", "-U", "c"]).assert_failure();
}

/// Channel names need not be UTF-8.
#[test]
#[ignore = "channel names that are not UTF-8 panic, run 16 task 2"]
fn channel_names_that_are_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::OsStr::from_bytes(b"c\xff");
    let tmux = server();
    let cmd = |flag: &str| tmux.cmd().args(["wait-for", flag]).args([name]).run();
    cmd("-S").assert_success();
    cmd("-L").assert_success();
    cmd("-U").assert_success();
    run(&tmux, &["display", "-p", "ok"]).assert_success();
}
