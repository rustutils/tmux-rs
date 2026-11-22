use super::harness;

use std::time::Duration;

use harness::{PtyClient, TmuxTestHarness};

#[test]
fn list_clients_empty_for_detached_session() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // No clients should be attached to a detached session
    let result = tmux.cmd().args(["list-clients"]).run();
    result.assert_success();
    assert_eq!(
        result.stdout_trimmed(),
        "",
        "detached session should have no clients"
    );
}

#[test]
fn list_clients_with_format() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // list-clients with a format string should succeed even with no clients
    let result = tmux
        .cmd()
        .args(["list-clients", "-F", "#{client_name}"])
        .run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "");
}

#[test]
fn detach_client_no_client_errors() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // detach-client with no attached clients should fail
    let result = tmux.cmd().args(["detach-client"]).run();
    assert!(
        !result.success(),
        "detach-client should fail when no clients are attached"
    );
}

#[test]
fn detach_client_with_nonexistent_target() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Detaching a nonexistent client should fail
    let result = tmux
        .cmd()
        .args(["detach-client", "-t", "/dev/nonexistent"])
        .run();
    assert!(
        !result.success(),
        "detach-client with bad target should fail"
    );
}

#[test]
fn switch_client_no_client_errors() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().args(["-s", "sess1"]).run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Create a second session
    tmux.cmd()
        .args(["new-session", "-d", "-s", "sess2"])
        .run()
        .assert_success();

    // switch-client with no attached clients should fail
    let result = tmux.cmd().args(["switch-client", "-t", "sess2"]).run();
    assert!(
        !result.success(),
        "switch-client should fail with no attached client"
    );
}

#[test]
fn switch_client_with_nonexistent_session() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Switching to a nonexistent session should fail
    let result = tmux
        .cmd()
        .args(["switch-client", "-t", "nonexistent"])
        .run();
    assert!(
        !result.success(),
        "switch-client to nonexistent session should fail"
    );
}

#[test]
fn refresh_client_no_client() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // refresh-client with no attached clients should fail or be a no-op
    let result = tmux.cmd().args(["refresh-client"]).run();
    // With no clients attached, this will likely fail
    // Just verify it doesn't crash -- either success or clean failure is fine
    let _ = result.success();
}

#[test]
fn refresh_client_with_flag() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // refresh-client -S (status) with no clients
    let result = tmux.cmd().args(["refresh-client", "-S"]).run();
    let _ = result.success();
}

#[test]
fn show_messages_succeeds() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Attach a PTY client so show-messages has a client context
    let _client = PtyClient::attach(&tmux, 80, 24);

    // show-messages should succeed (may have empty output or server messages)
    let result = tmux.cmd().args(["show-messages"]).run();
    result.assert_success();
}

#[test]
fn show_messages_after_commands() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Attach a PTY client so show-messages has a client context
    let _client = PtyClient::attach(&tmux, 80, 24);

    // Run a few commands to generate activity
    tmux.cmd()
        .args(["display-message", "test message one"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["display-message", "test message two"])
        .run()
        .assert_success();

    // show-messages should succeed
    let result = tmux.cmd().args(["show-messages"]).run();
    result.assert_success();
}

#[test]
fn lock_server_smoke_test() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // lock-server should not crash.
    // With no clients attached, it may fail or succeed depending on implementation.
    let result = tmux.cmd().args(["lock-server"]).run();
    // Just verify the process completed without crashing
    let _ = result.success();
}

#[test]
fn lock_session_smoke_test() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // lock-session should not crash
    let result = tmux.cmd().args(["lock-session"]).run();
    let _ = result.success();

    // Server should still be responsive after lock
    tmux.cmd().args(["has-session"]).run().assert_success();
}

#[test]
fn server_access_show() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // server-access without arguments or with invalid args should produce an error
    // (it requires subcommands like allow/deny)
    let result = tmux.cmd().args(["server-access"]).run();
    // Just verify it doesn't crash - it may error due to missing arguments
    let _ = result.success();
}

#[test]
fn server_access_allow_deny() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Try to allow a user -- this may fail if the feature isn't supported
    // or if the user doesn't exist, but it should not crash
    let result = tmux
        .cmd()
        .args(["server-access", "-a", "nobody"])
        .run();
    let _ = result.success();

    // Try to deny a user
    let result = tmux
        .cmd()
        .args(["server-access", "-d", "nobody"])
        .run();
    let _ = result.success();
}

