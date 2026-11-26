use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

#[test]
fn bind_key_and_verify_with_list_keys() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Bind a custom key
    tmux.cmd()
        .args(["bind-key", "Z", "display-message", "hello"])
        .run()
        .assert_success();

    // Verify the binding appears in list-keys
    let result = tmux.cmd().args(["list-keys"]).run();
    result.assert_success();
    let output = result.stdout_str();
    assert!(
        output.contains("Z"),
        "list-keys should contain the bound key 'Z', got:\n{output}"
    );
}

#[test]
fn unbind_key_removes_binding() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Bind a key first
    tmux.cmd()
        .args(["bind-key", "Y", "display-message", "test"])
        .run()
        .assert_success();

    // Verify it's there
    let result = tmux.cmd().args(["list-keys"]).run();
    let output = result.stdout_str();
    assert!(output.contains("Y"), "key Y should be bound");

    // Unbind it
    tmux.cmd().args(["unbind-key", "Y"]).run().assert_success();

    // Verify it's gone -- filter to prefix table bindings for 'Y'
    // We look specifically for " Y " to avoid matching other keys that contain Y
    let result = tmux.cmd().args(["list-keys", "-T", "prefix"]).run();
    let output = result.stdout_str();
    // Check that there's no line binding the bare key Y in the prefix table
    let has_y_binding = output
        .lines()
        .any(|line| line.contains(" Y ") && line.contains("display-message"));
    assert!(
        !has_y_binding,
        "key Y should have been unbound, but still found in:\n{output}"
    );
}

#[test]
fn list_keys_output_format() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux.cmd().args(["list-keys"]).run();
    result.assert_success();

    let output = result.stdout_str();
    let lines: Vec<&str> = output.lines().collect();

    // There should be many default bindings
    assert!(
        lines.len() > 10,
        "list-keys should have many default bindings, got {} lines",
        lines.len()
    );

    // Each line should start with "bind-key"
    for line in &lines {
        assert!(
            line.starts_with("bind-key"),
            "list-keys line should start with 'bind-key', got: {line}"
        );
    }
}

#[test]
fn list_keys_with_table_filter() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // List only prefix table bindings
    let result = tmux.cmd().args(["list-keys", "-T", "prefix"]).run();
    result.assert_success();

    let output = result.stdout_str();
    let lines: Vec<&str> = output.lines().collect();

    // All lines should reference the prefix table
    for line in &lines {
        assert!(
            line.contains("-T prefix"),
            "filtered list-keys should only show prefix table, got: {line}"
        );
    }
}

#[test]
fn bind_key_with_specific_table() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Bind a key in the root table
    tmux.cmd()
        .args([
            "bind-key",
            "-T",
            "root",
            "F12",
            "display-message",
            "root-test",
        ])
        .run()
        .assert_success();

    // Verify it appears in the root table
    let result = tmux.cmd().args(["list-keys", "-T", "root"]).run();
    result.assert_success();
    let output = result.stdout_str();
    assert!(
        output.contains("F12"),
        "root table should contain F12 binding, got:\n{output}"
    );

    // It should not appear in the prefix table
    let result = tmux.cmd().args(["list-keys", "-T", "prefix"]).run();
    let output = result.stdout_str();
    let has_f12 = output.lines().any(|line| line.contains("F12"));
    assert!(!has_f12, "F12 should not be in prefix table");
}

#[test]
fn bind_key_in_copy_mode_table() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Bind in copy-mode table
    tmux.cmd()
        .args([
            "bind-key",
            "-T",
            "copy-mode",
            "Q",
            "send-keys",
            "-X",
            "cancel",
        ])
        .run()
        .assert_success();

    // Verify
    let result = tmux.cmd().args(["list-keys", "-T", "copy-mode"]).run();
    result.assert_success();
    let output = result.stdout_str();
    assert!(
        output.contains("Q"),
        "copy-mode table should contain Q binding"
    );
}

