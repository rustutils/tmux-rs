//! Hook monitors (set-hook -B), checked against tmux 3.8.
//!
//! A hook monitor checks a format once a second and runs its hook when
//! the value changes. The first value a monitor sees is never reported,
//! and a change made before the first check is that first value. So the
//! tests change a value, wait for the hook with a timeout, and change it
//! again until the hook runs ([`fire`]).

use std::time::{Duration, Instant};

use harness::{TmuxResult, TmuxTestHarness};

use super::harness;

const T: Duration = Duration::from_secs(5);
/// Longer than a check, with room for slow runners.
const TICK: Duration = Duration::from_millis(2500);

fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "main", "-n", "first", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(T);
    ok(&tmux, &["set", "-g", "automatic-rename", "off"]);
    tmux
}

fn run(tmux: &TmuxTestHarness, args: &[&str]) -> TmuxResult {
    tmux.cmd().args(args).run()
}

fn ok(tmux: &TmuxTestHarness, args: &[&str]) {
    run(tmux, args).assert_success();
}

fn lines(tmux: &TmuxTestHarness, args: &[&str]) -> Vec<String> {
    let r = run(tmux, args);
    r.assert_success();
    r.stdout_lines()
}

fn error(tmux: &TmuxTestHarness, args: &[&str]) -> String {
    let r = run(tmux, args);
    r.assert_failure();
    r.stderr_str().trim().to_string()
}

/// The value of global user option `name`, empty if it is not set.
fn option(tmux: &TmuxTestHarness, name: &str) -> String {
    tmux.query(&format!("#{{{name}}}"))
}

/// Waits up to `time` for global user option `name` to be set.
fn wait_option(tmux: &TmuxTestHarness, name: &str, time: Duration) -> Option<String> {
    let start = Instant::now();
    loop {
        let v = option(tmux, name);
        if !v.is_empty() {
            return Some(v);
        }
        if start.elapsed() >= time {
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Calls `change` with each value until the hook sets `seen`. Returns
/// what it set and the index of the value that made it run.
fn fire(
    tmux: &TmuxTestHarness,
    seen: &str,
    values: &[&str],
    change: impl Fn(&str),
) -> (String, usize) {
    for (i, value) in values.iter().enumerate() {
        change(value);
        if let Some(v) = wait_option(tmux, seen, TICK) {
            return (v, i);
        }
    }
    panic!("the hook never ran for {values:?}");
}

/// Renames session `first` and then the session it was renamed to.
fn renamer<'a>(tmux: &'a TmuxTestHarness, first: &str) -> impl Fn(&str) + 'a {
    let current = std::cell::RefCell::new(first.to_string());
    move |n| {
        ok(tmux, &["rename-session", "-t", &current.borrow(), n]);
        *current.borrow_mut() = n.to_string();
    }
}

const NAMES: [&str; 4] = ["n1", "n2", "n3", "n4"];

#[test]
fn session_monitor_runs_the_hook_on_a_change() {
    let tmux = server();
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            "@mon::#{session_name}",
            "set -g @seen '#{hook}|#{hook_value}|#{hook_last}|#{hook_session_name}'",
        ],
    );
    let (seen, i) = fire(&tmux, "@seen", &NAMES, |n| {
        ok(&tmux, &["rename-session", n]);
    });
    let parts: Vec<&str> = seen.split('|').collect();
    assert_eq!(parts[0], "@mon");
    assert_eq!(parts[1], NAMES[i]);
    // The last value is one the session had before.
    let before: Vec<&str> = std::iter::once("main")
        .chain(NAMES[..i].iter().copied())
        .collect();
    assert!(before.contains(&parts[2]), "{seen}");
    assert_eq!(parts[3], NAMES[i]);
}

#[test]
fn the_first_value_is_not_reported() {
    let tmux = server();
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            "@mon::#{session_name}",
            "set -g @seen yes",
        ],
    );
    assert_eq!(wait_option(&tmux, "@seen", TICK), None);
}

/// With -T only values that are true run the hook.
#[test]
fn only_true_values_with_t() {
    let tmux = server();
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-T",
            "-B",
            "@t::#{m:a*,#{session_name}}",
            "set -g @seen '#{hook_value}'",
        ],
    );
    let (seen, _) = fire(&tmux, "@seen", &["x1", "a1", "x2", "a2", "x3", "a3"], |n| {
        ok(&tmux, &["rename-session", n]);
    });
    assert_eq!(seen, "1");
    ok(&tmux, &["set", "-gu", "@seen"]);
    ok(&tmux, &["rename-session", "zz"]);
    assert_eq!(wait_option(&tmux, "@seen", TICK), None);
    ok(&tmux, &["rename-session", "aa"]);
    assert_eq!(wait_option(&tmux, "@seen", T).as_deref(), Some("1"));
}

