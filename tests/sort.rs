//! Sort orders: -O and -r on the list commands, switch-client -O, next and
//! previous session, and the sort arguments of the format loops. Checked
//! against tmux 3.8.

use std::time::Duration;

use harness::{PtyClient, TmuxTestHarness};

use super::harness;

/// Runs a command and returns its output lines.
fn lines(tmux: &TmuxTestHarness, args: &[&str]) -> Vec<String> {
    let result = tmux.cmd().args(args).run();
    result.assert_success();
    result.stdout_lines()
}

/// Runs a command that should fail and returns its error output.
fn error(tmux: &TmuxTestHarness, args: &[&str]) -> String {
    let result = tmux.cmd().args(args).run();
    result.assert_failure();
    result.stderr_str().trim().to_string()
}

/// A server whose first session "b" runs `sleep 1000`.
fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "b", "-x", "80", "-y", "24", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux
}

#[test]
fn list_buffers_orders() {
    let tmux = server();
    for (name, data) in [("b", "xxx"), ("a", "xxxxx"), ("c", "x")] {
        tmux.cmd()
            .args(["set-buffer", "-b", name, data])
            .run()
            .assert_success();
    }
    let ls = |extra: &[&str]| {
        let mut args = vec!["list-buffers", "-F", "#{buffer_name}"];
        args.extend_from_slice(extra);
        lines(&tmux, &args).join(" ")
    };
    // Without -O the list is newest first and -r does nothing.
    assert_eq!(ls(&[]), "c a b");
    assert_eq!(ls(&["-r"]), "c a b");
    assert_eq!(ls(&["-O", "name"]), "a b c");
    assert_eq!(ls(&["-O", "NAME", "-r"]), "c b a");
    assert_eq!(ls(&["-O", "creation"]), "c a b");
    assert_eq!(ls(&["-O", "size"]), "c b a");
    assert_eq!(ls(&["-O", "order", "-r"]), "b a c");
    // Orders buffers do not use fall back to the name.
    assert_eq!(ls(&["-O", "activity"]), "a b c");
    assert_eq!(
        error(&tmux, &["list-buffers", "-O", "bogus"]),
        "invalid sort order"
    );
}

#[test]
fn list_sessions_orders() {
    let tmux = server();
    tmux.cmd()
        .args(["new-session", "-d", "-s", "a", "sleep 1000"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["new-session", "-d", "-s", "c", "sleep 1000"])
        .run()
        .assert_success();
    let ls = |extra: &[&str]| {
        let mut args = vec!["list-sessions", "-F", "#{session_name}:#{line}"];
        args.extend_from_slice(extra);
        lines(&tmux, &args).join(" ")
    };
    assert_eq!(ls(&[]), "a:0 b:1 c:2");
    assert_eq!(ls(&["-r"]), "a:0 b:1 c:2");
    assert_eq!(ls(&["-O", "index"]), "b:0 a:1 c:2");
    assert_eq!(ls(&["-O", "key", "-r"]), "c:0 a:1 b:2");
    assert_eq!(ls(&["-O", "title", "-r"]), "c:0 b:1 a:2");
    assert_eq!(ls(&["-O", "creation"]), "b:0 a:1 c:2");
    assert_eq!(ls(&["-O", "order", "-r"]), "c:0 b:1 a:2");
    assert_eq!(
        error(&tmux, &["list-sessions", "-O", "x"]),
        "invalid sort order"
    );
}

#[test]
fn list_windows_orders() {
    let tmux = server();
    tmux.cmd()
        .args(["rename-window", "-t", "b:0", "zz"])
        .run()
        .assert_success();
    for name in ["aa", "mm"] {
        tmux.cmd()
            .args(["new-window", "-d", "-t", "b:", "-n", name, "sleep 1000"])
            .run()
            .assert_success();
    }
    tmux.cmd()
        .args(["new-session", "-d", "-s", "a", "-n", "kk", "sleep 1000"])
        .run()
        .assert_success();
    let ls = |extra: &[&str]| {
        let mut args = vec![
            "list-windows",
            "-t",
            "b",
            "-F",
            "#{window_index}:#{window_name}",
        ];
        args.extend_from_slice(extra);
        lines(&tmux, &args).join(" ")
    };
    assert_eq!(ls(&[]), "0:zz 1:aa 2:mm");
    assert_eq!(ls(&["-O", "name"]), "1:aa 2:mm 0:zz");
    assert_eq!(ls(&["-O", "index", "-r"]), "2:mm 1:aa 0:zz");
    assert_eq!(ls(&["-O", "creation"]), "0:zz 1:aa 2:mm");
    // Equal sizes fall back to the name.
    assert_eq!(ls(&["-O", "size"]), "1:aa 2:mm 0:zz");
    assert_eq!(
        lines(
            &tmux,
            &["list-windows", "-a", "-O", "name", "-F", "#{window_name}"]
        )
        .join(" "),
        "aa kk mm zz"
    );
    assert_eq!(
        error(&tmux, &["list-windows", "-O", "x"]),
        "invalid sort order"
    );
}

#[test]
fn list_windows_all_template() {
    let tmux = server();
    tmux.cmd()
        .args(["rename-window", "-t", "b:0", "sleep"])
        .run()
        .assert_success();
    let out = lines(&tmux, &["list-windows", "-a"]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0], "b:0: sleep* (1 panes) [80x24] ");
}

