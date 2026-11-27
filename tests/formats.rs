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
#![expect(
    clippy::literal_string_with_formatting_args,
    reason = "tmux format strings, not Rust ones"
)]

use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::time::Duration;

use super::harness::{PtyClient, TmuxTestHarness};

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
        &["set", "-g", "@slash", "/"],
        &["set", "-g", "@dotdot", "/usr/.."],
        &["set", "-g", "@trail", "/usr/bin/"],
        &["set", "-g", "@rel", "a/b"],
        &["set", "-g", "@numstr", "a42b"],
        &["set", "-g", "@special", "a{b}c\td|e;f"],
        &["set", "-g", "@big", "9223372036854775807"],
        &["set", "-g", "@future", "4102444800"],
        // Values that expand themselves, for the loop limit.
        &["set", "-g", "@self", "#{E:@self}"],
        &["set", "-g", "@selfp", "x#{@selfp}"],
        &["set", "-g", "@ping", "#{E:@pong}"],
        &["set", "-g", "@pong", "#{E:@ping}"],
        // Window, pane and array options for the W: and O: loops.
        &["set", "-w", "-t", "alpha:one", "@wopt", "w1"],
        &["set", "-w", "-t", "alpha:two", "@wopt", "w2"],
        &["set", "-p", "-t", "alpha:one.0", "@popt", "p0"],
        &["set", "-g", "user-keys[0]", "x"],
        &["set", "-g", "user-keys[5]", "y"],
        // Global variables for the V: loop.
        &["set-environment", "-g", "FMT_A", "1"],
        &["set-environment", "-g", "FMT_B", "two"],
        &["set-environment", "-gh", "FMT_HIDDEN", "h"],
        &["set-environment", "-gr", "FMT_REMOVED"],
    ];
    for args in setup {
        tmux.cmd().args(*args).run().assert_success();
    }
    // The session environment of alpha holds only FMT_S and FMT_T, so the
    // V: loop does not depend on what update-environment copied from the
    // environment the tests run in.
    let session_environ = tmux
        .cmd()
        .args(["show-environment", "-t", "alpha"])
        .run()
        .stdout_str();
    for line in session_environ.lines() {
        let name = line.trim_start_matches('-');
        let name = name.split('=').next().unwrap_or(name);
        tmux.cmd()
            .args(["set-environment", "-u", "-t", "alpha", name])
            .run()
            .assert_success();
    }
    for (name, value) in [("FMT_S", "s"), ("FMT_T", "t")] {
        tmux.cmd()
            .args(["set-environment", "-t", "alpha", name, value])
            .run()
            .assert_success();
    }
    // A path whose last component is a byte that is not UTF-8.
    tmux.cmd()
        .args([
            OsStr::new("set"),
            OsStr::new("-g"),
            OsStr::new("@badpath"),
            OsStr::from_bytes(b"/a/\xff"),
        ])
        .run()
        .assert_success();
    tmux
}

/// Expands `format` in pane alpha:one.0 and returns the output with only
/// the final newline removed, so spaces at the end stay.
fn expand(tmux: &TmuxTestHarness, format: &str) -> String {
    expand_for(tmux, None, format)
}

/// Like `expand`, for the attached client `client` (display-message -c)
/// when one is given.
fn expand_for(tmux: &TmuxTestHarness, client: Option<&str>, format: &str) -> String {
    let mut args = vec!["display-message", "-t", "alpha:one.0"];
    if let Some(client) = client {
        args.extend(["-c", client]);
    }
    args.extend(["-p", "--", format]);
    let out = tmux.cmd().args(args).run().stdout_str();
    out.strip_suffix('\n').unwrap_or(&out).to_string()
}

/// Expands every format and fails with the list of all cases whose output
/// differs, so one run names every broken format.
fn check(tmux: &TmuxTestHarness, cases: &[(&str, &str)]) {
    check_for(tmux, None, cases);
}

/// Like `check`, for the attached client `client` when one is given.
fn check_for(tmux: &TmuxTestHarness, client: Option<&str>, cases: &[(&str, &str)]) {
    let mut failures = Vec::new();
    for (format, expected) in cases {
        let actual = expand_for(tmux, client, format);
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

/// Escapes: `##` is `#`, `#,` and `#}` are literal, runs of `#` halve,
/// `#` alone at the end is dropped, `##[` and `#[` are left for
/// `format_draw`, single letter aliases (#S, #W and so on) and `#{l:}`
/// keeping its argument unexpanded.
#[test]
fn escapes() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("##", "#"),
            ("a##b", "a#b"),
            ("#,", ","),
            ("#}", "}"),
            ("#{", ""),
            ("a#", "a"),
            ("#", ""),
            ("###", "#"),
            ("####", "##"),
            ("#####", "##"),
            ("######", "###"),
            ("##{session_name}", "#{session_name}"),
            ("###{session_name}", "#alpha"),
            ("####{session_name}", "##{session_name}"),
            ("#{session_name}#", "alpha"),
            ("#{session_name}##", "alpha#"),
            ("#,#}#,", ",},"),
            ("a#:b", "a#:b"),
            ("a#;b", "a#;b"),
            ("#x", "#x"),
            ("#1", "#1"),
            ("#%%", "#%"),
            ("#(", ""),
            ("#)", "#)"),
            ("#[", "#["),
            ("#]", "#]"),
            ("##[fg=red]", "##[fg=red]"),
            ("#[fg=red]x", "#[fg=red]x"),
            ("###[fg=red]", "###[fg=red]"),
            ("#[fg=red", "#[fg=red"),
            ("#[fg=red]#S", "#[fg=red]alpha"),
            ("#[#S]", "#[#S]"),
            ("#[x]#[y]#S", "#[x]#[y]alpha"),
            ("#S", "alpha"),
            ("#W", "one"),
            ("#I", "0"),
            ("#P", "0"),
            ("#D", "%0"),
            ("#F", "*"),
            ("#Q", "#Q"),
            ("#{l:#{session_name}}", "#{session_name}"),
            ("#{l:a#,b}", "a,b"),
            ("#{l:#}x}", "}x"),
            ("#{l:a#{b}c}", "a#{b}c"),
            ("#{l:##}", "#"),
            ("#{l:#:}", ":"),
            ("#{l:a:b}", "a:b"),
            ("#{l:a;b}", "a;b"),
            ("#{l:x#{l:y}z}", "x#{l:y}z"),
            ("a#{l:b,c}d", "ab,cd"),
            ("#{@hash}", "a#b"),
            ("#{E:@hash}", "a#b"),
            ("#{q/e:@hash}", "a##b"),
            ("#{q/h:@hash}", "a##b"),
        ],
    );
}