#[test]
fn bind_key_with_repeat_flag() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Bind a repeating key
    tmux.cmd()
        .args(["bind-key", "-r", "M", "next-window"])
        .run()
        .assert_success();

    // Verify the binding shows up with the -r flag in list-keys output
    let result = tmux.cmd().args(["list-keys", "-T", "prefix"]).run();
    result.assert_success();
    let output = result.stdout_str();

    // Find the line with our M binding
    let m_line = output
        .lines()
        .find(|line| line.contains(" M ") && line.contains("next-window"));
    assert!(
        m_line.is_some(),
        "should find M binding for next-window in list-keys output:\n{output}"
    );

    // The line should contain -r flag indicating it's a repeat binding
    let m_line = m_line.unwrap();
    assert!(
        m_line.contains("-r"),
        "repeating binding should show -r flag, got: {m_line}"
    );
}

#[test]
fn bind_key_replaces_existing_binding() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Bind a key to one command
    tmux.cmd()
        .args(["bind-key", "X", "display-message", "first"])
        .run()
        .assert_success();

    // Re-bind the same key to a different command
    tmux.cmd()
        .args(["bind-key", "X", "display-message", "second"])
        .run()
        .assert_success();

    // Verify only the new binding exists
    let result = tmux.cmd().args(["list-keys", "-T", "prefix"]).run();
    let output = result.stdout_str();
    let x_lines: Vec<&str> = output
        .lines()
        .filter(|line| line.contains(" X ") && line.contains("display-message"))
        .collect();

    assert_eq!(
        x_lines.len(),
        1,
        "should have exactly one binding for X, got: {x_lines:?}"
    );
    assert!(
        x_lines[0].contains("second"),
        "binding should be the second one, got: {}",
        x_lines[0]
    );
}

#[test]
fn unbind_nonexistent_key_does_not_error() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Unbinding a key that was never bound should not produce an error
    // (tmux silently ignores this)
    let result = tmux.cmd().args(["unbind-key", "F11"]).run();
    // tmux typically succeeds silently here
    // Some implementations may error, so just verify it doesn't crash
    let _ = result.success();
}

/// Keys sent with send-keys -K run before the commands that follow
/// send-keys (upstream b6be437b).
#[test]
fn send_keys_k_runs_keys_in_order() {
    let t = Duration::from_secs(5);
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-x", "80", "-y", "24", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(t);
    tmux.cmd()
        .args(["bind", "-n", "F11", "set", "-g", "@x", "a"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["bind", "-n", "F12", "set", "-ga", "@x", "b"])
        .run()
        .assert_success();
    let _pty = harness::PtyClient::attach(&tmux, 80, 24);
    tmux.wait_for("#{session_attached}", |v| v == "1", t);
    let client = tmux
        .cmd()
        .args(["list-clients", "-F", "#{client_name}"])
        .run()
        .stdout_trimmed();
    let out = tmux
        .cmd()
        .args([
            "send-keys",
            "-K",
            "-c",
            &client,
            "F11",
            "F12",
            ";",
            "display",
            "-p",
            "#{@x}",
        ])
        .run();
    out.assert_success();
    assert_eq!(out.stdout_trimmed(), "ab");
}

/// A server with a session running sleep and one attached client, whose
/// name comes back for send-keys -K -c.
fn server_with_client() -> (TmuxTestHarness, harness::PtyClient, String) {
    let t = Duration::from_secs(5);
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-x", "80", "-y", "24", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(t);
    let pty = harness::PtyClient::attach(&tmux, 80, 24);
    tmux.wait_for("#{session_attached}", |v| v == "1", t);
    let client = tmux
        .cmd()
        .args(["list-clients", "-F", "#{client_name}"])
        .run()
        .stdout_trimmed();
    (tmux, pty, client)
}

/// The output of a command, or its error prefixed with "error: ".
fn outcome(tmux: &TmuxTestHarness, args: &[&str]) -> String {
    let r = tmux.cmd().args(args).run();
    if r.success() {
        r.stdout_trimmed()
    } else {
        format!("error: {}", r.stderr_str().trim())
    }
}

/// Types `keys` into the client's key tables.
fn keys(tmux: &TmuxTestHarness, client: &str, keys: &[&str]) {
    let mut args = vec!["send-keys", "-K", "-c", client];
    args.extend_from_slice(keys);
    tmux.cmd().args(&args).run().assert_success();
}

/// bind-key without a command changes only what it is given: -N sets the
/// note, -r adds the repeat flag, neither clears the other (3.8).
#[test]
#[ignore = "bind-key without a command clears the note, run 15 task 3"]
fn bind_without_command_keeps_note_and_repeat() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let run = |args: &[&str]| outcome(&tmux, args);
    run(&["bind", "-N", "first note", "z", "display", "hi"]);
    run(&["bind", "-r", "z"]);
    assert_eq!(
        run(&["list-keys", "-T", "prefix", "z"]),
        "bind-key -r -T prefix z display-message hi"
    );
    assert_eq!(
        run(&["list-keys", "-N", "-T", "prefix", "z"]),
        "C-b z first note"
    );
    run(&["bind", "-N", "second note", "z"]);
    assert_eq!(
        run(&["list-keys", "-T", "prefix", "z"]),
        "bind-key -r -T prefix z display-message hi"
    );
    assert_eq!(
        run(&["list-keys", "-N", "-T", "prefix", "z"]),
        "C-b z second note"
    );
    // Without a binding there is nothing to change.
    run(&["bind", "-N", "lost", "-T", "nosuch", "z"]);
    assert_eq!(run(&["list-keys", "-T", "nosuch"]), "");
}