#[test]
fn pane_monitor_has_the_pane_as_target() {
    let tmux = server();
    let (p, w) = (tmux.query("#{pane_id}"), tmux.query("#{window_id}"));
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            &format!("@p:{p}:#{{pane_title}}"),
            "set -g @seen '#{hook_value}|#{hook_pane}|#{hook_window}|#{hook_window_index}|#{hook_session_name}'",
        ],
    );
    let (seen, i) = fire(&tmux, "@seen", &NAMES, |n| {
        ok(&tmux, &["select-pane", "-t", &p, "-T", n]);
    });
    assert_eq!(seen, format!("{}|{p}|{w}|0|main", NAMES[i]));
}

#[test]
fn window_monitor_has_the_window_as_target() {
    let tmux = server();
    let w = tmux.query("#{window_id}");
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            &format!("@w:{w}:#{{window_name}}"),
            "set -g @seen '#{hook_value}|#{hook_window}|#{hook_window_index}|#{hook_pane}'",
        ],
    );
    let (seen, i) = fire(&tmux, "@seen", &NAMES, |n| {
        ok(&tmux, &["rename-window", n]);
    });
    assert_eq!(seen, format!("{}|{w}|0|", NAMES[i]));
}

#[test]
fn all_panes_and_all_windows() {
    let tmux = server();
    ok(&tmux, &["split-window", "-d", "sleep 1000"]);
    let p = tmux.query("#{pane_id}");
    let w = tmux.query("#{window_id}");
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            "@ap:%*:#{pane_title}",
            "set -g @seen_p '#{hook_pane}=#{hook_value}'",
        ],
    );
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            "@aw:@*:#{window_name}",
            "set -g @seen_w '#{hook_window}=#{hook_value}'",
        ],
    );
    let (seen, i) = fire(&tmux, "@seen_p", &NAMES, |n| {
        ok(&tmux, &["select-pane", "-t", &p, "-T", n]);
    });
    assert_eq!(seen, format!("{p}={}", NAMES[i]));
    let (seen, i) = fire(&tmux, "@seen_w", &NAMES, |n| {
        ok(&tmux, &["rename-window", n]);
    });
    assert_eq!(seen, format!("{w}={}", NAMES[i]));
}

/// A global hook watches the first session by name, a session's hook
/// that session.
#[test]
fn which_session_a_monitor_watches() {
    let mut tmux = TmuxTestHarness::new();
    for s in ["bbb", "aaa"] {
        tmux.new_session()
            .args(["-s", s, "sleep 1000"])
            .run()
            .assert_success();
    }
    tmux.wait_ready(T);
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            "@g::#{session_name}",
            "set -g @seen_g '#{hook_value}'",
        ],
    );
    ok(
        &tmux,
        &[
            "set-hook",
            "-t",
            "bbb",
            "-B",
            "@s::#{session_name}",
            "set -g @seen_s '#{hook_value}'",
        ],
    );
    let names = ["a1", "a2", "a3", "a4"];
    let (seen, i) = fire(&tmux, "@seen_g", &names, renamer(&tmux, "aaa"));
    assert_eq!(seen, names[i]);
    assert_eq!(option(&tmux, "@seen_s"), "");
    let names = ["b1", "b2", "b3", "b4"];
    let (seen, i) = fire(&tmux, "@seen_s", &names, renamer(&tmux, "bbb"));
    assert_eq!(seen, names[i]);
    assert_eq!(
        lines(&tmux, &["show-hooks", "-t", names[i], "-B"]),
        ["@s::#{session_name}"]
    );
    assert_eq!(
        lines(&tmux, &["show-hooks", "-g", "-B"]),
        ["@g::#{session_name}"]
    );
}

#[test]
fn show_hooks_lists_monitors() {
    let tmux = server();
    let p = tmux.query("#{pane_id}");
    let w = tmux.query("#{window_id}");
    ok(
        &tmux,
        &["set-hook", "-g", "-B", "@s::#{session_name}", "display x"],
    );
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            &format!("@p:{p}:#{{pane_title}}"),
            "display x",
        ],
    );
    ok(
        &tmux,
        &["set-hook", "-g", "-B", "@ap:%*:#{pane_title}", "display x"],
    );
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            &format!("@w:{w}:#{{window_name}}"),
            "display x",
        ],
    );
    ok(
        &tmux,
        &["set-hook", "-g", "-B", "@aw:@*:#{window_name}", "display x"],
    );
    assert_eq!(
        lines(&tmux, &["show-hooks", "-g", "-B"]),
        [
            "@ap:%*:#{pane_title}".to_string(),
            "@aw:@*:#{window_name}".to_string(),
            format!("@p:{p}:#{{pane_title}}"),
            "@s::#{session_name}".to_string(),
            format!("@w:{w}:#{{window_name}}"),
        ]
    );
    assert_eq!(
        lines(
            &tmux,
            &[
                "show-hooks",
                "-g",
                "-B",
                "-F",
                "#{option_name}|#{hook_monitor_target}|#{hook_monitor_format}|#{hook_fire_count}|#{option_value}|#{option_is_hook}|#{option_is_user}",
                "@ap",
            ]
        ),
        ["@ap|%*|#{pane_title}|0|@ap:%*:#{pane_title}|1|1"]
    );
}