/// `#{?cond,a,b}`: a condition is a variable name or a format, a name
/// that is not found is false, and nested `#{}` and escaped commas stay
/// in their branch. 3.8's else-if chains are in `else_if_chains`.
#[test]
fn conditionals() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{?session_name,yes,no}", "yes"),
            ("#{?@zero,yes,no}", "no"),
            ("#{?@empty,yes,no}", "no"),
            ("#{?@nonexistent,yes,no}", "no"),
            ("#{?,yes,no}", "no"),
            ("#{?@zero,a}", ""),
            ("#{?@num,a,b}", "a"),
            ("#{?#{==:1,1},t,f}", "t"),
            ("#{?#{@num},t,f}", "t"),
            ("#{?1,t,f}", "f"),
            ("#{?0,t,f}", "f"),
            ("#{?abc,t,f}", "f"),
            ("#{?#{l:1},t,f}", "t"),
            ("#{?#{l:0},t,f}", "f"),
            ("#{?#{@empty},t,f}", "f"),
            ("#{?@num,#{?@zero,a,b},c}", "b"),
            ("#{?@num,#{l:x,y},c}", "x,y"),
            ("#{?@num,x#,y,z}", "x,y"),
            ("#{?@zero,x,y#,z}", "y,z"),
            ("#{?@num,#{session_name},no}", "alpha"),
            ("#{?@zero,no,#{window_name}}", "one"),
            ("#{?#{&&:#{@num},#{@zero}},t,f}", "f"),
            ("#{?#{||:#{@num},#{@zero}},t,f}", "t"),
            (
                "#{?session_name,#{?window_name,#{?pane_index,deep,x},y},z}",
                "x",
            ),
            ("#{?@num,,b}", ""),
            ("#{?@zero,a,}", ""),
            ("#{?@num,}", ""),
            ("#{?a,}", ""),
            ("#{?}", ""),
            ("#{?,}", ""),
            ("#{?,,}", ""),
            ("x#{?@num,a,b", "x"),
        ],
    );
}

/// String comparisons (`==`, `!=`, `<`, `>`, `<=`, `>=`) and `||` and `&&`
/// with two arguments. Arguments are formats, not names, so a bare `@num`
/// is the text itself (and true), unlike the condition of `#{?}`.
#[test]
fn comparisons() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{==:a,a}", "1"),
            ("#{==:a,b}", "0"),
            ("#{!=:a,b}", "1"),
            ("#{!=:a,a}", "0"),
            ("#{<:a,b}", "1"),
            ("#{<:b,a}", "0"),
            ("#{>:a,b}", "0"),
            ("#{>:b,a}", "1"),
            ("#{<=:a,a}", "1"),
            ("#{<=:b,a}", "0"),
            ("#{>=:b,a}", "1"),
            ("#{>=:a,b}", "0"),
            ("#{==:#{@num},42}", "1"),
            ("#{==:@num,@num}", "1"),
            ("#{!=:@num,42}", "1"),
            ("#{<:10,9}", "1"),
            ("#{==:,}", "1"),
            ("#{==:a,}", "0"),
            ("#{==:#{session_name},alpha}", "1"),
            ("#{==:a,b,c}", "0"),
            ("#{==:a}", ""),
            ("#{==:}", ""),
            ("#{||:0,0}", "0"),
            ("#{||:0,1}", "1"),
            ("#{||:1,0}", "1"),
            ("#{||:0,0,1}", "1"),
            ("#{||:@num,@zero}", "1"),
            ("#{||:#{@zero},#{@empty}}", "0"),
            ("#{&&:1,1}", "1"),
            ("#{&&:1,0}", "0"),
            ("#{&&:1,1,1,1}", "1"),
            ("#{&&:#{@num},#{@zero}}", "0"),
            ("#{&&:@num,@num}", "1"),
            ("#{&&:abc,1}", "1"),
        ],
    );
}

/// `n` (length in bytes), `w` (width), `E:` and `T:` (expand the value
/// again, stopping at the loop limit when a value expands itself).
#[test]
fn lengths_and_reexpansion() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{n:@plain}", "11"),
            ("#{n:@wide}", "9"),
            ("#{w:@wide}", "6"),
            ("#{w:@plain}", "11"),
            ("#{n:}", "0"),
            ("#{w:}", "0"),
            ("#{n:hello}", "0"),
            ("#{n;w:@plain}", "2"),
            ("#{n;=3:@plain}", "3"),
            ("#{E:@fmt}", "alpha"),
            ("#{@fmt}", "#{session_name}"),
            ("#{T:@fmt}", "alpha"),
            ("#{E:}", ""),
            ("#{T:}", ""),
            ("#{E:@empty}", ""),
            ("#{E:@nonexistent}", ""),
            ("#{E:@self}", ""),
            ("#{T:@self}", ""),
            ("#{@selfp}", "x#{@selfp}"),
            ("#{E:@selfp}", "xx#{@selfp}"),
            ("#{E:@ping}", ""),
        ],
    );
}

