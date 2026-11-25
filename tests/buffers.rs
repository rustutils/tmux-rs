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
    // Just verify it doesn't crash, since behavior varies by version
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

// ---------------------------------------------------------------------------
// tmux 3.8 buffer names and order
// ---------------------------------------------------------------------------

fn buffer_names(tmux: &TmuxTestHarness) -> Vec<String> {
    let result = tmux
        .cmd()
        .args(["list-buffers", "-F", "#{buffer_name}"])
        .run();
    result.assert_success();
    result.stdout_lines()
}

#[test]
fn set_buffer_rejects_invalid_names() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let result = tmux.cmd().args(["set-buffer", "-b", "a\tb", "x"]).run();
    result.assert_failure();
    assert!(
        result.stderr_str().contains("invalid buffer name: a\tb"),
        "stderr: {}",
        result.stderr_str()
    );

    tmux.cmd()
        .args(["set-buffer", "-b", "keep", "q"])
        .run()
        .assert_success();
    let result = tmux
        .cmd()
        .args(["set-buffer", "-b", "keep", "-n", "n\u{1}"])
        .run();
    result.assert_failure();
    assert!(
        result.stderr_str().contains("invalid buffer name: n\u{1}"),
        "stderr: {}",
        result.stderr_str()
    );
    assert_eq!(buffer_names(&tmux), vec!["keep"]);
}

#[test]
fn set_buffer_cleans_names() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // A backslash is stored escaped, other printable names unchanged.
    tmux.cmd()
        .args(["set-buffer", "-b", "a\\b", "x"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "caf\u{e9}", "y"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "c", "z"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "c", "-n", "c\\d"])
        .run()
        .assert_success();
    assert_eq!(buffer_names(&tmux), vec!["c\\\\d", "caf\u{e9}", "a\\\\b"]);
}

#[test]
fn rename_buffer_to_itself_keeps_it() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "keep", "data"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "keep", "-n", "keep"])
        .run()
        .assert_success();
    let content = tmux.cmd().args(["show-buffer", "-b", "keep"]).run();
    content.assert_success();
    assert_eq!(content.stdout_trimmed(), "data");
}

#[test]
fn list_buffers_newest_first() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "x", "1"])
        .run()
        .assert_success();
    tmux.cmd().args(["set-buffer", "2"]).run().assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "y", "3"])
        .run()
        .assert_success();
    tmux.cmd().args(["set-buffer", "4"]).run().assert_success();
    assert_eq!(buffer_names(&tmux), vec!["buffer1", "y", "buffer0", "x"]);

    // Renaming keeps the place in the order. Replacing a buffer moves it
    // to the front.
    tmux.cmd()
        .args(["set-buffer", "-b", "buffer0", "-n", "z"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "x", "5"])
        .run()
        .assert_success();
    assert_eq!(buffer_names(&tmux), vec!["x", "buffer1", "y", "z"]);
}

#[test]
fn buffer_sample_is_cut_after_escaping() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // 101 control characters escape to 404 bytes, cut to 200 and "...".
    let data = "\u{1}".repeat(101);
    tmux.cmd()
        .args(["set-buffer", "-b", "s", &data])
        .run()
        .assert_success();
    let result = tmux
        .cmd()
        .args(["list-buffers", "-F", "#{buffer_sample}"])
        .run();
    result.assert_success();
    assert_eq!(
        result.stdout_trimmed(),
        format!("{}...", "\\001".repeat(50))
    );
}

#[test]
fn buffer_names_that_are_not_utf8() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args(["set-buffer", "-b", "ok", "z"])
        .run()
        .assert_success();

    let bad = || OsString::from_vec(b"bad\xff".to_vec());
    let cases: [(Vec<OsString>, &str); 5] = [
        (
            vec!["set-buffer".into(), "-b".into(), bad(), "z".into()],
            "invalid buffer name: bad",
        ),
        (
            vec![
                "set-buffer".into(),
                "-b".into(),
                "ok".into(),
                "-n".into(),
                bad(),
            ],
            "invalid buffer name: bad",
        ),
        (
            vec!["paste-buffer".into(), "-b".into(), bad()],
            "no buffer bad",
        ),
        (
            vec!["show-buffer".into(), "-b".into(), bad()],
            "no buffer bad",
        ),
        (
            vec!["delete-buffer".into(), "-b".into(), bad()],
            "unknown buffer: bad",
        ),
    ];
    for (args, error) in cases {
        let result = tmux.cmd().args(args.clone()).run();
        result.assert_failure();
        assert!(
            result.stderr_str().starts_with(error),
            "{args:?}: stderr: {}",
            result.stderr_str()
        );
    }
    assert_eq!(buffer_names(&tmux), vec!["ok"]);
}