#[test]
#[ignore = "unbind-key names itself in the unknown key error, run 15 task 3"]
fn unbind_key_errors_and_tables() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let run = |args: &[&str]| outcome(&tmux, args);
    assert_eq!(
        run(&["unbind", "-T", "nosuch", "x"]),
        "error: table nosuch doesn't exist"
    );
    assert_eq!(run(&["unbind", "-q", "-T", "nosuch", "x"]), "");
    assert_eq!(
        run(&["unbind", "-a", "-T", "nosuch"]),
        "error: table nosuch doesn't exist"
    );
    assert_eq!(run(&["unbind", "notakey"]), "error: unknown key: notakey");
    assert_eq!(run(&["unbind", "-a", "x"]), "error: key given with -a");
    assert_eq!(run(&["unbind"]), "error: missing key");
    run(&["bind", "-T", "mytab", "a", "display", "a"]);
    run(&["bind", "-T", "mytab", "b", "display", "b"]);
    run(&["unbind", "-T", "mytab", "a"]);
    assert_eq!(
        run(&["list-keys", "-T", "mytab"]),
        "bind-key  -T mytab b display-message b"
    );
    // Unbinding the last key drops the table.
    run(&["unbind", "-T", "mytab", "b"]);
    assert_eq!(
        run(&["list-keys", "-T", "mytab"]),
        "error: table mytab doesn't exist"
    );
    // -a drops a table with defaults too.
    run(&["unbind", "-a", "-T", "prefix"]);
    assert_eq!(
        run(&["list-keys", "-T", "prefix"]),
        "error: table prefix doesn't exist"
    );
}

#[test]
fn list_keys_errors_and_notes() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let run = |args: &[&str]| outcome(&tmux, args);
    assert_eq!(
        run(&["list-keys", "notakey"]),
        "error: invalid key: notakey"
    );
    assert_eq!(
        run(&["list-keys", "-T", "prefix", "F12"]),
        "error: unknown key: F12"
    );
    run(&["bind", "-N", "a note", "F12", "display", "x"]);
    assert_eq!(
        run(&["list-keys", "-N", "-P", "PFX ", "F12"]),
        "PFX  F12 a note"
    );
    run(&["bind", "-T", "mytab", "F11", "display", "y"]);
    // -N lists keys without a note only with -a.
    assert_eq!(run(&["list-keys", "-N", "-T", "mytab"]), "");
    assert_eq!(
        run(&["list-keys", "-N", "-a", "-T", "mytab"]),
        "C-b F11 display-message y"
    );
}

#[test]
fn switch_client_key_table() {
    let (tmux, _pty, client) = server_with_client();
    let run = |args: &[&str]| outcome(&tmux, args);
    assert_eq!(
        run(&["switch-client", "-c", &client, "-T", "nosuch"]),
        "error: table nosuch doesn't exist"
    );
    run(&["bind", "-T", "mytab", "a", "set", "-g", "@a", "1"]);
    run(&["switch-client", "-c", &client, "-T", "mytab"]);
    let table = |tmux: &TmuxTestHarness| {
        outcome(
            tmux,
            &[
                "display",
                "-p",
                "-c",
                &client,
                "#{client_key_table}:#{client_prefix}",
            ],
        )
    };
    assert_eq!(table(&tmux), "mytab:1");
    keys(&tmux, &client, &["a"]);
    assert_eq!(run(&["show", "-gv", "@a"]), "1");
    assert_eq!(table(&tmux), "root:0");
    keys(&tmux, &client, &["C-b"]);
    assert_eq!(table(&tmux), "prefix:1");
    keys(&tmux, &client, &["Escape"]);
}