/// `=` (trim to a width, from the end when negative, with an optional
/// marker) and `p` (pad to a width), by display width, so wide characters
/// are never cut in half.
#[test]
fn trim_and_pad() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{=3;n:@plain}", "3"),
            ("#{=3:@plain}", "hel"),
            ("#{=-3:@plain}", "rld"),
            ("#{=/3/...:@plain}", "hel..."),
            ("#{=/-3/...:@plain}", "...rld"),
            ("#{=/20/...:@plain}", "hello world"),
            ("#{=/3/#{@num}:@plain}", "hel42"),
            ("#{=:@plain}", "hello world"),
            ("#{=x:@plain}", "hello world"),
            ("#{=0:@plain}", "hello world"),
            ("#{=/5/:@plain}", "hello"),
            ("#{=5/x:@plain}", "hello world"),
            ("#{=2:@wide}", "日"),
            ("#{=-2:@wide}", "語"),
            ("#{=3:@wide}", "日"),
            ("#{=/2/..:@wide}", "日.."),
            ("#{=3:#{@plain}}", "hel"),
            ("#{=3:hello}", ""),
            ("#{=11:@plain}", "hello world"),
            ("#{=/11/...:@plain}", "hello world"),
            ("#{p10:@num}|", "42        |"),
            ("#{p:@num}|", "42|"),
            ("#{px:@num}|", "42|"),
            ("#{p0:@num}|", "42|"),
            ("#{p1:@num}|", "42|"),
            ("#{p3:@wide}|", "日本語|"),
            ("#{p8:@wide}|", "日本語  |"),
            ("#{p5;=2:@plain}|", "he   |"),
            ("#{=2;p5:@plain}|", "he   |"),
            ("#{p5;n:@num}", "5"),
            ("#{p-10:@num}|", "        42|"),
            ("#{p-8:@wide}|", "  日本語|"),
        ],
    );
}

/// `s/pattern/with/flags`: extended regular expressions, back references,
/// `i` for ignoring case, any wrapper character, several in a row,
/// patterns that do not compile and the empty pattern leaving the value
/// alone, an anchored empty match and a backslash at the end of the
/// replacement.
#[test]
fn substitution() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{s/l/L/:@plain}", "heLLo worLd"),
            ("#{s/L/x/i:@plain}", "hexxo worxd"),
            ("#{s/o/0/:@plain}", "hell0 w0rld"),
            ("#{s|o|0|:@plain}", "hell0 w0rld"),
            ("#{s/(l+)/[\\1]/:@plain}", "he[ll]o wor[l]d"),
            ("#{s/^h/H/:@plain}", "Hello world"),
            ("#{s/o/0/;s/l/L/:@plain}", "heLL0 w0rLd"),
            ("#{s/#{@num}/x/:@numstr}", "axb"),
            ("#{s/o/#{@num}/:@plain}", "hell42 w42rld"),
            ("#{s/a:@plain}", "hello world"),
            ("#{s/l/:@plain}", "hello world"),
            ("#{s/(/x/:@plain}", "hello world"),
            ("#{s/(l)/\\9/:@plain}", "he99o wor9d"),
            ("#{s/:/X/:@path}", ""),
            ("#{s/#:/X/:@path}", "/usr/local/bin/tmux"),
            ("#{s/ /_/:@plain}", "hello_world"),
            ("#{s/\\//|/:@path}", "/usr/local/bin/tmux"),
            ("#{s//x/:@plain}", "hello world"),
            ("#{s/^/>/:@plain}", ">hello world"),
            ("#{s/^h*/X/:@plain}", "Xello world"),
            ("#{s/x*/-/:@plain}", "h-e-l-l-o- -w-o-r-l-d-"),
            ("#{s/o/x\\/:@plain}", "hellx\\ wx\\rld"),
        ],
    );
}

/// `m` (fnmatch), `m/r` (extended regular expression) and `i` for
/// ignoring case. The pattern comes first, the text second.
#[test]
fn matching() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{m:*wor*,#{@plain}}", "1"),
            ("#{m:x*,#{@plain}}", "0"),
            ("#{m:*,}", "1"),
            ("#{m:a?c,abc}", "1"),
            ("#{m:[ab]c,bc}", "1"),
            ("#{m/r:^h.*d$,#{@plain}}", "1"),
            ("#{m/r:^x,#{@plain}}", "0"),
            ("#{m/i:HELLO*,#{@plain}}", "1"),
            ("#{m/ri:WORLD,#{@plain}}", "1"),
            ("#{m/r:WORLD,#{@plain}}", "0"),
            ("#{m/r:(,abc}", "0"),
            ("#{m/r:[,x}", "0"),
            ("#{m:a}", ""),
            ("#{m:}", ""),
        ],
    );
}

/// `q` (shell quoting with backslashes), `q/s` (single quotes), `q/a`
/// (tmux command arguments) and `q/e` and `q/h` (doubling `#`).
#[test]
fn quoting() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{q:@quote}", "a\\'b\\ c\\$d"),
            ("#{q/s:@quote}", "'a'\\''b c$d'"),
            ("#{q/a:@quote}", "\"a'b c\\$d\""),
            ("#{q:@hash}", "a\\#b"),
            ("#{q/a:@special}", "\"a{b}c\\td|e;f\""),
            ("#{q/s:@special}", "'a{b}c\td|e;f'"),
            ("#{q:@empty}", ""),
            ("#{q/a:@empty}", "''"),
            ("#{q/s:@empty}", "''"),
            ("#{q/x:@quote}", "a'b c$d"),
        ],
    );
}