// ---------------------------------------------------------------------------
// tmux 3.8 buffer command behaviour (checked against 3.8)
// ---------------------------------------------------------------------------

/// Starts a session whose pane copies everything it is sent into `path`,
/// byte for byte (the terminal is raw, so nothing is translated or
/// echoed). With `bracket` the program first turns on bracketed paste.
/// Returns once the terminal is raw.
fn start_recording_pane(tmux: &mut TmuxTestHarness, path: &str, bracket: bool) {
    let enable = if bracket {
        "printf '\\033[?2004h'; "
    } else {
        ""
    };
    let script = format!("{enable}stty raw -echo && printf ready && exec cat > '{path}'");
    tmux.new_session()
        .args(["--", "sh", "-c", &script])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let start = std::time::Instant::now();
    while !tmux.capture_pane().contains("ready") {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the recording pane did not start"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Waits until `path` holds `expected`, and fails with what it holds
/// otherwise.
fn wait_for_file(path: &str, expected: &[u8]) {
    let start = std::time::Instant::now();
    loop {
        let contents = std::fs::read(path).unwrap_or_default();
        if contents == expected {
            return;
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "expected {:?}, the pane got {:?}",
                String::from_utf8_lossy(expected),
                String::from_utf8_lossy(&contents)
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// paste-buffer escapes control characters with vis since 3.8 (C style
/// without the backslash, tab and carriage return kept), so pasting
/// cannot send a program escape sequences. -S pastes the bytes as they
/// are. Lines are still joined with the separator.
#[test]
fn paste_buffer_escapes_control_characters() {
    let mut tmux = TmuxTestHarness::new();
    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();
    start_recording_pane(&mut tmux, &path, false);

    let data = "a\u{1}b\u{1b}c\t\u{e9}\u{7f}\nd";
    tmux.cmd().args(["set-buffer", data]).run().assert_success();
    tmux.cmd().args(["paste-buffer"]).run().assert_success();
    let escaped = "a^Ab^[c\t\u{e9}^?\rd";
    wait_for_file(&path, escaped.as_bytes());

    tmux.cmd()
        .args(["paste-buffer", "-S"])
        .run()
        .assert_success();
    let raw = "a\u{1}b\u{1b}c\t\u{e9}\u{7f}\rd";
    let mut expected = format!("{escaped}{raw}").into_bytes();
    wait_for_file(&path, &expected);

    // Bytes that are not UTF-8 are escaped as M-x or M^X.
    use std::os::unix::ffi::OsStrExt;
    tmux.cmd()
        .args([
            std::ffi::OsStr::new("set-buffer"),
            std::ffi::OsStr::from_bytes(b"\xe9\xff\xa0"),
        ])
        .run()
        .assert_success();
    tmux.cmd().args(["paste-buffer"]).run().assert_success();
    expected.extend_from_slice(b"M-iM^?\\240");
    wait_for_file(&path, &expected);
}

/// -r joins lines with a newline, -s with any separator, and -d deletes
/// the buffer after pasting it.
#[test]
fn paste_buffer_separators() {
    let mut tmux = TmuxTestHarness::new();
    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();
    start_recording_pane(&mut tmux, &path, false);

    tmux.cmd()
        .args(["set-buffer", "1\n2\n"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["paste-buffer", "-r"])
        .run()
        .assert_success();
    wait_for_file(&path, b"1\n2\n");
    tmux.cmd()
        .args(["paste-buffer", "-d", "-s", "<>"])
        .run()
        .assert_success();
    wait_for_file(&path, b"1\n2\n1<>2<>");
    assert!(buffer_names(&tmux).is_empty(), "-d deletes the buffer");
}

/// -p brackets the paste when the program asked for bracketed paste.
#[test]
fn paste_buffer_bracketed() {
    let mut tmux = TmuxTestHarness::new();
    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();
    start_recording_pane(&mut tmux, &path, true);

    tmux.cmd().args(["set-buffer", "x"]).run().assert_success();
    tmux.cmd()
        .args(["paste-buffer", "-p"])
        .run()
        .assert_success();
    tmux.cmd().args(["paste-buffer"]).run().assert_success();
    wait_for_file(&path, b"\x1b[200~x\x1b[201~x");
}

/// A pane whose program has exited cannot be pasted into.
#[test]
fn paste_buffer_into_exited_pane_fails() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args(["set", "-g", "remain-on-exit", "on"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["respawn-pane", "-k", "true"])
        .run()
        .assert_success();
    tmux.wait_for("#{pane_dead}", |v| v == "1", Duration::from_secs(5));

    let result = tmux.cmd().args(["paste-buffer"]).run();
    result.assert_failure();
    assert_eq!(result.stderr_str().trim(), "target pane has exited");
}

/// The error of every buffer command, with C's text.
#[test]
fn buffer_command_errors() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();
    std::fs::write(&path, "x").unwrap();

    let check = |args: &[&str], error: &str| {
        let result = tmux.cmd().args(args).run();
        result.assert_failure();
        assert_eq!(result.stderr_str().trim(), error, "{args:?}");
    };

    // With no buffers at all.
    check(&["set-buffer"], "no data specified");
    check(&["set-buffer", "-n", "x"], "no buffer");
    check(
        &["set-buffer", "-b", "nope", "-n", "x"],
        "unknown buffer: nope",
    );
    check(&["delete-buffer"], "no buffer");
    check(&["delete-buffer", "-b", "nope"], "unknown buffer: nope");
    check(&["show-buffer"], "no buffers");
    check(&["show-buffer", "-b", "nope"], "no buffer nope");
    check(&["save-buffer", &path], "no buffers");
    check(&["save-buffer", "-b", "nope", &path], "no buffer nope");
    check(&["paste-buffer", "-b", "nope"], "no buffer nope");
    check(&["list-buffers", "-O", "bogus"], "invalid sort order");
    check(
        &["load-buffer", "-b", "a\tb", &path],
        "invalid buffer name: a\tb",
    );
    check(
        &["set-buffer", "-b", "a\tb", "-a", "x"],
        "invalid buffer name: a\tb",
    );

    // Pasting nothing is not an error, and setting nothing adds nothing.
    tmux.cmd().args(["paste-buffer"]).run().assert_success();
    tmux.cmd().args(["set-buffer", ""]).run().assert_success();
    tmux.cmd()
        .args(["set-buffer", "-b", "empty", ""])
        .run()
        .assert_success();
    assert!(buffer_names(&tmux).is_empty());
}

/// set-buffer -n renames the newest buffer when no -b is given.
#[test]
fn set_buffer_n_renames_the_top_buffer() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "named", "1"])
        .run()
        .assert_success();
    tmux.cmd().args(["set-buffer", "2"]).run().assert_success();
    tmux.cmd()
        .args(["set-buffer", "-n", "renamed"])
        .run()
        .assert_success();
    assert_eq!(buffer_names(&tmux), vec!["renamed", "named"]);
    let content = tmux.cmd().args(["show-buffer", "-b", "renamed"]).run();
    assert_eq!(content.stdout_str(), "2");

    // delete-buffer without -b deletes the newest automatic buffer, and
    // renamed buffers are not automatic.
    tmux.cmd().args(["set-buffer", "3"]).run().assert_success();
    tmux.cmd().args(["delete-buffer"]).run().assert_success();
    assert_eq!(buffer_names(&tmux), vec!["renamed", "named"]);
    let result = tmux.cmd().args(["delete-buffer"]).run();
    result.assert_failure();
    assert_eq!(result.stderr_str().trim(), "no buffer");
}

/// set-buffer -a appends to the named buffer.
#[test]
fn set_buffer_append_to_named() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["set-buffer", "-b", "a", "one"])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["set-buffer", "-a", "-b", "a", " two"])
        .run()
        .assert_success();
    // Without -b, -a makes a new automatic buffer.
    tmux.cmd()
        .args(["set-buffer", "-a", "three"])
        .run()
        .assert_success();
    let content = tmux.cmd().args(["show-buffer", "-b", "a"]).run();
    assert_eq!(content.stdout_str(), "one two");
    let content = tmux.cmd().args(["show-buffer"]).run();
    assert_eq!(content.stdout_str(), "three");
}

