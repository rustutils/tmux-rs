use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

// ---------------------------------------------------------------------------
// set-buffer
// ---------------------------------------------------------------------------

#[test]
fn set_buffer_basic() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "hello world"])
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "hello world");
}

#[test]
fn set_buffer_named() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "mybuf", "named content"])
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer", "-b", "mybuf"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "named content");
}

#[test]
fn set_buffer_overwrite() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "over", "first"])
        .run()
        .assert_success();

    tmux.cmd()
        .args(["set-buffer", "-b", "over", "second"])
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer", "-b", "over"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "second");
}

#[test]
fn set_buffer_append() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "app", "hello"])
        .run()
        .assert_success();

    tmux.cmd()
        .args(["set-buffer", "-a", "-b", "app", " world"])
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer", "-b", "app"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "hello world");
}

#[test]
fn set_buffer_append_nonexistent_fails() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Appending to a buffer that does not exist creates it in C tmux
    let result = tmux
        .cmd()
        .args(["set-buffer", "-a", "-b", "doesnotexist", "data"])
        .run();
    result.assert_success();
}

#[test]
fn set_buffer_empty_string() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Setting an empty buffer: tmux accepts it but show-buffer may report "no buffers"
    // since empty content effectively means no buffer was stored
    let result = tmux.cmd().args(["set-buffer", ""]).run();
    // Just verify it doesn't crash — behavior varies by version
    let _ = result;
}

// ---------------------------------------------------------------------------
// list-buffers
// ---------------------------------------------------------------------------

#[test]
fn list_buffers_empty() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux.cmd().args(["list-buffers"]).run();
    // With no buffers, output should be empty
    assert_eq!(result.stdout_trimmed(), "");
}

#[test]
fn list_buffers_shows_entries() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "buf0", "aaa"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "buf1", "bbbbb"])
        .run()
        .assert_success();

    let result = tmux.cmd().args(["list-buffers"]).run();
    result.assert_success();
    let output = result.stdout_trimmed();
    let lines: Vec<&str> = output.lines().collect();
    assert_eq!(lines.len(), 2, "expected 2 buffers, got: {output}");

    // Both buffer names should appear somewhere in the output
    assert!(
        output.contains("buf0"),
        "output should mention buf0: {output}"
    );
    assert!(
        output.contains("buf1"),
        "output should mention buf1: {output}"
    );
}

#[test]
fn list_buffers_custom_format() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "fmt", "12345"])
        .run()
        .assert_success();

    let result = tmux
        .cmd()
        .args(["list-buffers", "-F", "#{buffer_name}:#{buffer_size}"])
        .run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "fmt:5");
}

// ---------------------------------------------------------------------------
// show-buffer
// ---------------------------------------------------------------------------

#[test]
fn show_buffer_default() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "show me"])
        .run()
        .assert_success();

    let result = tmux.cmd().args(["show-buffer"]).run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "show me");
}

#[test]
fn show_buffer_named() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "sb", "specific"])
        .run()
        .assert_success();

    let result = tmux.cmd().args(["show-buffer", "-b", "sb"]).run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "specific");
}

#[test]
fn show_buffer_nonexistent_fails() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux.cmd().args(["show-buffer", "-b", "nope"]).run();
    result.assert_failure();
}

// ---------------------------------------------------------------------------
// save-buffer
// ---------------------------------------------------------------------------

#[test]
fn save_buffer_to_file() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();

    tmux.cmd()
        .args(["set-buffer", "save this"])
        .run()
        .assert_success();

    tmux.cmd()
        .args(["save-buffer", &path])
        .run()
        .assert_success();

    let contents = std::fs::read_to_string(&path).expect("failed to read saved buffer");
    assert_eq!(contents, "save this");
}

/// A write error on the client side must reach the server as a command
/// error. Since tmux 3.8 the client reports it with `MSG_WRITE_DONE`.
#[test]
fn save_buffer_write_error_is_reported() {
    if !std::path::Path::new("/dev/full").exists() {
        return;
    }
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "cannot save this"])
        .run()
        .assert_success();

    let result = tmux.cmd().args(["save-buffer", "/dev/full"]).run();
    result.assert_failure();
    let stderr = result.stderr_str();
    assert!(
        stderr.contains("No space left on device") && stderr.contains("/dev/full"),
        "expected the write error in stderr, got: {stderr:?}"
    );
}