/// `a` (a character code from 32 to 126) and `c` (a colour as six hex
/// digits). The escape sequences of `c/f` and `c/b` are in
/// `colour_escapes`.
#[test]
fn characters_and_colours() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{a:65}", "A"),
            ("#{a:32}", " "),
            ("#{a:126}", "~"),
            ("#{a:31}", ""),
            ("#{a:127}", ""),
            ("#{a:#{@num}}", "*"),
            ("#{a:}", ""),
            ("#{a:65x}", ""),
            ("#{a:-1}", ""),
            ("#{a:@num}", ""),
            ("#{c:red}", "800000"),
            ("#{c:colour1}", "800000"),
            ("#{c:#ff0000}", "ff0000"),
            ("#{c:bogus}", ""),
            ("#{c:colour300}", ""),
            ("#{c:default}", ""),
            ("#{c:terminal}", ""),
            ("#{c:@colour}", ""),
            ("#{c:#{@colour}}", "800000"),
            ("#{c:brightred}", "ff0000"),
            ("#{c:colour255}", "eeeeee"),
            ("#{c/f:bogus}", ""),
        ],
    );
}

/// `N` and `N/w` (a window with that name in the current session) and
/// `N/s` (a session with that name).
#[test]
fn name_checks() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{N:one}", "1"),
            ("#{N:nope}", "0"),
            ("#{N/w:two}", "1"),
            ("#{N/s:beta}", "1"),
            ("#{N/s:gamma}", "0"),
            ("#{N/s:#{session_name}}", "1"),
            ("#{N/x:one}", ""),
        ],
    );
}

/// `t` (ctime), `t/f` (strftime, written `%%%%` because display-message
/// and the argument both pass through strftime), `t/p` (pretty) and the
/// values that are not times. The server runs in UTC.
#[test]
fn time() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{t:@time}", "Tue Nov 14 22:13:20 2023"),
            (
                "#{t/f/%%%%Y-%%%%m-%%%%d %%%%H#:%%%%M:@time}",
                "2023-11-14 22:13",
            ),
            ("#{t/f/%%%%s:@time}", "1700000000"),
            ("#{t/f:@time}", "Tue Nov 14 22:13:20 2023"),
            ("#{t/f/:@time}", "Tue Nov 14 22:13:20 2023"),
            ("#{t/x:@time}", "Tue Nov 14 22:13:20 2023"),
            ("#{t/p:@time}", "Nov23"),
            ("#{t:@neg}", ""),
            ("#{t:@zero}", ""),
            ("#{t:@plain}", ""),
            ("#{t:@empty}", ""),
            ("#{t:@nonexistent}", ""),
        ],
    );
}

/// The loops `S:`, `W:`, `P:` and `L:` with their sort flags,
/// `loop_index` and `loop_last_flag`, a second format for the current
/// item, and loops inside loops. `O:` and `V:` are in `options_loop` and
/// `environment_loop`.
#[test]
fn loops() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{S:#{session_name} }", "alpha beta "),
            ("#{S/n:#{session_name} }", "alpha beta "),
            ("#{S/nr:#{session_name} }", "beta alpha "),
            ("#{S/r:#{session_name} }", "beta alpha "),
            ("#{S/i:#{session_name} }", "alpha beta "),
            ("#{S/x:#{session_name} }", "alpha beta "),
            (
                "#{S:#{loop_index}#{session_name}#{loop_last_flag} }",
                "0alpha0 1beta1 ",
            ),
            ("#{S:#{session_name},[#{session_name}]}", "alphabeta"),
            ("#{W:#{window_name} }", "one two "),
            ("#{W/n:#{window_name} }", "one two "),
            ("#{W/r:#{window_name} }", "two one "),
            ("#{W/nr:#{window_name} }", "two one "),
            ("#{W/i:#{window_name} }", "one two "),
            (
                "#{W:#{loop_index}#{window_name},[#{window_name}]}",
                "[one]1two",
            ),
            ("#{W:#{window_name}#{loop_last_flag} }", "one0 two1 "),
            ("#{P:#{pane_index} }", "0 1 "),
            ("#{P/r:#{pane_index} }", "1 0 "),
            ("#{P/i:#{pane_index} }", "0 1 "),
            ("#{P/ir:#{pane_index} }", "1 0 "),
            ("#{P:#{loop_index}#{loop_last_flag} }", "00 11 "),
            ("#{P:#{pane_index},[#{pane_index}]}", "[0]1"),
            (
                "#{S:#{W:#{session_name}.#{window_name} }}",
                "alpha.one alpha.two beta.three ",
            ),
            ("#{S:#{W:#{P:x}}}", "xxxx"),
            ("#{L:#{client_name}}", ""),
            ("#{L/n:x}", ""),
            ("#{O/x:x}", ""),
            ("#{V/x:x}", ""),
            ("#{O/gp:x}", ""),
            ("#{W/t:x}", "xx"),
            ("#{P/z:#{pane_index} }", "0 1 "),
            ("#{S/t:x}", "xx"),
        ],
    );
}

/// `R` cases that already give 3.8's output: a count that is not a number
/// from 1 to 10000 or a result over 65536 bytes is empty.
#[test]
fn repeat_limits() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{R:ab,0}", ""),
            ("#{R:ab,-1}", ""),
            ("#{R:ab,x}", ""),
            ("#{R:,5}", ""),
            ("#{R:ab}", ""),
            ("#{n:#{R:ab,10001}}", "0"),
            ("#{n:#{R:abcdefgh,8193}}", "0"),
            ("#{n:#{R:a,65536}}", "0"),
            ("#{R:ab,2147483648}", ""),
        ],
    );
}