#[test]
fn monitor_fire_count() {
    let tmux = server();
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            "@mon::#{session_name}",
            "set -g @seen yes",
        ],
    );
    let format = "#{hook_fire_count} #{?hook_fire_time,set,unset}";
    assert_eq!(
        lines(&tmux, &["show-hooks", "-g", "-B", "-F", format, "@mon"]),
        ["0 unset"]
    );
    fire(&tmux, "@seen", &NAMES, |n| {
        ok(&tmux, &["rename-session", n]);
    });
    assert_eq!(
        lines(&tmux, &["show-hooks", "-g", "-B", "-F", format, "@mon"]),
        ["1 set"]
    );
}

/// -u -B removes the monitor and keeps the option, set-hook -u removes
/// both.
#[test]
fn removing_monitors() {
    let tmux = server();
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            "@a::#{session_name}",
            "set -g @seen_a yes",
        ],
    );
    ok(
        &tmux,
        &[
            "set-hook",
            "-g",
            "-B",
            "@b::#{session_name}",
            "set -g @seen_b yes",
        ],
    );
    ok(&tmux, &["set-hook", "-gu", "-B", "@a"]);
    ok(&tmux, &["set-hook", "-gu", "@b"]);
    assert!(lines(&tmux, &["show-hooks", "-g", "-B"]).is_empty());
    assert_eq!(
        lines(&tmux, &["show", "-gH", "@a"]),
        ["@a \"set -g @seen_a yes\""]
    );
    assert_eq!(error(&tmux, &["show", "-gH", "@b"]), "invalid option: @b");
    // The option left behind is no longer a hook.
    assert!(lines(&tmux, &["show", "-g"]).contains(&"@a \"set -g @seen_a yes\"".to_string()));
    ok(&tmux, &["rename-session", "other"]);
    assert_eq!(wait_option(&tmux, "@seen_a", TICK), None);
    assert_eq!(option(&tmux, "@seen_b"), "");
}

#[test]
fn monitor_without_a_command() {
    let tmux = server();
    ok(&tmux, &["set-hook", "-g", "-B", "@n::#{session_name}"]);
    assert_eq!(lines(&tmux, &["show", "-gH", "@n"]), ["@n ''"]);
    assert_eq!(
        lines(&tmux, &["show-hooks", "-g", "-B"]),
        ["@n::#{session_name}"]
    );
    // A monitor hook is a hook, not an option.
    assert!(
        !lines(&tmux, &["show", "-g"])
            .iter()
            .any(|l| l.starts_with("@n"))
    );
    assert!(lines(&tmux, &["show-hooks", "-g"]).contains(&"@n ''".to_string()));
}

#[test]
fn monitor_errors() {
    let tmux = server();
    assert_eq!(
        error(&tmux, &["set-hook", "-g", "-B", "mon::x", "a"]),
        "monitor hook name must start with @"
    );
    assert_eq!(
        error(&tmux, &["set-hook", "-g", "-B", "@x:bad:y", "a"]),
        "invalid subscription: @x:bad:y"
    );
    assert_eq!(
        error(&tmux, &["set-hook", "-g", "-B", "@y::x", "a", "b"]),
        "too many arguments"
    );
}

/// A monitor change is an event like any other, so wait-for -E sees it.
#[test]
fn wait_for_sees_monitor_events() {
    let tmux = server();
    let s = tmux.query("#{session_id}");
    ok(&tmux, &["set-hook", "-g", "-B", "@g::#{session_name}", ""]);
    std::thread::scope(|scope| {
        let waiter = scope.spawn(|| run(&tmux, &["wait-for", "-E", "-v", "@g"]));
        let start = Instant::now();
        let mut i = 0;
        // Rename until the monitor reports a change.
        while !waiter.is_finished() {
            assert!(start.elapsed() < Duration::from_secs(10), "no event");
            ok(&tmux, &["rename-session", NAMES[i % NAMES.len()]]);
            i += 1;
            std::thread::sleep(Duration::from_millis(1200));
        }
        let lines = waiter.join().unwrap().stdout_lines();
        assert_eq!(lines[0], "event=@g");
        assert!(lines[1].starts_with("last="), "{lines:?}");
        assert_eq!(lines[2], format!("session={s}"));
        assert!(lines[3].starts_with("value=n"), "{lines:?}");
        assert_eq!(lines.len(), 4, "{lines:?}");
    });
}