#[test]
fn list_panes_orders() {
    let tmux = server();
    for _ in 0..2 {
        tmux.cmd()
            .args(["split-window", "-d", "-t", "b:0", "sleep 1000"])
            .run()
            .assert_success();
    }
    for (pane, title) in [("0", "two"), ("1", "three"), ("2", "one")] {
        tmux.cmd()
            .args(["select-pane", "-t", &format!("b:0.{pane}"), "-T", title])
            .run()
            .assert_success();
    }
    let ls = |extra: &[&str]| {
        let mut args = vec![
            "list-panes",
            "-t",
            "b:0",
            "-F",
            "#{pane_index}:#{pane_title}",
        ];
        args.extend_from_slice(extra);
        lines(&tmux, &args).join(" ")
    };
    assert_eq!(ls(&[]), "0:two 1:three 2:one");
    assert_eq!(ls(&["-O", "name"]), "2:one 1:three 0:two");
    assert_eq!(ls(&["-O", "index", "-r"]), "2:one 1:three 0:two");
    assert_eq!(ls(&["-O", "z"]), "2:one 1:three 0:two");
    assert_eq!(ls(&["-O", "order", "-r"]), "2:one 1:three 0:two");
    assert_eq!(
        error(&tmux, &["list-panes", "-O", "x"]),
        "invalid sort order"
    );
}

#[test]
fn list_panes_creation_is_pane_id() {
    let tmux = server();
    // The new pane goes before the first one, so layout order and
    // creation order differ.
    tmux.cmd()
        .args(["split-window", "-b", "-d", "-t", "b:0", "sleep 1000"])
        .run()
        .assert_success();
    let ls = |extra: &[&str]| {
        let mut args = vec!["list-panes", "-t", "b:0", "-F", "#{pane_index}"];
        args.extend_from_slice(extra);
        lines(&tmux, &args).join(" ")
    };
    assert_eq!(ls(&[]), "0 1");
    assert_eq!(ls(&["-O", "creation"]), "1 0");
}

#[test]
fn list_keys_orders() {
    let tmux = server();
    for key in ["b", "M-a", "a", "C-a"] {
        tmux.cmd()
            .args(["bind-key", "-T", "sorttest", key, "display-message", key])
            .run()
            .assert_success();
    }
    let ls = |extra: &[&str]| {
        let mut args = vec!["list-keys", "-T", "sorttest", "-F", "#{key_string}"];
        args.extend_from_slice(extra);
        lines(&tmux, &args).join(" ")
    };
    assert_eq!(ls(&[]), "a b M-a C-a");
    assert_eq!(ls(&["-r"]), "a b M-a C-a");
    assert_eq!(ls(&["-O", "index", "-r"]), "C-a M-a b a");
    // Ctrl is a higher modifier bit than Meta.
    assert_eq!(ls(&["-O", "modifier"]), "a b M-a C-a");
    assert_eq!(
        error(&tmux, &["list-keys", "-O", "x"]),
        "invalid sort order"
    );
}