/// `e` arithmetic: integers unless `f` is given (then two digits or the
/// given precision), `%%` and `m` for the remainder, comparisons that
/// give 1 or 0, and an empty result for a bad operator or operand.
#[test]
fn arithmetic() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{e|+|:1,2}", "3"),
            ("#{e|-|:1,5}", "-4"),
            ("#{e|*|:3,4}", "12"),
            ("#{e|/|:7,2}", "3"),
            ("#{e|%%|:7,3}", "1"),
            ("#{e|m|:7,3}", "1"),
            ("#{e|m|:-7,3}", "-1"),
            ("#{e|/|f|:7,2}", "3.50"),
            ("#{e|/|f|4:7,2}", "3.5000"),
            ("#{e|/|f|0:7,2}", "4"),
            ("#{e|*|f|1:1.25,2}", "2.5"),
            ("#{e|+|f|:#{@float},1}", "3.50"),
            ("#{e|+|:#{@float},1}", "3"),
            ("#{e|+|f|:@float,1}", ""),
            ("#{e|+|:#{@num},#{@neg}}", "35"),
            ("#{e|==|:1,1}", "1"),
            ("#{e|==|:1,2}", "0"),
            ("#{e|!=|:1,2}", "1"),
            ("#{e|<|:1,2}", "1"),
            ("#{e|>|:1,2}", "0"),
            ("#{e|<=|:2,2}", "1"),
            ("#{e|>=|:1,2}", "0"),
            ("#{e|==|f|:1.5,1.5}", "1.00"),
            ("#{e|<|f|:1.5,1.6}", "1.00"),
            ("#{e|==|:1.4,1.6}", "1"),
            ("#{e|+|:1}", ""),
            ("#{e|+|:}", ""),
            ("#{e|+|:1,}", "1"),
            ("#{e|+|:,}", "0"),
            ("#{e:1,2}", ""),
            ("#{e|x|:1,2}", ""),
            ("#{e|+|:a,1}", ""),
            ("#{e|+|:1,a}", ""),
            ("#{e|+|:1 ,2}", ""),
            ("#{e|+|:0x10,1}", "17"),
            ("#{e|+|:1e2,1}", "101"),
            ("#{e|+|x|y|z:1,2}", ""),
            ("#{e|+|f|x:1,2}", ""),
            ("#{e|+|:1,2,3}", ""),
            ("#{e|/|f|:1,3}", "0.33"),
            ("#{e|%%|f|:7.5,2}", "1.50"),
            ("#{e|-|f|:0,0}", "0.00"),
            ("#{e|*|:3000000000,3}", "9000000000"),
            ("#{?#{e|>|:#{@num},40},big,small}", "big"),
            ("#{?#{e|>|:@num,40},big,small}", "small"),
            ("#{e|+|:#{e|*|:2,3},1}", "7"),
        ],
    );
}

/// Pane variables as 3.8 prints them: the `pane_at` flags as 1 or 0, the
/// history sizes in C's struct sizes, `config_files` without a trailing
/// comma, and `C` finding nothing in a pane that shows nothing.
#[test]
fn pane_variables() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{pane_at_top}", "1"),
            ("#{pane_at_bottom}", "0"),
            ("#{pane_at_left}", "1"),
            ("#{pane_at_right}", "1"),
            ("#{P:#{pane_at_top}#{pane_at_bottom} }", "10 01 "),
            ("#{history_bytes}", "480"),
            ("#{history_all_bytes}", "12,480,0,0,0,0"),
            // Every command of the harness passes -f/dev/null.
            ("[#{config_files}]", "[/dev/null,/dev/null]"),
            ("#{pane_tabs}", "8,16,24,32,40,48,56,64,72"),
            ("#{C:zzz}", "0"),
            ("#{C/r:z+}", "0"),
            ("#{C:}", "1"),
        ],
    );
}

/// Without an attached client `I` gives nothing, `A` only works in a
/// status line, and the client loop `L:` finds no client.
#[test]
fn client() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{I/f:RGB}", ""),
            ("#{I/c:Tc}", ""),
            ("#{I/e:TERM}", ""),
            ("#{I:x}", ""),
            ("#{A:a,b,c}", ""),
            ("#{A/3:a,b}", ""),
            ("#{A:}", ""),
            ("#{client_name}", ""),
            ("#{L:x}", ""),
        ],
    );
}

/// Malformed formats. An unterminated `#{` ends the expansion, unknown
/// modifiers expand to nothing, and very deep nesting stops at the loop
/// limit. None of this may crash the server.
#[test]
fn malformed() {
    let tmux = server();
    let deep_conditional = format!("{}x{}", "#{?@num,".repeat(120), ",y}".repeat(120));
    let deep_expand = format!("{}@num{}", "#{E:".repeat(120), "}".repeat(120));
    check(
        &tmux,
        &[
            ("a#{b", "a"),
            ("a#{", "a"),
            ("a#{?", "a"),
            ("x#{?@num,a,b", "x"),
            ("#{}", ""),
            ("#{:}", ""),
            ("#{l:}", ""),
            ("#{l}", ""),
            ("#{X:foo}", ""),
            ("#{zz:@num}", ""),
            ("#{;:@num}", "42"),
            ("#{;;n:@num}", ""),
            ("#{n;:@num}", "2"),
            ("#{:@num}", "42"),
            ("#{n:@num", ""),
            ("#{s/:@plain}", "hello world"),
            ("#{s:@plain}", "hello world"),
            ("#{=/:@plain}", "hello world"),
            ("#{m/:x,x}", "1"),
            (&"#{".repeat(50), ""),
            (&"#{l:".repeat(50), ""),
            ("}}}}}", "}}}}}"),
            (&deep_conditional, ""),
            (&deep_expand, ""),
            ("#{S:#{S:#{S:#{S:x}}}}", "xxxxxxxxxxxxxxxx"),
            ("a#{session_name", "a"),
            ("#{session_name}#{", "alpha"),
            ("#{?@num,#{,b}", ""),
            ("#{=3:", ""),
            ("#{p", ""),
            ("#{e|", ""),
            ("#{e|+|:1,2", ""),
            ("#{@", ""),
            ("#{#{session_name}}", "alpha"),
            ("#{#}}", ""),
            ("#{##}", ""),
            ("#{#,}", ""),
        ],
    );
}