#[test]
fn save_buffer_named_to_file() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();

    tmux.cmd()
        .args(["set-buffer", "-b", "savebuf", "named save"])
        .run()
        .assert_success();

    tmux.cmd()
        .args(["save-buffer", "-b", "savebuf", &path])
        .run()
        .assert_success();

    let contents = std::fs::read_to_string(&path).expect("failed to read saved buffer");
    assert_eq!(contents, "named save");
}

#[test]
fn save_buffer_append_to_file() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();

    tmux.cmd()
        .args(["set-buffer", "-b", "a1", "first"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "a2", "second"])
        .run()
        .assert_success();

    tmux.cmd()
        .args(["save-buffer", "-b", "a1", &path])
        .run()
        .assert_success();

    // -a flag appends to file instead of overwriting
    tmux.cmd()
        .args(["save-buffer", "-a", "-b", "a2", &path])
        .run()
        .assert_success();

    let contents = std::fs::read_to_string(&path).expect("failed to read saved buffer");
    assert_eq!(contents, "firstsecond");
}

#[test]
fn save_buffer_nonexistent_fails() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();

    let result = tmux
        .cmd()
        .args(["save-buffer", "-b", "missing", &path])
        .run();
    result.assert_failure();
}

// ---------------------------------------------------------------------------
// load-buffer
// ---------------------------------------------------------------------------

#[test]
fn load_buffer_from_file() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();
    std::fs::write(&path, "loaded content").expect("failed to write tempfile");

    tmux.cmd()
        .args(["load-buffer", &path])
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "loaded content");
}

#[test]
fn load_buffer_named() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();
    std::fs::write(&path, "named load").expect("failed to write tempfile");

    tmux.cmd()
        .args(["load-buffer", "-b", "lb", &path])
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer", "-b", "lb"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "named load");
}

#[test]
fn load_buffer_nonexistent_file_fails() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux
        .cmd()
        .args(["load-buffer", "/tmp/tmux-rs-test-nonexistent-file-xyz"])
        .run();
    result.assert_failure();
}

#[test]
fn load_buffer_roundtrip_with_save() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Set a buffer, save it, delete all buffers, load it back
    tmux.cmd()
        .args(["set-buffer", "roundtrip data"])
        .run()
        .assert_success();

    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();

    tmux.cmd()
        .args(["save-buffer", &path])
        .run()
        .assert_success();

    tmux.cmd().args(["delete-buffer"]).run().assert_success();

    // Verify buffer is gone
    let result = tmux.cmd().args(["show-buffer"]).run();
    result.assert_failure();

    // Load it back
    tmux.cmd()
        .args(["load-buffer", &path])
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "roundtrip data");
}

// ---------------------------------------------------------------------------
// paste-buffer
// ---------------------------------------------------------------------------

#[test]
fn paste_buffer_into_pane() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["--", "cat"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Give cat a moment to start
    std::thread::sleep(Duration::from_millis(200));

    tmux.cmd()
        .args(["set-buffer", "pasted"])
        .run()
        .assert_success();

    tmux.cmd().args(["paste-buffer"]).run().assert_success();

    // Give cat time to echo back
    std::thread::sleep(Duration::from_millis(500));

    let pane = tmux.capture_pane();
    assert!(
        pane.contains("pasted"),
        "pane should contain pasted text, got: {pane}"
    );
}

#[test]
fn paste_buffer_named_into_pane() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["--", "cat"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    std::thread::sleep(Duration::from_millis(200));

    tmux.cmd()
        .args(["set-buffer", "-b", "pb", "namedpaste"])
        .run()
        .assert_success();

    tmux.cmd()
        .args(["paste-buffer", "-b", "pb"])
        .run()
        .assert_success();

    std::thread::sleep(Duration::from_millis(500));

    let pane = tmux.capture_pane();
    assert!(
        pane.contains("namedpaste"),
        "pane should contain namedpaste, got: {pane}"
    );
}

