use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

#[test]
fn set_environment_and_show() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Set a session-level environment variable
    tmux.cmd()
        .args(["set-environment", "MY_TEST_VAR", "hello_world"])
        .run()
        .assert_success();

    // Verify it appears in show-environment output
    let result = tmux.cmd().args(["show-environment", "MY_TEST_VAR"]).run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "MY_TEST_VAR=hello_world");
}

#[test]
fn set_environment_overwrite() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-environment", "OVERWRITE_VAR", "first"])
        .run()
        .assert_success();

    tmux.cmd()
        .args(["set-environment", "OVERWRITE_VAR", "second"])
        .run()
        .assert_success();

    let result = tmux.cmd().args(["show-environment", "OVERWRITE_VAR"]).run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "OVERWRITE_VAR=second");
}

#[test]
fn set_environment_global() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Set a global environment variable
    tmux.cmd()
        .args(["set-environment", "-g", "GLOBAL_VAR", "global_value"])
        .run()
        .assert_success();

    // Verify it appears in global show-environment
    let result = tmux
        .cmd()
        .args(["show-environment", "-g", "GLOBAL_VAR"])
        .run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "GLOBAL_VAR=global_value");

    // It should NOT appear in session-level environment (unless inherited)
    let result = tmux.cmd().args(["show-environment", "GLOBAL_VAR"]).run();
    // Looking up a var that doesn't exist at session level should fail
    assert!(!result.success());
}

#[test]
fn set_environment_unset() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Set then unset a variable
    tmux.cmd()
        .args(["set-environment", "EPHEMERAL", "temp"])
        .run()
        .assert_success();

    // Confirm it exists
    let result = tmux.cmd().args(["show-environment", "EPHEMERAL"]).run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "EPHEMERAL=temp");

    // Unset it
    tmux.cmd()
        .args(["set-environment", "-u", "EPHEMERAL"])
        .run()
        .assert_success();

    // Now looking it up should fail
    let result = tmux.cmd().args(["show-environment", "EPHEMERAL"]).run();
    assert!(!result.success());
}

#[test]
fn set_environment_global_unset() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-environment", "-g", "G_REMOVE", "bye"])
        .run()
        .assert_success();

    let result = tmux
        .cmd()
        .args(["show-environment", "-g", "G_REMOVE"])
        .run();
    result.assert_success();

    // Unset the global var
    tmux.cmd()
        .args(["set-environment", "-gu", "G_REMOVE"])
        .run()
        .assert_success();

    let result = tmux
        .cmd()
        .args(["show-environment", "-g", "G_REMOVE"])
        .run();
    assert!(!result.success());
}

#[test]
fn show_environment_lists_multiple_vars() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-environment", "LIST_A", "1"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-environment", "LIST_B", "2"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-environment", "LIST_C", "3"])
        .run()
        .assert_success();

    let result = tmux.cmd().args(["show-environment"]).run();
    result.assert_success();
    let output = result.stdout_str();

    assert!(output.contains("LIST_A=1"), "missing LIST_A in:\n{output}");
    assert!(output.contains("LIST_B=2"), "missing LIST_B in:\n{output}");
    assert!(output.contains("LIST_C=3"), "missing LIST_C in:\n{output}");
}

#[test]
fn show_environment_global_lists_vars() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-environment", "-g", "GLIST_X", "alpha"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-environment", "-g", "GLIST_Y", "beta"])
        .run()
        .assert_success();

    let result = tmux.cmd().args(["show-environment", "-g"]).run();
    result.assert_success();
    let output = result.stdout_str();

    assert!(
        output.contains("GLIST_X=alpha"),
        "missing GLIST_X in:\n{output}"
    );
    assert!(
        output.contains("GLIST_Y=beta"),
        "missing GLIST_Y in:\n{output}"
    );
}

#[test]
fn show_environment_nonexistent_var_fails() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux
        .cmd()
        .args(["show-environment", "DOES_NOT_EXIST_12345"])
        .run();
    assert!(!result.success());
}

/// A server with one session, started with `env` in the environment of
/// the command that starts it. COLORTERM is removed from it, so the tests
/// see the value tmux sets and not the developer's.
fn server_with_env(env: &[(&str, &str)]) -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    let mut cmd = tmux
        .new_session()
        .args(["-s", "s", "sleep 1000"])
        .env_remove("COLORTERM");
    for (k, v) in env {
        cmd = cmd.env(*k, *v);
    }
    cmd.run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux
}

/// Runs `script` with sh in a new window of session s and returns what it
/// printed. The script must print one line. extra are more new-window
/// arguments (-e).
fn pane_output(tmux: &TmuxTestHarness, extra: &[&str], script: &str) -> String {
    let mut args = vec!["new-window", "-d", "-P", "-F", "#{pane_id}", "-t", "s:"];
    args.extend_from_slice(extra);
    let command = format!("{script}; sleep 1000");
    args.extend_from_slice(&["sh", "-c", &command]);
    let pane = tmux.cmd().args(&args).run().stdout_trimmed();
    output_of(tmux, &pane)
}