/// The current time in seconds since the epoch.
fn now() -> i64 {
    let since = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock after 1970");
    i64::try_from(since.as_secs()).expect("time fits")
}

/// Times that depend on the clock, compared by structure: `t` of
/// `session_created` is a ctime string that `t/f/%s` turns back into the
/// number, and `t/p` of a time a minute ago is its hour and minute (UTC).
#[test]
fn time_against_clock() {
    let tmux = server();
    let created = expand(&tmux, "#{session_created}");
    assert_eq!(expand(&tmux, "#{t/f/%%%%s:session_created}"), created);
    let ctime = expand(&tmux, "#{t:session_created}");
    assert_eq!(ctime.len(), 24, "ctime string: {ctime:?}");
    assert_eq!(&ctime[3..4], " ", "ctime string: {ctime:?}");

    let recent = now() - 60;
    tmux.cmd()
        .args(["set", "-g", "@recent", &recent.to_string()])
        .run()
        .assert_success();
    let expected = format!("{:02}:{:02}", recent % 86400 / 3600, recent % 3600 / 60);
    assert_eq!(expand(&tmux, "#{t/p:@recent}"), expected);
}

/// 3.8's `t/r` (time since, in the largest two units) and `t/d` (seconds
/// since). The time is 1 hour, 2 minutes and 5 seconds ago, so `t/r` is
/// the same for a minute.
#[test]
fn time_relative() {
    let tmux = server();
    let then = now() - 3725;
    tmux.cmd()
        .args(["set", "-g", "@then", &then.to_string()])
        .run()
        .assert_success();
    assert_eq!(expand(&tmux, "#{t/r:@then}"), "1h2m");
    let seconds: i64 = expand(&tmux, "#{t/d:@then}")
        .parse()
        .expect("t/d is a number");
    assert!((3725..3735).contains(&seconds), "t/d gave {seconds}");
    let days = expand(&tmux, "#{t/r:@time}");
    assert!(
        days.ends_with('h') && days.contains('d'),
        "t/r of a time years ago is days and hours: {days:?}"
    );
}

/// With an attached client (TERM=screen, a terminal that never answers
/// queries), `L:` loops over it, `I/e` finds nothing for a variable it does
/// not have, and `A` still gives nothing outside a status line.
#[test]
fn attached_client() {
    let tmux = server();
    let _client = PtyClient::attach_to(&tmux, "alpha", 80, 24);
    let name = tmux.wait_for(
        "#{L:#{client_name}}",
        |name| !name.is_empty(),
        Duration::from_secs(5),
    );
    check_for(
        &tmux,
        Some(&name),
        &[
            ("#{I/e:NOPE}", ""),
            (
                "#{L:#{client_termname}:#{loop_index}#{loop_last_flag} }",
                "screen:01 ",
            ),
            ("#{A:a,b,c}", ""),
            ("#{c:red}", "800000"),
        ],
    );
}

/// 3.8's client modifiers, with an attached client as in
/// `attached_client`: `I/e` reads the client's environment, `I/f` its
/// terminal features and `I/c` its terminfo capabilities, `c/f` and `c/b`
/// give escape sequences for its terminal. RGB is left out because
/// COLORTERM from the environment the tests run in turns it on.
#[test]
fn attached_client_3_8() {
    let tmux = server();
    let _client = PtyClient::attach_to(&tmux, "alpha", 80, 24);
    let name = tmux.wait_for(
        "#{L:#{client_name}}",
        |name| !name.is_empty(),
        Duration::from_secs(5),
    );
    check_for(
        &tmux,
        Some(&name),
        &[
            ("#{I/e:TERM}", "screen"),
            ("#{I/f:title}", "1"),
            ("#{I/f:bogus}", "0"),
            ("#{I/c:Tc}", "0"),
            // Without an argument I is no modifier, TERM is found in the global
            // environment.
            ("#{I:TERM}", "screen"),
            ("#{c/f:red}", "\u{1b}[31m"),
            ("#{c/b:#123456}", "\u{1b}[48;2;18;52;86m"),
            ("#{c/f:none}", "\u{1b}[0m"),
        ],
    );
}

/// Option values that are not UTF-8 (`x` and two bytes of a three byte
/// character, a path ending in 0xff) keep their bytes in 3.8: `n` counts
/// them, `w` gives them no width, and `=` and `p` work around them.
#[ignore = "behaviour differs from 3.8: option values that are not UTF-8 lose their bytes, formats run"]
#[test]
fn values_not_utf8() {
    let tmux = server();
    tmux.cmd()
        .args([
            OsStr::new("set"),
            OsStr::new("-g"),
            OsStr::new("@trunc"),
            OsStr::from_bytes(b"x\xe6\x97"),
        ])
        .run()
        .assert_success();
    check(
        &tmux,
        &[
            ("#{n:@badpath}", "4"),
            ("#{n:#{b:@badpath}}", "1"),
            ("#{n:@trunc}", "3"),
            ("#{w:@trunc}", "1"),
            ("#{=1:@trunc}", "x"),
            ("#{n:#{=-1:@trunc}}", "3"),
            ("#{n:#{p5:@trunc}}", "7"),
            ("#{q/a:@trunc}", "x\\346\\227"),
            ("#{n:#{s/x/y/:@trunc}}", "3"),
            ("#{m:x*,#{@trunc}}", "1"),
        ],
    );
}

