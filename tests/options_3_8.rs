//! Option behaviour that changed in tmux 3.8, checked against 3.8: array
//! items keyed by strings, -F on the show commands and the hook fire
//! count.

use std::time::Duration;

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

/// Numeric keys sort as numbers before other keys, which sort as
/// strings. A numeric key is normalised (`01` is `1`).
#[test]
fn array_keys_can_be_strings() {
    let tmux = server();
    for (key, value) in [("01", "a"), ("x", "b"), ("10", "c"), ("2", "d"), ("B", "e")] {
        ok(&tmux, &["set", "-g", &format!("user-keys[{key}]"), value]);
    }
    assert_eq!(
        lines(&tmux, &["show", "-g", "user-keys"]),
        [
            "user-keys[1] a",
            "user-keys[2] d",
            "user-keys[10] c",
            "user-keys[B] e",
            "user-keys[x] b",
        ]
    );
    assert_eq!(
        lines(&tmux, &["show", "-g", "user-keys[001]"]),
        ["user-keys[1] a"]
    );
    assert_eq!(lines(&tmux, &["show", "-gv", "user-keys[x]"]), ["b"]);
}

#[test]
fn array_keys_with_spaces_and_signs() {
    let tmux = server();
    ok(&tmux, &["set", "-g", "user-keys[a b]", "q"]);
    ok(&tmux, &["set", "-g", "user-keys[-1]", "r"]);
    assert_eq!(
        lines(&tmux, &["show", "-g", "user-keys"]),
        ["user-keys[-1] r", "user-keys[a b] q"]
    );
}

#[test]
fn array_append_and_unset_by_string_key() {
    let tmux = server();
    ok(&tmux, &["set", "-g", "user-keys[x]", "b"]);
    ok(&tmux, &["set", "-ga", "user-keys[x]", "zz"]);
    assert_eq!(lines(&tmux, &["show", "-gv", "user-keys[x]"]), ["bzz"]);
    ok(&tmux, &["set", "-g", "user-keys[1]", "a"]);
    // Appending without a key takes the first free number.
    ok(&tmux, &["set", "-ga", "user-keys", "new"]);
    ok(&tmux, &["set", "-gu", "user-keys[x]"]);
    assert_eq!(
        lines(&tmux, &["show", "-g", "user-keys"]),
        ["user-keys[0] new", "user-keys[1] a"]
    );
}

#[test]
fn array_key_on_option_that_is_not_an_array() {
    let tmux = server();
    assert_eq!(
        error(&tmux, &["set", "-g", "status-left[x]", "a"]),
        "not an array: status-left[x]"
    );
}

#[test]
fn command_alias_with_string_key() {
    let tmux = server();
    ok(
        &tmux,
        &[
            "set",
            "-g",
            "command-alias[foo]",
            "foo=display-message -p hi",
        ],
    );
    assert_eq!(lines(&tmux, &["foo"]), ["hi"]);
    let all = lines(&tmux, &["show", "-g", "command-alias"]);
    assert_eq!(
        all.last().unwrap(),
        "command-alias[foo] \"foo=display-message -p hi\""
    );
}

#[test]
fn show_options_with_format() {
    let tmux = server();
    assert_eq!(
        lines(
            &tmux,
            &[
                "show",
                "-g",
                "-F",
                "[#{option_name}=#{option_value} p=#{option_is_parent} s=#{option_is_string} u=#{option_is_user} h=#{option_is_hook} hv=#{option_has_value} vo=#{option_value_only} a=#{option_is_array}]",
                "base-index",
            ]
        ),
        ["[base-index=0 p=0 s=0 u=0 h=0 hv=1 vo=0 a=0]"]
    );
    assert_eq!(
        lines(
            &tmux,
            &["show", "-gv", "-F", "X#{option_value_only}", "base-index"]
        ),
        ["X1"]
    );
    ok(&tmux, &["set", "-g", "@u", "hi"]);
    assert_eq!(
        lines(
            &tmux,
            &[
                "show",
                "-g",
                "-F",
                "#{option_name} u=#{option_is_user}",
                "@u"
            ]
        ),
        ["@u u=1"]
    );
    assert_eq!(
        lines(
            &tmux,
            &[
                "showw",
                "-g",
                "-F",
                "#{option_name}:#{option_value}",
                "aggressive-resize"
            ]
        ),
        ["aggressive-resize:off"]
    );
    assert_eq!(
        lines(
            &tmux,
            &[
                "show",
                "-A",
                "-F",
                "#{option_name}#{?option_is_parent,*,}",
                "base-index"
            ]
        ),
        ["base-index*"]
    );
}