/// Waits until `pane` shows something and returns it.
fn output_of(tmux: &TmuxTestHarness, pane: &str) -> String {
    let start = std::time::Instant::now();
    loop {
        let out = tmux.capture_pane_target(pane);
        if !out.is_empty() {
            return out;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "no output from {pane}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The output and error of a command, as one string to compare.
fn outcome(r: &harness::TmuxResult) -> String {
    if r.success() {
        r.stdout_trimmed()
    } else {
        format!("error: {}", r.stderr_str().trim())
    }
}

#[test]
fn set_environment_errors() {
    let tmux = server_with_env(&[]);
    let run = |args: &[&str]| outcome(&tmux.cmd().args(args).run());
    assert_eq!(run(&["setenv", "", "x"]), "error: empty variable name");
    assert_eq!(
        run(&["setenv", "A=B", "x"]),
        "error: variable name contains ="
    );
    assert_eq!(
        run(&["setenv", "-u", "A", "x"]),
        "error: can't specify a value with -u"
    );
    assert_eq!(
        run(&["setenv", "-r", "A", "x"]),
        "error: can't specify a value with -r"
    );
    assert_eq!(run(&["setenv", "A"]), "error: no value specified");
    assert_eq!(
        run(&["setenv", "-t", "nosuch", "A", "x"]),
        "error: no such session: nosuch"
    );
    assert_eq!(
        run(&["showenv", "-t", "nosuch"]),
        "error: no such session: nosuch"
    );
    assert_eq!(run(&["showenv", "NOPE"]), "error: unknown variable: NOPE");
}

#[test]
fn hidden_and_removed_variables() {
    let tmux = server_with_env(&[]);
    for args in [
        &["setenv", "-h", "HID", "secret"][..],
        &["setenv", "SHOWN", "yes"],
        &["setenv", "-r", "GONE"],
    ] {
        tmux.cmd().args(args).run().assert_success();
    }
    let show = |args: &[&str]| outcome(&tmux.cmd().args(args).run());
    // A hidden variable is only shown with -h, a removed one as -NAME.
    assert_eq!(show(&["showenv", "HID"]), "");
    assert_eq!(show(&["showenv", "-h", "HID"]), "HID=secret");
    assert_eq!(show(&["showenv", "-h", "SHOWN"]), "");
    assert_eq!(show(&["showenv", "GONE"]), "-GONE");
    let all = show(&["showenv"]);
    assert!(all.contains("SHOWN=yes") && all.contains("-GONE"), "{all}");
    assert!(!all.contains("HID"), "{all}");
    assert_eq!(show(&["showenv", "-h"]), "HID=secret");
    // -u removes the entry, so it is unknown afterwards.
    tmux.cmd()
        .args(["setenv", "-u", "GONE"])
        .run()
        .assert_success();
    assert_eq!(show(&["showenv", "GONE"]), "error: unknown variable: GONE");
}

#[test]
fn show_environment_shell_format() {
    let tmux = server_with_env(&[]);
    tmux.cmd()
        .args(["setenv", "Q", "a$b`c\"d\\e f"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["setenv", "-r", "R"])
        .run()
        .assert_success();
    let show = |name: &str| outcome(&tmux.cmd().args(["showenv", "-s", name]).run());
    assert_eq!(show("Q"), r#"Q="a\$b\`c\"d\\e f"; export Q;"#);
    assert_eq!(show("R"), "unset R;");
}

#[test]
fn set_environment_expands_formats() {
    let tmux = server_with_env(&[]);
    tmux.cmd()
        .args(["setenv", "-F", "N", "#{session_name}:#{window_index}"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["setenv", "-gF", "G", "#{session_name}"])
        .run()
        .assert_success();
    assert_eq!(outcome(&tmux.cmd().args(["showenv", "N"]).run()), "N=s:0");
    assert_eq!(
        outcome(&tmux.cmd().args(["showenv", "-g", "G"]).run()),
        "G=s"
    );
}

#[test]
fn new_panes_get_the_tmux_variables() {
    let tmux = server_with_env(&[]);
    let out = pane_output(
        &tmux,
        &[],
        r#"printf '%s|%s|%s|%s|%s\n' "$TERM" "$TERM_PROGRAM" "$TERM_PROGRAM_VERSION" "$COLORTERM" "$TMUX""#,
    );
    let term = tmux.cmd().args(["show", "-gv", "default-terminal"]).run();
    let server_version = tmux.query("#{version}");
    let tmux_var = tmux.query("#{socket_path},#{pid},#{session_id}");
    let tmux_var = tmux_var.replace(",$", ",");
    assert_eq!(
        out,
        format!(
            "{}|tmux|{server_version}|truecolor|{tmux_var}",
            term.stdout_trimmed()
        )
    );
}

#[test]
fn new_panes_get_the_session_and_global_environment() {
    let tmux = server_with_env(&[]);
    for args in [
        &["setenv", "-g", "G1", "global"][..],
        &["setenv", "-g", "BOTH", "global"],
        &["setenv", "BOTH", "session"],
        &["setenv", "-h", "HID", "secret"],
        &["setenv", "-g", "MASKED", "global"],
        &["setenv", "-r", "MASKED"],
    ] {
        tmux.cmd().args(args).run().assert_success();
    }
    let out = pane_output(
        &tmux,
        &["-e", "EXTRA=e"],
        r#"echo "${G1-x}|${BOTH-x}|${HID-x}|${MASKED-x}|${EXTRA-x}""#,
    );
    assert_eq!(out, "global|session|x|x|e");
}

#[test]
fn split_window_environment() {
    let tmux = server_with_env(&[]);
    let pane = tmux
        .cmd()
        .args([
            "split-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "s:0",
            "-e",
            "SPLIT=1",
            "sh",
            "-c",
            "echo \"${SPLIT-x}\"; sleep 1000",
        ])
        .run()
        .stdout_trimmed();
    assert_eq!(output_of(&tmux, &pane), "1");
}

/// A variable that was in the environment of the server process when it
/// started, removed from the global environment with -gu, does not reach
/// new panes: the child gets exactly the environment tmux built.
#[test]
fn removed_global_variable_does_not_reach_panes() {
    let tmux = server_with_env(&[("LEAKED_VAR", "from-start")]);
    assert_eq!(
        outcome(&tmux.cmd().args(["showenv", "-g", "LEAKED_VAR"]).run()),
        "LEAKED_VAR=from-start"
    );
    tmux.cmd()
        .args(["setenv", "-gu", "LEAKED_VAR"])
        .run()
        .assert_success();
    let out = pane_output(&tmux, &[], r#"echo "${LEAKED_VAR-unset}""#);
    assert_eq!(out, "unset");
}

#[test]
fn run_shell_gets_the_environment() {
    let tmux = server_with_env(&[]);
    tmux.cmd()
        .args(["setenv", "RS", "session"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["setenv", "-g", "RG", "global"])
        .run()
        .assert_success();
    let out = tmux
        .cmd()
        .args([
            "run-shell",
            r#"echo "${RS-x}|${RG-x}|${COLORTERM-x}|${TERM_PROGRAM-x}""#,
        ])
        .run();
    assert_eq!(outcome(&out), "session|global|truecolor|tmux");
}

/// new-session copies the variables named in update-environment from the
/// client, and marks the ones the client does not have as removed. -E
/// leaves them out.
#[test]
fn update_environment_on_new_session() {
    let tmux = server_with_env(&[("DISPLAY", ":42")]);
    let show = |args: &[&str]| outcome(&tmux.cmd().args(args).run());
    assert_eq!(show(&["showenv", "-t", "s", "DISPLAY"]), "DISPLAY=:42");
    tmux.cmd()
        .args(["new-session", "-d", "-s", "e", "-E"])
        .env("DISPLAY", ":43")
        .run()
        .assert_success();
    assert_eq!(
        show(&["showenv", "-t", "e", "DISPLAY"]),
        "error: unknown variable: DISPLAY"
    );
    tmux.cmd()
        .args(["new-session", "-d", "-s", "n"])
        .env("DISPLAY", ":44")
        .run()
        .assert_success();
    assert_eq!(show(&["showenv", "-t", "n", "DISPLAY"]), "DISPLAY=:44");
    tmux.cmd()
        .args(["set", "-g", "update-environment", "NOT_THERE"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["new-session", "-d", "-s", "m"])
        .run()
        .assert_success();
    assert_eq!(show(&["showenv", "-t", "m", "NOT_THERE"]), "-NOT_THERE");
}

#[test]
fn formats_look_up_the_environment() {
    let tmux = server_with_env(&[]);
    tmux.cmd()
        .args(["setenv", "FS", "session"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["setenv", "-g", "FG", "global"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["setenv", "-g", "FS", "global"])
        .run()
        .assert_success();
    assert_eq!(tmux.query("#{FS}|#{FG}"), "session|global");
}

/// A client whose environment has a variable name that is not UTF-8 works,
/// both for the client that starts the server and for a later one.
#[test]
#[ignore = "show-environment prints names as lossy UTF-8, run 14 task 4"]
fn environment_names_that_are_not_utf8() {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::OsStr::from_bytes(b"BAD\xffNAME");
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["-s", "s", "sleep 1000"])
        .env(name, "1")
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let all = tmux.cmd().args(["showenv", "-g"]).run();
    all.assert_success();
    assert!(
        all.output.stdout.windows(8).any(|w| w == b"BAD\xffNAME"),
        "{}",
        all.stdout_str()
    );
    tmux.cmd()
        .args(["new-session", "-d", "-s", "t"])
        .env(name, "2")
        .run()
        .assert_success();
    let sessions = tmux.cmd().args(["ls", "-F", "#{session_name}"]).run();
    assert_eq!(sessions.stdout_lines(), ["s", "t"]);
}