/// 3.8's limits. The precision of `e` is between -100 and 100 (a negative
/// one is printf's default of 6), outside that the result is empty. The
/// widths of `=` and `p` are between -10000 and 10000, outside that they
/// are ignored.
#[test]
fn limits() {
    let tmux = server();
    let hundred = format!("3.{}", "0".repeat(100));
    let padded = format!("42{}", " ".repeat(9998));
    check(
        &tmux,
        &[
            ("#{=10000:@plain}", "hello world"),
            ("#{=-10000:@plain}", "hello world"),
            ("#{=10001:@plain}", "hello world"),
            ("#{=-10001:@plain}", "hello world"),
            ("#{=-2147483648:@plain}", "hello world"),
            ("#{=-2147483647:@plain}", "hello world"),
            ("#{=/-2147483648/...:@plain}", "hello world"),
            ("#{p10000:@num}", &padded),
            ("#{p10001:@num}|", "42|"),
            ("#{p-10001:@num}|", "42|"),
            ("#{p2000000000:@num}|", "42|"),
            ("#{p-2147483648:@num}|", "42|"),
            ("#{e|+|f|-3:1234,0}", "1234.000000"),
            ("#{e|+|f|100:1,2}", &hundred),
            ("#{e|+|f|101:1,2}", ""),
            ("#{e|+|f|-101:1,2}", ""),
            ("#{e|+|f|2147483647:1,2}", ""),
            ("#{e|+|:1,2}", "3"),
        ],
    );
}

/// 3.8 stops expanding a format after 100 milliseconds. Thirty nested
/// session loops over two sessions would print 2^30 characters, the time
/// limit cuts that short (how short depends on the machine).
#[test]
fn time_limit() {
    let tmux = server();
    let format = format!("{}x{}", "#{S:".repeat(30), "}".repeat(30));
    let start = std::time::Instant::now();
    let out = expand(&tmux, &format);
    assert!(out.len() < 1 << 30, "{} bytes", out.len());
    assert!(
        out.bytes().all(|b| b == b'x'),
        "only x: {:?}",
        &out[..out.len().min(40)]
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "took {:?}",
        start.elapsed()
    );
}

/// `b` and `d` are POSIX basename and dirname, applied to a value found by
/// name. An empty value, a value of only slashes and ".." are where they
/// differ from Rust's Path.
#[test]
fn basename_and_dirname() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{b:@path}", "tmux"),
            ("#{d:@path}", "/usr/local/bin"),
            ("#{b:@empty}", "."),
            ("#{d:@empty}", "."),
            ("#{b:@slash}", "/"),
            ("#{d:@slash}", "/"),
            ("#{b:@dotdot}", ".."),
            ("#{d:@dotdot}", "/usr"),
            ("#{b:@trail}", "bin"),
            ("#{d:@trail}", "/usr"),
            ("#{b:@rel}", "b"),
            ("#{d:@rel}", "a"),
            ("#{b:@plain}", "hello world"),
            ("#{d:@plain}", "."),
            ("#{b;d:@path}", "."),
            ("#{d;b:@path}", "."),
            ("#{b:@nonexistent}|", "|"),
            ("#{d:@badpath}", "/a"),
            // A literal is not looked up, so there is nothing to cut.
            ("#{b:hello/world}", ""),
        ],
    );
}

/// 3.8's `#{?c1,a,c2,b,c}` else-if chains, and `#{?c,a}` with an empty
/// else.
#[test]
fn else_if_chains() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{?@num,a}", "a"),
            ("#{?@zero,a,@num,b,c}", "b"),
            ("#{?@zero,a,@zero,b,c}", "c"),
            ("#{?@zero,a,@zero,b}", ""),
            ("#{?@zero,a,@zero,b,@num,c,d}", "c"),
            ("#{?@zero,a,@zero,b,@zero,c,d}", "d"),
            ("#{?a}", "a"),
        ],
    );
}

/// 3.8's `||` and `&&` take one, two or more arguments, and none is 0.
#[test]
fn boolean_operators_any_arity() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{||:0,0,0,0}", "0"),
            ("#{||:}", "0"),
            ("#{||:1}", "1"),
            ("#{&&:1,1,0}", "0"),
            ("#{&&:1}", "1"),
            ("#{&&:0}", "0"),
            ("#{&&:}", "0"),
        ],
    );
}

/// 3.8's `!` and `!!`, which expand their argument and give 1 or 0.
#[test]
fn not_operators() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{!:0}", "1"),
            ("#{!:1}", "0"),
            ("#{!:}", "1"),
            ("#{!:abc}", "0"),
            ("#{!:@num}", "0"),
            ("#{!:#{@num}}", "0"),
            ("#{!:#{@zero}}", "1"),
            ("#{!!:abc}", "1"),
            ("#{!!:0}", "0"),
            ("#{!!:}", "0"),
            ("#{!!:#{@num}}", "1"),
            ("#{!;!:1}", "0"),
            ("#{!:#{==:a,a}}", "0"),
        ],
    );
}

/// Since 3.8, `q` also escapes `{`, `}`, newline and tab.
#[test]
fn shell_quoting_3_8() {
    let tmux = server();
    check(&tmux, &[("#{q:@special}", "a\\{b\\}c\\\td\\|e\\;f")]);
}

/// 3.8's `c/f` and `c/b` give the escape sequence for a foreground or
/// background colour, and `none` resets.
#[test]
fn colour_escapes() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{c/f:red}", "\u{1b}[31m"),
            ("#{c/b:red}", "\u{1b}[41m"),
            ("#{c/f:none}", "\u{1b}[0m"),
            ("#{c/f:default}", "\u{1b}[39m"),
            ("#{c/b:#123456}", "\u{1b}[48;2;18;52;86m"),
            ("#{c/f:colour200}", "\u{1b}[38;5;200m"),
        ],
    );
}