#[test]
fn show_options_with_format_on_arrays() {
    let tmux = server();
    ok(&tmux, &["set", "-g", "user-keys[x]", "b"]);
    ok(&tmux, &["set", "-g", "user-keys[3]", "a"]);
    assert_eq!(
        lines(
            &tmux,
            &[
                "show",
                "-g",
                "-F",
                "#{option_name}|#{option_value}|#{option_is_array}|#{option_array_key}|#{option_has_array_key}",
                "user-keys",
            ]
        ),
        ["user-keys|a|1|3|1", "user-keys|b|1|x|1"]
    );
    ok(&tmux, &["set", "-gu", "user-keys"]);
    assert_eq!(
        lines(
            &tmux,
            &[
                "show",
                "-g",
                "-F",
                "#{option_name}|#{option_has_value}|#{option_has_array_key}",
                "user-keys",
            ]
        ),
        ["user-keys|0|0"]
    );
}

/// An empty array is not shown as inherited with -A.
#[test]
fn empty_array_is_not_marked_as_parent() {
    let tmux = server();
    assert_eq!(lines(&tmux, &["show", "-A", "user-keys"]), ["user-keys"]);
}

/// Hooks count how often they ran and when, show-hooks -F shows it.
#[test]
fn hook_fire_count_and_time() {
    let tmux = server();
    ok(&tmux, &["set-hook", "-g", "@ev", "set -g @seen yes"]);
    let format = "#{option_name} n=#{hook_fire_count} t=#{?hook_fire_time,set,unset}";
    assert_eq!(
        lines(&tmux, &["show-hooks", "-g", "-F", format, "@ev"]),
        ["@ev n=0 t=unset"]
    );
    ok(&tmux, &["set-hook", "-E", "@ev"]);
    ok(&tmux, &["set-hook", "-E", "@ev"]);
    tmux.wait_for("#{@seen}", |v| v == "yes", T);
    assert_eq!(
        lines(&tmux, &["show-hooks", "-g", "-F", format, "@ev"]),
        ["@ev n=2 t=set"]
    );
    // Table hooks count too, show-hooks lists every hook with -F.
    let all = lines(
        &tmux,
        &[
            "show-hooks",
            "-g",
            "-F",
            "#{option_name} #{hook_fire_count}",
        ],
    );
    assert_eq!(all[0], "@ev 2");
    assert!(all.contains(&"after-bind-key 0".to_string()), "{all:?}");
}

/// show-options does not give hook formats.
#[test]
fn show_options_has_no_hook_formats() {
    let tmux = server();
    ok(&tmux, &["set-hook", "-g", "@ev", "set -g @seen yes"]);
    assert_eq!(
        lines(
            &tmux,
            &[
                "show",
                "-gH",
                "-F",
                "#{option_name} [#{hook_fire_count}]",
                "@ev"
            ]
        ),
        ["@ev []"]
    );
}

/// Setting history-limit applies to panes that exist already.
#[test]
fn history_limit_applies_to_existing_panes() {
    let tmux = server();
    assert_eq!(tmux.query("#{history_limit}"), "2000");
    ok(&tmux, &["set", "-g", "history-limit", "5"]);
    assert_eq!(tmux.query("#{history_limit}"), "5");
    ok(&tmux, &["set", "history-limit", "7"]);
    assert_eq!(tmux.query("#{history_limit}"), "7");
    ok(&tmux, &["set", "-u", "history-limit"]);
    assert_eq!(tmux.query("#{history_limit}"), "5");
}

/// Lines over a lowered limit are dropped from the history.
#[test]
fn lowering_history_limit_drops_lines() {
    let tmux = server();
    ok(&tmux, &["respawn-pane", "-k", "seq 1 50; sleep 1000"]);
    tmux.wait_for("#{history_size}", |v| v == "27", T);
    ok(&tmux, &["set", "-g", "history-limit", "10"]);
    assert_eq!(tmux.query("#{history_size}"), "10");
}