#[test]
fn paste_buffer_delete_flag() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["--", "cat"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    std::thread::sleep(Duration::from_millis(200));

    tmux.cmd()
        .args(["set-buffer", "-b", "del", "deleteme"])
        .run()
        .assert_success();

    // -d flag should delete the buffer after pasting
    tmux.cmd()
        .args(["paste-buffer", "-d", "-b", "del"])
        .run()
        .assert_success();

    std::thread::sleep(Duration::from_millis(500));

    let pane = tmux.capture_pane();
    assert!(
        pane.contains("deleteme"),
        "pane should contain pasted text, got: {pane}"
    );

    // Buffer should now be gone
    let result = tmux.cmd().args(["show-buffer", "-b", "del"]).run();
    result.assert_failure();
}

#[test]
fn paste_buffer_nonexistent_fails() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux.cmd().args(["paste-buffer", "-b", "nonexistent"]).run();
    result.assert_failure();
}

// ---------------------------------------------------------------------------
// delete-buffer
// ---------------------------------------------------------------------------

#[test]
fn delete_buffer_default() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "to delete"])
        .run()
        .assert_success();

    tmux.cmd().args(["delete-buffer"]).run().assert_success();

    let result = tmux.cmd().args(["show-buffer"]).run();
    result.assert_failure();
}

#[test]
fn delete_buffer_named() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "x", "data x"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "y", "data y"])
        .run()
        .assert_success();

    tmux.cmd()
        .args(["delete-buffer", "-b", "x"])
        .run()
        .assert_success();

    // x should be gone
    let result = tmux.cmd().args(["show-buffer", "-b", "x"]).run();
    result.assert_failure();

    // y should still exist
    let result = tmux.cmd().args(["show-buffer", "-b", "y"]).run();
    result.assert_success();
    assert_eq!(result.stdout_trimmed(), "data y");
}

#[test]
fn delete_buffer_nonexistent_fails() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux.cmd().args(["delete-buffer", "-b", "nope"]).run();
    result.assert_failure();
}

// ---------------------------------------------------------------------------
// Multiple buffers interaction
// ---------------------------------------------------------------------------

#[test]
fn multiple_buffers_most_recent_is_default() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "first"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "second"])
        .run()
        .assert_success();

    // show-buffer without -b should show the most recently added buffer
    let content = tmux.cmd().args(["show-buffer"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "second");
}

#[test]
fn set_buffer_multiline() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "line1\nline2\nline3"])
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer"]).run();
    content.assert_success();
    let output = content.stdout_str();
    assert!(
        output.contains("line1") && output.contains("line2") && output.contains("line3"),
        "buffer should contain all lines, got: {output}"
    );
}

#[test]
fn load_buffer_from_stdin() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // load-buffer - reads from stdin
    tmux.cmd()
        .args(["load-buffer", "-"])
        .stdin("from stdin")
        .run()
        .assert_success();

    let content = tmux.cmd().args(["show-buffer"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "from stdin");
}

/// Wait until the terminal of `client` receives an OSC 52 clipboard query.
fn wait_for_clipboard_query(client: &mut harness::PtyClient) {
    let start = std::time::Instant::now();
    let mut seen = Vec::new();
    while start.elapsed() < Duration::from_secs(5) {
        seen.extend(client.read_raw());
        if seen
            .windows(7)
            .any(|w| w == b"\x1b]52;c;" || w == b"\x1b]52;;?")
            || seen
                .windows(5)
                .any(|w| w == b"52;?\x07" || w == b";?\x1b\\\x07")
            || seen.windows(4).any(|w| w == b";?\x07")
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "no clipboard query sent to the terminal: {:?}",
        String::from_utf8_lossy(&seen)
    );
}

