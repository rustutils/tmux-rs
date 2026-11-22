use super::harness;

use std::time::Duration;

use harness::TmuxTestHarness;

// Ported from regress/am-terminal.sh
// Tests drawing the status line on a terminal without auto margins (am@).
// An inner server is attached from a pane of an outer server, and the last
// line of the outer pane is the inner status line. Without auto margins
// the last column cannot be written, so the R of RRR there is dropped.
#[test]
fn am_terminal_status_line() {
    let t = Duration::from_secs(5);
    let mut inner = TmuxTestHarness::new();
    inner.new_session().run().assert_success();
    inner.wait_ready(t);
    for args in [
        ["set", "-as", "terminal-overrides", ",*:am@"],
        ["set", "-g", "status-right", "RRR"],
        ["set", "-g", "status-left", "LLL"],
        ["set", "-g", "window-status-current-format", "WWW"],
    ] {
        inner.cmd().args(args).run().assert_success();
    }

    let mut outer = TmuxTestHarness::new();
    let attach = format!("{} -S {} attach", harness::client_bin().display(), inner.socket_path());
    outer.new_session().args(["-x", "20", "-y", "2", &attach]).run().assert_success();
    outer.wait_ready(t);

    let start = std::time::Instant::now();
    loop {
        let captured = outer.cmd().args(["capture-pane", "-p"]).run().stdout_str();
        let last = captured.lines().last().unwrap_or("").to_string();
        if last == "LLLWWW           RR" {
            break;
        }
        assert!(start.elapsed() < t, "last line is {last:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}
