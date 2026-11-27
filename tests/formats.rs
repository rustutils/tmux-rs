//! Format expansion as tmux 3.8 does it, case by case.
//!
//! Every test sets up the same small server (see `server`) and expands
//! formats with `display-message -p`. Values to probe a modifier on are
//! held in user options, because a modifier applied to a literal string
//! (`#{l:...}`) returns before most other modifiers run.
//!
//! The expected values were checked against tmux 3.8 (`TMUX_SERVER_BIN` and
//! `TMUX_CLIENT_BIN` set to /usr/bin/tmux). Cases that 3.8 passes and tmux-rs
//! does not are in the ignored tests at the end, grouped by what is
//! missing, so `--ignored` shows what is left to port.
//!
//! display-message -p passes the format through strftime first, so a `%`
//! meant for a modifier is written `%%`.

use std::time::Duration;

use super::harness::TmuxTestHarness;

/// A server with two sessions, alpha (windows one and two, one has two
/// panes) and beta (window three), every pane running `sleep 1000`, and
/// user options that hold the values the modifiers are tried on. The
/// server runs in UTC so times print the same everywhere.
fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args([
            "-s",
            "alpha",
            "-n",
            "one",
            "-x",
            "80",
            "-y",
            "24",
            "sleep 1000",
        ])
        .env("TZ", "UTC")
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let setup: &[&[&str]] = &[
        &[
            "new-window",
            "-d",
            "-t",
            "alpha:",
            "-n",
            "two",
            "sleep 1000",
        ],
        &["split-window", "-d", "-t", "alpha:one", "sleep 1000"],
        &[
            "new-session",
            "-d",
            "-s",
            "beta",
            "-n",
            "three",
            "-x",
            "80",
            "-y",
            "24",
            "sleep 1000",
        ],
        &["set", "-g", "@plain", "hello world"],
        &["set", "-g", "@path", "/usr/local/bin/tmux"],
        &["set", "-g", "@num", "42"],
        &["set", "-g", "@neg", "-7"],
        &["set", "-g", "@zero", "0"],
        &["set", "-g", "@empty", ""],
        &["set", "-g", "@fmt", "#{session_name}"],
        &["set", "-g", "@time", "1700000000"],
        &["set", "-g", "@quote", "a'b c$d"],
        &["set", "-g", "@wide", "日本語"],
        &["set", "-g", "@hash", "a#b"],
        &["set", "-g", "@colour", "red"],
        &["set", "-g", "@float", "2.5"],
    ];
    for args in setup {
        tmux.cmd().args(*args).run().assert_success();
    }
    tmux
}

/// Expands `format` in pane alpha:one.0 and returns the output with only
/// the final newline removed, so spaces at the end stay.
fn expand(tmux: &TmuxTestHarness, format: &str) -> String {
    let result = tmux
        .cmd()
        .args(["display-message", "-t", "alpha:one.0", "-p", format])
        .run();
    let out = result.stdout_str();
    out.strip_suffix('\n').unwrap_or(&out).to_string()
}

/// Expands every format and fails with the list of all cases whose output
/// differs, so one run names every broken format.
fn check(tmux: &TmuxTestHarness, cases: &[(&str, &str)]) {
    let mut failures = Vec::new();
    for (format, expected) in cases {
        let actual = expand(tmux, format);
        if actual != *expected {
            failures.push(format!(
                "  {format:?}: expected {expected:?}, got {actual:?}"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} formats differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

/// 3.8's limits: the precision of `e` is between -100 and 100 (a negative
/// one is printf's default of 6), outside that the result is empty.
#[test]
fn limits() {
    let tmux = server();
    let hundred = format!("3.{}", "0".repeat(100));
    check(
        &tmux,
        &[
            ("#{e|+|f|-3:1234,0}", "1234.000000"),
            ("#{e|+|f|100:1,2}", &hundred),
            ("#{e|+|f|101:1,2}", ""),
            ("#{e|+|f|-101:1,2}", ""),
            ("#{e|+|f|2147483647:1,2}", ""),
            ("#{e|+|:1,2}", "3"),
        ],
    );
}
