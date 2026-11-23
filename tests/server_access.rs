//! server-access and the server ACL, checked against tmux 3.8.
//!
//! These tests only change the ACL of their own test server: they add
//! the nobody user and the user's own primary group, and nothing else.
//! The user running them always keeps access, because the server owner
//! cannot be changed and user entries win over group entries.

use std::process::Command;
use std::time::Duration;

use harness::{PtyClient, TmuxTestHarness};

use super::harness;

fn server() -> TmuxTestHarness {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux
}

/// Output of `id` with one flag, such as -un for the user name.
fn id(flag: &str) -> String {
    let out = Command::new("id").arg(flag).output().expect("run id");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn access(tmux: &TmuxTestHarness, args: &[&str]) -> Vec<String> {
    let mut all = vec!["server-access"];
    all.extend_from_slice(args);
    let result = tmux.cmd().args(all).run();
    result.assert_success();
    result.stdout_lines()
}

fn access_error(tmux: &TmuxTestHarness, args: &[&str]) -> String {
    let mut all = vec!["server-access"];
    all.extend_from_slice(args);
    let result = tmux.cmd().args(all).run();
    result.assert_failure();
    result.stderr_str().trim().to_string()
}

#[test]
fn list_shows_the_owner_with_its_type() {
    let tmux = server();
    assert_eq!(access(&tmux, &["-l"]), [format!("{} (U,W)", id("-un"))]);
}

#[test]
fn users_and_groups_are_listed_users_first() {
    let tmux = server();
    let user = id("-un");
    let group = id("-gn");
    access(&tmux, &["-a", "-g", &group]);
    access(&tmux, &["-a", "nobody"]);
    assert_eq!(
        access(&tmux, &["-l"]),
        [
            format!("{user} (U,W)"),
            "nobody (U,W)".to_string(),
            format!("{group} (G,W)"),
        ]
    );
    access(&tmux, &["-r", "-g", &group]);
    access(&tmux, &["-r", "nobody"]);
    assert_eq!(
        access(&tmux, &["-l"]),
        [
            format!("{user} (U,W)"),
            "nobody (U,R)".to_string(),
            format!("{group} (G,R)"),
        ]
    );
    access(&tmux, &["-d", "-g", &group]);
    access(&tmux, &["-d", "nobody"]);
    assert_eq!(access(&tmux, &["-l"]), [format!("{user} (U,W)")]);
}

#[test]
fn write_flag_adds_a_missing_group() {
    let tmux = server();
    let group = id("-gn");
    access(&tmux, &["-w", "-g", &group]);
    let list = access(&tmux, &["-l"]);
    assert_eq!(list.last(), Some(&format!("{group} (G,W)")));
}

#[test]
fn group_errors() {
    let tmux = server();
    let group = id("-gn");
    assert_eq!(
        access_error(&tmux, &["-d", "-g", &group]),
        format!("group {group} not found")
    );
    access(&tmux, &["-a", "-g", &group]);
    assert_eq!(
        access_error(&tmux, &["-a", "-g", &group]),
        format!("group {group} is already added")
    );
    assert_eq!(
        access_error(&tmux, &["-a", "-g", "no-such-group-xyz"]),
        "unknown group: no-such-group-xyz"
    );
    assert_eq!(
        access_error(&tmux, &["-a", "no-such-user-xyz"]),
        "unknown user: no-such-user-xyz"
    );
    assert_eq!(access_error(&tmux, &[]), "missing user or group argument");
}

#[test]
fn owner_cannot_be_changed_but_its_group_can() {
    let tmux = server();
    let user = id("-un");
    assert_eq!(
        access_error(&tmux, &["-r", &user]),
        format!("{user} owns the server, can't change access")
    );
    access(&tmux, &["-r", "-g", &id("-gn")]);
}

#[test]
fn read_only_group_does_not_change_the_owner() {
    let tmux = server();
    let _client = PtyClient::attach(&tmux, 80, 24);
    access(&tmux, &["-r", "-g", &id("-gn")]);
    let flags = tmux
        .cmd()
        .args(["list-clients", "-F", "#{client_readonly}"])
        .run();
    flags.assert_success();
    assert_eq!(flags.stdout_trimmed(), "0");
    // Removing the group leaves the client attached, the user entry
    // still allows it.
    access(&tmux, &["-d", "-g", &id("-gn")]);
    let clients = tmux
        .cmd()
        .args(["list-clients", "-F", "#{client_readonly}"])
        .run();
    assert_eq!(clients.stdout_trimmed(), "0");
}

#[test]
fn write_access_update_clears_read_only_of_matching_clients() {
    let tmux = server();
    let _client = PtyClient::attach(&tmux, 80, 24);
    tmux.cmd()
        .args(["switch-client", "-r"])
        .run()
        .assert_success();
    let readonly = || {
        tmux.cmd()
            .args(["list-clients", "-F", "#{client_readonly}"])
            .run()
            .stdout_trimmed()
    };
    assert_eq!(readonly(), "1");
    // Any change reapplies the ACL to every client. The owner's entry is
    // read and write, so the client loses read only.
    access(&tmux, &["-a", "nobody"]);
    access(&tmux, &["-r", "nobody"]);
    assert_eq!(readonly(), "0");
}