/// A time that localtime cannot convert prints nothing in 3.8.
#[test]
fn times_out_of_range() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{t:@big}", ""),
            ("#{t/p:@big}", ""),
            ("#{t/f/%%%%Y:@big}", ""),
        ],
    );
}

/// `t/r` is empty for a time in the future or out of range.
#[test]
fn relative_time_future() {
    let tmux = server();
    check(&tmux, &[("#{t/r:@big}", ""), ("#{t/r:@future}", "")]);
}

/// 3.8 adds `window_before_active`, `window_after_active` and the
/// `next_` and `prev_` window index, active flag and user options to each
/// item of a `W:` loop.
#[test]
fn window_neighbours() {
    let tmux = server();
    check(
        &tmux,
        &[
            (
                "#{W:#{window_name}:#{window_before_active}#{window_after_active}:#{next_window_index}:#{prev_window_index}:#{next_window_active}:#{prev_window_active} }",
                "one:00:1::0: two:01::0::1 ",
            ),
            ("#{W:#{prev_@wopt}|#{next_@wopt} }", "|w2 w1| "),
        ],
    );
}

/// 3.8's `O:` loop over the options of a scope (`s`, `w`, `p`, `g`, `v`),
/// with array items, user options and hooks.
#[test]
fn options_loop() {
    let tmux = server();
    check(
        &tmux,
        &[
            (
                "#{O:#{option_name}=#{option_value} }",
                "default-size=80x24 ",
            ),
            (
                "#{O/g:#{?#{m:@*,#{option_name}},#{option_name} ,}}",
                "@badpath @big @colour @dotdot @empty @float @fmt @future @hash @neg @num @numstr @path @ping @plain @pong @quote @rel @self @selfp @slash @special @time @trail @wide @zero ",
            ),
            ("#{O/s:#{option_name} }", "default-size "),
            (
                "#{O/w:#{option_name}=#{option_value}:#{option_is_user} }",
                "@wopt=w1:1 automatic-rename=off:0 ",
            ),
            ("#{O/p:#{option_name}=#{option_value} }", "@popt=p0 "),
            (
                "#{O/gw:#{?#{==:#{option_name},@wopt},#{option_value},}}",
                "",
            ),
            (
                "#{O/g:#{?#{==:#{option_name},status-left},#{option_value}|#{option_is_array}|#{option_is_user}|#{option_is_hook},}}",
                "[#{session_name}] |0|0|0",
            ),
            (
                "#{O/v:#{?#{==:#{option_name},user-keys},#{option_array_key}=#{option_value}:#{option_array_first}#{option_array_last}#{option_array_count}:#{option_is_array} ,}}",
                "0=x:102:1 5=y:012:1 ",
            ),
            (
                "#{O/v:#{?#{==:#{option_name},user-keys},#{option_array_index}|,}}",
                "0|5|",
            ),
            (
                "#{O/gs:#{?#{==:#{option_name},@num},#{option_value}:#{option_is_user},}}",
                "42:1",
            ),
            (
                "#{O/v:#{?#{==:#{option_name},escape-time},#{option_value},}}",
                "10",
            ),
            (
                "#{O/g:#{?#{==:#{option_name},after-new-window},#{option_is_hook}:#{option_is_array},}}",
                "1:1",
            ),
            (
                "#{O/g:#{?#{==:#{option_name},@zero},#{loop_last_flag},}}",
                "0",
            ),
        ],
    );
}

/// 3.8's `V:` loop over the session (`s`), global (`g`) or client (`c`)
/// environment. The session environment holds only `FMT_S` and `FMT_T`.
#[test]
fn environment_loop() {
    let tmux = server();
    check(
        &tmux,
        &[
            (
                "#{V/g:#{?#{m:FMT*,#{environ_name}},#{environ_name}=#{environ_value}:#{environ_hidden}#{environ_removed} ,}}",
                "FMT_A=1:00 FMT_B=two:00 FMT_HIDDEN=h:10 FMT_REMOVED=:01 ",
            ),
            ("#{V/c:#{?#{==:#{environ_name},TERM},y,}}", "y"),
            (
                "#{V:#{environ_name}=#{environ_value}:#{loop_index}#{loop_last_flag} }",
                "FMT_S=s:00 FMT_T=t:11 ",
            ),
            ("#{V/s:#{environ_name} }", "FMT_S FMT_T "),
        ],
    );
}

/// 3.8's `R:text,count`, with arguments that are formats and its limits.
#[test]
fn repeat() {
    let tmux = server();
    check(
        &tmux,
        &[
            ("#{R:ab,3}", "ababab"),
            ("#{R:ab,1}", "ab"),
            ("#{R:#{@num},2}", "4242"),
            ("#{R:a#,b,2}", "a,ba,b"),
            ("#{n:#{R:ab,3}}", "6"),
            (
                "#{R:ab,#{@num}}",
                "abababababababababababababababababababababababababababababababababababababababababab",
            ),
            ("#{n:#{R:ab,10000}}", "20000"),
            ("#{n:#{R:abcdefgh,8192}}", "65536"),
            ("#{R:@num,2}", "@num@num"),
        ],
    );
}

/// 3.8's status-format has three lines, so the array loop of `O:` finds
/// three keys. tmux-rs still has 3.5's default (Track 3, switch-mode and
/// the status lines).
#[ignore = "3.8 status-format default: Track 3, formats run"]
#[test]
fn options_loop_status_format() {
    let tmux = server();
    check(
        &tmux,
        &[(
            "#{O/g:#{?#{==:#{option_name},status-format},#{option_array_key},}}",
            "012",
        )],
    );
}