#[test]
fn list_keys_default_format() {
    let tmux = server();
    tmux.cmd()
        .args(["bind-key", "-T", "sorttest", "a", "display-message", "hi"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["bind-key", "-r", "-T", "sorttest", "F12", "kill-pane"])
        .run()
        .assert_success();
    assert_eq!(
        lines(&tmux, &["list-keys", "-T", "sorttest"]),
        [
            "bind-key    -T sorttest a   display-message hi",
            "bind-key -r -T sorttest F12 kill-pane",
        ]
    );
    assert_eq!(
        lines(&tmux, &["list-keys", "-T", "sorttest", "F12"]),
        ["bind-key -r -T sorttest F12 kill-pane"]
    );
    assert_eq!(
        error(&tmux, &["list-keys", "-T", "sorttest", "F11"]),
        "unknown key: F11"
    );
}

#[test]
fn list_keys_notes() {
    let tmux = server();
    tmux.cmd()
        .args([
            "bind-key",
            "-N",
            "say hi",
            "-T",
            "prefix",
            "F11",
            "display-message",
            "hi",
        ])
        .run()
        .assert_success();
    tmux.cmd()
        .args([
            "bind-key",
            "-N",
            "root note",
            "-T",
            "root",
            "F12",
            "display-message",
            "x",
        ])
        .run()
        .assert_success();
    let out = lines(&tmux, &["list-keys", "-N", "-P", "P ", "F11"]);
    assert_eq!(out, ["P  F11 say hi"]);
    // The prefix is shown for every table.
    let out = lines(&tmux, &["list-keys", "-N", "-T", "root", "F12"]);
    assert_eq!(out, ["C-b F12 root note"]);
}

#[test]
fn list_clients_orders() {
    let tmux = server();
    tmux.cmd()
        .args(["new-session", "-d", "-s", "a", "sleep 1000"])
        .run()
        .assert_success();
    let _wide = PtyClient::attach_to(&tmux, "b", 100, 30);
    let _narrow = PtyClient::attach_to(&tmux, "a", 60, 20);
    let ls = |extra: &[&str]| {
        let mut args = vec!["list-clients", "-F", "#{client_width}:#{line}"];
        args.extend_from_slice(extra);
        lines(&tmux, &args).join(" ")
    };
    assert_eq!(ls(&[]), "100:0 60:1");
    assert_eq!(ls(&["-O", "size"]), "60:0 100:1");
    assert_eq!(ls(&["-O", "size", "-r"]), "100:0 60:1");
    assert_eq!(ls(&["-O", "creation", "-r"]), "60:0 100:1");
    // The line number counts clients skipped by -t too.
    assert_eq!(ls(&["-t", "a"]), "60:1");
    assert_eq!(
        error(&tmux, &["list-clients", "-O", "x"]),
        "invalid sort order"
    );
}

/// The session the only attached client is in.
fn client_session(tmux: &TmuxTestHarness) -> String {
    lines(tmux, &["list-clients", "-F", "#{client_session}"]).join(" ")
}

#[test]
fn switch_client_next_and_previous_orders() {
    let tmux = server();
    // IDs: b is $0, c is $1, a is $2.
    for name in ["c", "a"] {
        tmux.cmd()
            .args(["new-session", "-d", "-s", name, "sleep 1000"])
            .run()
            .assert_success();
    }
    let _client = PtyClient::attach_to(&tmux, "a", 80, 24);
    let switch = |extra: &[&str]| {
        let mut args = vec!["switch-client"];
        args.extend_from_slice(extra);
        tmux.cmd().args(args).run().assert_success();
        client_session(&tmux)
    };
    assert_eq!(switch(&["-n"]), "b");
    assert_eq!(switch(&["-n"]), "c");
    assert_eq!(switch(&["-n"]), "a");
    assert_eq!(switch(&["-p"]), "c");
    // By index: b ($0), c ($1), a ($2).
    assert_eq!(switch(&["-n", "-O", "index"]), "a");
    assert_eq!(switch(&["-n", "-O", "index"]), "b");
    assert_eq!(switch(&["-p", "-O", "index"]), "a");
    assert_eq!(
        error(&tmux, &["switch-client", "-n", "-O", "x"]),
        "invalid sort order"
    );
}

#[test]
fn switch_client_next_with_one_session() {
    let tmux = server();
    let _client = PtyClient::attach_to(&tmux, "b", 80, 24);
    tmux.cmd()
        .args(["switch-client", "-n"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["switch-client", "-p"])
        .run()
        .assert_success();
    assert_eq!(client_session(&tmux), "b");
}

#[test]
fn destroy_session_moves_to_next_by_name() {
    let tmux = server();
    // IDs differ from name order so index order would give another answer.
    for name in ["c", "a"] {
        tmux.cmd()
            .args(["new-session", "-d", "-s", name, "sleep 1000"])
            .run()
            .assert_success();
    }
    tmux.cmd()
        .args(["set-option", "-g", "detach-on-destroy", "next"])
        .run()
        .assert_success();
    let _client = PtyClient::attach_to(&tmux, "a", 80, 24);
    tmux.cmd()
        .args(["kill-session", "-t", "a"])
        .run()
        .assert_success();
    tmux.wait_for(
        "#{S:#{session_name}}",
        |s| s == "bc" || s == "cb",
        Duration::from_secs(5),
    );
    assert_eq!(client_session(&tmux), "b");

    tmux.cmd()
        .args(["set-option", "-g", "detach-on-destroy", "previous"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["kill-session", "-t", "b"])
        .run()
        .assert_success();
    assert_eq!(client_session(&tmux), "c");
}

#[test]
fn format_session_loop_orders() {
    let tmux = server();
    for name in ["c", "a"] {
        tmux.cmd()
            .args(["new-session", "-d", "-s", name, "sleep 1000"])
            .run()
            .assert_success();
    }
    // The default is by index (session ID), not by name.
    assert_eq!(tmux.query("#{S:#{session_name}}"), "bca");
    assert_eq!(tmux.query("#{S/n:#{session_name}}"), "abc");
    assert_eq!(tmux.query("#{S/nr:#{session_name}}"), "cba");
    assert_eq!(tmux.query("#{S/r:#{session_name}}"), "acb");
    assert_eq!(tmux.query("#{S/i:#{session_name}}"), "bca");
}

#[test]
fn format_window_loop_orders() {
    let tmux = server();
    tmux.cmd()
        .args(["rename-window", "-t", "b:0", "zz"])
        .run()
        .assert_success();
    for name in ["aa", "mm"] {
        tmux.cmd()
            .args(["new-window", "-d", "-t", "b:", "-n", name, "sleep 1000"])
            .run()
            .assert_success();
    }
    let q = |f: &str| tmux.query(&format!("#{{{f}:#{{window_name}} }}"));
    assert_eq!(q("W"), "zz aa mm");
    assert_eq!(q("W/r"), "mm aa zz");
    assert_eq!(q("W/n"), "aa mm zz");
    assert_eq!(q("W/nr"), "zz mm aa");
}

#[test]
fn format_pane_loop_orders() {
    let tmux = server();
    // Layout order differs from creation order.
    tmux.cmd()
        .args(["split-window", "-b", "-d", "-t", "b:0", "sleep 1000"])
        .run()
        .assert_success();
    let q = |f: &str| tmux.query(&format!("#{{{f}:#{{pane_index}}}}"));
    // The default is creation order (pane ID).
    assert_eq!(q("P"), "10");
    assert_eq!(q("P/r"), "01");
    assert_eq!(q("P/i"), "01");
    assert_eq!(q("P/ir"), "10");
}

#[test]
fn format_loop_index_and_last_flag() {
    let tmux = server();
    tmux.cmd()
        .args(["new-session", "-d", "-s", "a", "sleep 1000"])
        .run()
        .assert_success();
    assert_eq!(
        tmux.query("#{S/n:#{session_name}#{loop_index}#{loop_last_flag} }"),
        "a00 b11"
    );
}

#[test]
fn format_session_loop_active_format() {
    let tmux = server();
    tmux.cmd()
        .args(["new-session", "-d", "-s", "a", "sleep 1000"])
        .run()
        .assert_success();
    let _client = PtyClient::attach_to(&tmux, "a", 80, 24);
    let name = lines(&tmux, &["list-clients", "-F", "#{client_name}"]).join("");
    let out = lines(
        &tmux,
        &[
            "display-message",
            "-p",
            "-c",
            &name,
            "#{S/n:#{session_name} ,(#{session_name}) }",
        ],
    );
    assert_eq!(out, ["(a) b "]);
}