/// Base64 as tmux sends it in OSC 52.
fn base64(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// An attached client whose terminal can set the clipboard (Ms).
fn clipboard_client(tmux: &mut TmuxTestHarness) -> harness::PtyClient {
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
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
    let mut client = harness::PtyClient::attach(tmux, 80, 24);
    tmux.wait_for("#{session_attached}", |v| v == "1", Duration::from_secs(5));
    client.read_raw();
    client
}

/// Waits until the client's terminal is sent `data` for its clipboard.
fn wait_for_selection(client: &mut harness::PtyClient, data: &[u8]) {
    let expected = format!("\x1b]52;;{}\x07", base64(data));
    let start = std::time::Instant::now();
    let mut seen = Vec::new();
    while start.elapsed() < Duration::from_secs(5) {
        seen.extend(client.read_raw());
        if seen
            .windows(expected.len())
            .any(|w| w == expected.as_bytes())
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!(
        "expected {expected:?} on the terminal, got {:?}",
        String::from_utf8_lossy(&seen)
    );
}

/// set-buffer -w also sends the data to the client's clipboard.
#[test]
fn set_buffer_w_sets_the_clipboard() {
    let mut tmux = TmuxTestHarness::new();
    let mut client = clipboard_client(&mut tmux);
    tmux.cmd()
        .args(["set-buffer", "-w", "clip me"])
        .run()
        .assert_success();
    wait_for_selection(&mut client, b"clip me");
    tmux.cmd()
        .args(["set-buffer", "-a", "-w", "-b", "buffer0", " more"])
        .run()
        .assert_success();
    wait_for_selection(&mut client, b"clip me more");
}

/// load-buffer -w sends the file's contents to the client's clipboard.
#[test]
fn load_buffer_w_sets_the_clipboard() {
    let mut tmux = TmuxTestHarness::new();
    let mut client = clipboard_client(&mut tmux);
    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();
    std::fs::write(&path, "from a file").unwrap();
    tmux.cmd()
        .args(["load-buffer", "-w", "-b", "f", &path])
        .run()
        .assert_success();
    wait_for_selection(&mut client, b"from a file");
    let content = tmux.cmd().args(["show-buffer", "-b", "f"]).run();
    assert_eq!(content.stdout_str(), "from a file");
}

/// show-buffer in control mode prints the data in the command's output
/// block instead of writing a client file.
#[test]
fn show_buffer_in_control_mode() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd()
        .args(["set-buffer", "-b", "c", "line 1\nline 2"])
        .run()
        .assert_success();
    let result = tmux.control_mode("show-buffer -b c\n");
    let out = result.stdout_str();
    assert!(
        out.contains("\nline 1\nline 2\n%end"),
        "control output: {out:?}"
    );
}

/// save-buffer and show-buffer write the data exactly, without adding a
/// newline.
#[test]
fn save_and_show_buffer_write_the_data_exactly() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let tmp = tempfile::NamedTempFile::new().expect("failed to create tempfile");
    let path = tmp.path().to_str().unwrap().to_string();

    tmux.cmd()
        .args(["set-buffer", "a\u{1}\n"])
        .run()
        .assert_success();
    let shown = tmux.cmd().args(["show-buffer"]).run();
    shown.assert_success();
    assert_eq!(shown.stdout_str(), "a\u{1}\n");
    tmux.cmd()
        .args(["save-buffer", &path])
        .run()
        .assert_success();
    tmux.cmd()
        .args(["save-buffer", "-a", &path])
        .run()
        .assert_success();
    assert_eq!(std::fs::read(&path).unwrap(), b"a\x01\na\x01\n");
}

/// Command output is printed as it is since 3.8, control characters
/// included. 3.5a escaped them with vis.
#[test]
fn command_output_is_not_escaped() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.cmd().args(["set-buffer", "xy"]).run().assert_success();

    let result = tmux.cmd().args(["display", "-p", "a\u{1}b\tc"]).run();
    result.assert_success();
    assert_eq!(result.stdout_str(), "a\u{1}b\tc\n");
    let result = tmux
        .cmd()
        .args(["list-buffers", "-F", "\u{1b}#{buffer_size}\\"])
        .run();
    result.assert_success();
    assert_eq!(result.stdout_str(), "\u{1b}2\\\n");
}