#[test]
fn multiple_sessions_list_clients() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().args(["-s", "alpha"]).run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Create additional sessions
    tmux.cmd()
        .args(["new-session", "-d", "-s", "beta"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["new-session", "-d", "-s", "gamma"])
        .run()
        .assert_success();

    // All sessions should exist
    tmux.cmd()
        .args(["has-session", "-t", "alpha"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["has-session", "-t", "beta"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["has-session", "-t", "gamma"])
        .run()
        .assert_success();

    // Still no clients attached
    let result = tmux.cmd().args(["list-clients"]).run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "");
}

#[test]
fn list_clients_format_flag() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // list-clients -F with various format strings should work
    let result = tmux
        .cmd()
        .args(["list-clients", "-F", "#{client_name} #{client_session}"])
        .run();
    result.assert_success();
    // Output should be empty since no clients are attached
    assert_eq!(result.stdout_trimmed(), "");
}

#[test]
fn detach_client_all_flag() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().args(["-s", "mysess"]).run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // detach-client -a (detach all other clients) with no clients
    // should fail since there's no current client
    let result = tmux
        .cmd()
        .args(["detach-client", "-a", "-s", "mysess"])
        .run();
    assert!(
        !result.success(),
        "detach-client -a should fail with no attached clients"
    );
}

#[test]
fn switch_client_next_previous_no_client() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().args(["-s", "s1"]).run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["new-session", "-d", "-s", "s2"])
        .run()
        .assert_success();

    // switch-client -n (next) and -p (previous) with no client should fail
    let result = tmux.cmd().args(["switch-client", "-n"]).run();
    assert!(
        !result.success(),
        "switch-client -n should fail with no attached client"
    );

    let result = tmux.cmd().args(["switch-client", "-p"]).run();
    assert!(
        !result.success(),
        "switch-client -p should fail with no attached client"
    );
}

/// Build one imsg frame: a 16 byte header (type, length, peer id carrying
/// the protocol version, pid) followed by the payload. No fd is passed, so
/// the fd bit in the length stays clear.
fn imsg_frame(msg_type: u32, payload: &[u8]) -> Vec<u8> {
    const PROTOCOL_VERSION: u32 = 8;
    let len = (16 + payload.len()) as u32;
    let mut frame = Vec::with_capacity(len as usize);
    frame.extend_from_slice(&msg_type.to_ne_bytes());
    frame.extend_from_slice(&len.to_ne_bytes());
    frame.extend_from_slice(&PROTOCOL_VERSION.to_ne_bytes());
    frame.extend_from_slice(&std::process::id().to_ne_bytes());
    frame.extend_from_slice(payload);
    frame
}

/// A tmux 3.8 client sends identify flags the 3.5a protocol did not have
/// (WRITE_ACK and others) and the new MSG_WRITE_DONE message. The server
/// must keep running, and it must also ignore message types it does not
/// know, as C tmux does.
#[test]
fn server_survives_newer_client_messages() {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream;

    const MSG_IDENTIFY_FLAGS: u32 = 100;
    const MSG_IDENTIFY_DONE: u32 = 106;
    const MSG_IDENTIFY_LONGFLAGS: u32 = 111;
    const MSG_WRITE_DONE: u32 = 308;
    const UNKNOWN_TYPE: u32 = 4242;
    const CLIENT_WRITE_ACK: u64 = 0x4000000000;
    const CLIENT_NO_DETACH_ON_DESTROY: u64 = 0x8000000000;
    const UNKNOWN_FLAG: u64 = 1 << 60;

    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let mut sock = UnixStream::connect(tmux.socket_path()).expect("connect to server");
    let longflags = CLIENT_WRITE_ACK | CLIENT_NO_DETACH_ON_DESTROY | UNKNOWN_FLAG;
    let mut bytes = Vec::new();
    bytes.extend(imsg_frame(MSG_IDENTIFY_LONGFLAGS, &longflags.to_ne_bytes()));
    bytes.extend(imsg_frame(MSG_IDENTIFY_FLAGS, &i32::MIN.to_ne_bytes()));
    bytes.extend(imsg_frame(UNKNOWN_TYPE, &[]));
    bytes.extend(imsg_frame(MSG_IDENTIFY_DONE, &[]));
    // msg_write_done for a stream that does not exist: stream 99, error 0.
    let mut write_done = Vec::new();
    write_done.extend_from_slice(&99i32.to_ne_bytes());
    write_done.extend_from_slice(&0i32.to_ne_bytes());
    bytes.extend(imsg_frame(MSG_WRITE_DONE, &write_done));
    bytes.extend(imsg_frame(UNKNOWN_TYPE, &[1, 2, 3]));
    sock.write_all(&bytes).expect("send frames");

    // Give the server time to process the frames before checking on it.
    std::thread::sleep(Duration::from_millis(200));
    drop(sock);

    tmux.cmd().args(["has-session"]).run().assert_success();
}