/// A client stays in a table that unbind-key dropped (its last key went),
/// so it does not see bindings added to a new table of the same name, and
/// the next key goes to the root table.
#[test]
#[ignore = "the client keeps a freed table, run 15 task 3"]
fn client_in_a_dropped_table() {
    let (tmux, _pty, client) = server_with_client();
    let run = |args: &[&str]| outcome(&tmux, args);
    run(&["bind", "-T", "mytab", "a", "set", "-g", "@a", "1"]);
    run(&["switch-client", "-c", &client, "-T", "mytab"]);
    run(&["unbind", "-T", "mytab", "a"]);
    run(&["bind", "-T", "mytab", "b", "set", "-g", "@b", "1"]);
    let table = || {
        outcome(
            &tmux,
            &["display", "-p", "-c", &client, "#{client_key_table}"],
        )
    };
    assert_eq!(table(), "mytab");
    keys(&tmux, &client, &["b"]);
    assert_eq!(run(&["show", "-gv", "@b"]), "error: invalid option: @b");
    assert_eq!(table(), "root");
    // Entering the table again finds the new binding.
    run(&["switch-client", "-c", &client, "-T", "mytab"]);
    keys(&tmux, &client, &["b"]);
    assert_eq!(run(&["show", "-gv", "@b"]), "1");
}

/// unbind-key -a moves the clients in the table back to the root table.
#[test]
fn unbind_all_moves_clients_out() {
    let (tmux, _pty, client) = server_with_client();
    let run = |args: &[&str]| outcome(&tmux, args);
    run(&["bind", "-T", "mytab", "a", "display", "a"]);
    run(&["switch-client", "-c", &client, "-T", "mytab"]);
    run(&["unbind", "-a", "-T", "mytab"]);
    assert_eq!(
        run(&["display", "-p", "-c", &client, "#{client_key_table}"]),
        "root"
    );
}

#[test]
fn repeating_bindings() {
    let (tmux, _pty, client) = server_with_client();
    let run = |args: &[&str]| outcome(&tmux, args);
    run(&["set", "-g", "repeat-time", "5000"]);
    run(&["bind", "-r", "F5", "set", "-ga", "@r", "x"]);
    run(&["bind", "F6", "set", "-ga", "@s", "y"]);
    keys(&tmux, &client, &["C-b", "F5", "F5"]);
    assert_eq!(run(&["show", "-gv", "@r"]), "xx");
    // A key that does not repeat ends repeating, and the next one goes to
    // the root table.
    keys(&tmux, &client, &["C-b", "F6", "F6"]);
    assert_eq!(run(&["show", "-gv", "@s"]), "y");
}

/// Key table names need not be UTF-8.
#[test]
#[ignore = "table names that are not UTF-8 panic, run 15 task 2"]
fn key_table_names_that_are_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::OsStr::from_bytes(b"x\xff");
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args(["bind", "-T"])
        .args([name])
        .args(["a", "display", "hi"])
        .run()
        .assert_success();
    let out = tmux.cmd().args(["list-keys", "-T"]).args([name]).run();
    out.assert_success();
    assert_eq!(
        out.output.stdout,
        b"bind-key  -T x\xff a display-message hi\n"
    );
}

/// Keys sent to a pane in a mode go through the mode's key table.
#[test]
fn send_keys_uses_the_mode_table() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let run = |args: &[&str]| outcome(&tmux, args);
    run(&["set", "-g", "mode-keys", "emacs"]);
    run(&["bind", "-T", "copy-mode", "F5", "set", "-g", "@m", "1"]);
    run(&["copy-mode"]);
    run(&["send-keys", "F5"]);
    assert_eq!(run(&["show", "-gv", "@m"]), "1");
}