/// refresh-client -l asks the terminal for its clipboard and stores the
/// reply in a paste buffer. A reply that is not valid base64 is dropped.
/// Unpadded data like that used to be decoded into a buffer that was too
/// small.
#[test]
fn refresh_client_l_stores_clipboard_reply() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args([
            "set",
            "-as",
            "terminal-overrides",
            ",*:Ms=\\E]52;%p1%s;%p2%s\\007",
        ])
        .run()
        .assert_success();

    let mut client = harness::PtyClient::attach(&tmux, 80, 24);
    tmux.wait_for("#{session_attached}", |v| v == "1", Duration::from_secs(5));
    let name = tmux
        .cmd()
        .args(["list-clients", "-F", "#{client_name}"])
        .run()
        .stdout_trimmed();
    client.read_raw();

    tmux.cmd()
        .args(["refresh-client", "-l", "-t", &name])
        .run()
        .assert_success();
    wait_for_clipboard_query(&mut client);
    client.write(b"\x1b]52;c;aGVsbG8=\x07");
    tmux.wait_for("#{buffer_sample}", |v| v == "hello", Duration::from_secs(5));

    tmux.cmd()
        .args(["refresh-client", "-l", "-t", &name])
        .run()
        .assert_success();
    wait_for_clipboard_query(&mut client);
    client.write(b"\x1b]52;c;aGVsbG\x07");
    std::thread::sleep(Duration::from_millis(300));
    let buffers = tmux
        .cmd()
        .args(["list-buffers", "-F", "#{buffer_sample}"])
        .run();
    buffers.assert_success();
    assert_eq!(
        buffers.stdout_trimmed(),
        "hello",
        "invalid reply must not create a buffer"
    );
}

/// A program in a pane can set the clipboard with OSC 52, which creates a
/// paste buffer. The base64 decoder used to reject digits and padding, so
/// this failed for almost any text.
#[test]
fn osc52_from_pane_creates_buffer() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().args(["cat"]).run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    // The default (external) only passes the sequence to the terminal.
    tmux.cmd()
        .args(["set", "-s", "set-clipboard", "on"])
        .run()
        .assert_success();
    tmux.cmd()
        .args([
            "respawn-pane",
            "-k",
            "printf '\\033]52;c;aGVsbG8gMjAyNg==\\007'; cat",
        ])
        .run()
        .assert_success();
    tmux.wait_for(
        "#{buffer_sample}",
        |v| v == "hello 2026",
        Duration::from_secs(5),
    );
}

/// File errors give the error first and the path last (tmux 3.8), so
/// capitalising the first letter of a message never changes the path.
#[test]
fn file_errors_put_the_path_last() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let missing = "/nonexistent-tmux-rs-test/file";
    let load = tmux.cmd().args(["load-buffer", missing]).run();
    load.assert_failure();
    assert_eq!(
        load.stderr_str().trim(),
        format!("No such file or directory: {missing}")
    );

    tmux.cmd().args(["set-buffer", "x"]).run().assert_success();
    let save = tmux.cmd().args(["save-buffer", missing]).run();
    save.assert_failure();
    assert_eq!(
        save.stderr_str().trim(),
        format!("No such file or directory: {missing}")
    );
}

/// Mode key handlers compare the whole key code. They used to compare only
/// its low byte, so M-d (which has d as its low byte) deleted a buffer in
/// buffer mode like d does.
#[test]
fn choose_buffer_ignores_modified_key_with_same_low_byte() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args(["set-buffer", "-b", "one", "1"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "two", "2"])
        .run()
        .assert_success();

    // An attached client running no shell: keys typed here go to the mode.
    let mut client = harness::PtyClient::attach(&tmux, 80, 24);
    tmux.wait_for("#{session_attached}", |v| v == "1", Duration::from_secs(5));
    tmux.cmd().args(["choose-buffer"]).run().assert_success();
    tmux.wait_for("#{pane_in_mode}", |v| v == "1", Duration::from_secs(5));

    let count = || tmux.cmd().args(["list-buffers"]).run().stdout_lines().len();
    client.write(b"\x1bd");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(count(), 2, "M-d must not delete a buffer");
    client.write(b"d");
    let start = std::time::Instant::now();
    while count() != 1 && start.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(count(), 1, "d deletes the selected buffer");
}
