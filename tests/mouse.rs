//! Default mouse bindings, driven through an attached client.

use super::harness;

use std::time::Duration;

use harness::{PtyClient, TmuxTestHarness};

const TIMEOUT: Duration = Duration::from_secs(5);

/// A server with mouse on and an empty status-left, so the window list
/// starts at column 0 of the status line, and an attached 80x24 client.
fn mouse_setup() -> (TmuxTestHarness, PtyClient) {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().args(["-x", "80", "-y", "24", "sleep 1000"]).run().assert_success();
    tmux.wait_ready(TIMEOUT);
    for opt in [["mouse", "on"], ["status-left", ""]] {
        tmux.cmd().args(["set", "-g", opt[0], opt[1]]).run().assert_success();
    }
    let pty = PtyClient::attach(&tmux, 80, 24);
    tmux.wait_for("#{session_attached}", |v| v == "1", TIMEOUT);
    (tmux, pty)
}

#[test]
fn click_on_status_selects_window() {
    let (tmux, mut pty) = mouse_setup();
    tmux.cmd().args(["new-window", "sleep 1000"]).run().assert_success();
    tmux.wait_for("#{window_index}", |v| v == "1", TIMEOUT);
    // The window list reads "0:sleep- 1:sleep*", click on window 0.
    pty.click(1, 23);
    tmux.wait_for("#{window_index}", |v| v == "0", TIMEOUT);
}

#[test]
#[ignore = "the 3.8 binding needs || with three arguments in formats (Track 3: new format modifiers)"]
fn wheel_up_in_alternate_screen_does_not_enter_copy_mode() {
    let (tmux, mut pty) = mouse_setup();
    tmux.cmd()
        .args(["respawn-pane", "-k", "printf '\\033[?1049h'; exec sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_for("#{alternate_on}", |v| v == "1", TIMEOUT);
    pty.mouse(64, 10, 5, true);
    // There is nothing to wait for when no mode is entered, so give the
    // server a moment to handle the event.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(tmux.query("#{pane_in_mode}"), "0");

    // Outside the alternate screen the wheel still enters copy mode.
    tmux.cmd().args(["respawn-pane", "-k", "sleep 1000"]).run().assert_success();
    tmux.wait_for("#{alternate_on}", |v| v == "0", TIMEOUT);
    pty.mouse(64, 10, 5, true);
    tmux.wait_for("#{pane_in_mode}", |v| v == "1", TIMEOUT);
}

#[test]
fn click_on_border_selects_pane() {
    let (tmux, mut pty) = mouse_setup();
    tmux.cmd().args(["split-window", "-h", "-l", "39", "sleep 1000"]).run().assert_success();
    tmux.wait_for("#{window_panes}", |v| v == "2", TIMEOUT);
    tmux.cmd().args(["select-pane", "-t", "1"]).run().assert_success();
    // The left pane is 40 wide, the border is column 40.
    pty.click(40, 5);
    tmux.wait_for("#{pane_index}", |v| v == "0", TIMEOUT);
}

