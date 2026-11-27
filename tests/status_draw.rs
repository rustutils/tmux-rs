//! How status lines are drawn (`format_draw`), checked against tmux 3.8:
//! alignments, fill, trimming, window lists with markers and focus,
//! ranges and styles. The client under test is attached from a pane of
//! an outer 3.8 server (see `prompt_draw.rs`), whose capture-pane shows
//! what it drew. No keys are sent to a shell: the inner panes run sleep
//! and the outer pane runs the client.

use std::time::Duration;

use super::prompt_draw::{Nested, STATUS, WIDTH};

const T: Duration = Duration::from_secs(5);

/// 3.8's default status-format[0].
const STATUS_FORMAT_38: &str = "#[align=left range=left #{E:status-left-style}]#[push-default]#{T;=/#{status-left-length}:status-left}#[pop-default]#[norange default]#[list=on align=#{status-justify}]#[list=left-marker]<#[list=right-marker]>#[list=on]#{W:#[range=window|#{window_index} #{E:window-status-style}#{?#{&&:#{window_last_flag},#{!=:#{E:window-status-last-style},default}}, #{E:window-status-last-style},}#{?#{&&:#{window_bell_flag},#{!=:#{E:window-status-bell-style},default}}, #{E:window-status-bell-style},#{?#{&&:#{||:#{window_activity_flag},#{window_silence_flag}},#{!=:#{E:window-status-activity-style},default}}, #{E:window-status-activity-style},}}]#[push-default]#{T:window-status-format}#[pop-default]#[norange default]#{?loop_last_flag,,#{E:window-status-separator}},#[range=window|#{window_index} list=focus #{?#{!=:#{E:window-status-current-style},default},#{E:window-status-current-style},#{E:window-status-style}}#{?#{&&:#{window_last_flag},#{!=:#{E:window-status-last-style},default}}, #{E:window-status-last-style},}#{?#{&&:#{window_bell_flag},#{!=:#{E:window-status-bell-style},default}}, #{E:window-status-bell-style},#{?#{&&:#{||:#{window_activity_flag},#{window_silence_flag}},#{!=:#{E:window-status-activity-style},default}}, #{E:window-status-activity-style},}}]#[push-default]#{T:window-status-current-format}#[pop-default]#[norange list=on default]#{?loop_last_flag,,#{E:window-status-separator}}}#[nolist align=right range=right #{E:status-right-style}]#[push-default]#{T;=/#{status-right-length}:status-right}#[pop-default]#[norange default]";

/// A client drawing `format` as its status line, which it waits for to
/// start with `first`.
fn with_format(format: &str, first: &str) -> Nested {
    // 3.8's default status-style is a theme colour tmux-rs does not have
    // yet (Track 3), so set the 3.5 one.
    let n = Nested::new(&[&["set", "-g", "status-style", "bg=green,fg=black"]]);
    n.run(&["set", "-g", "status-format[0]", format]);
    n.wait_line(STATUS, |l| l.starts_with(first));
    n
}

/// Adds windows named `names` running sleep, after window 0 named
/// `first`, with fixed status-left and status-right and 3.8's default
/// status-format[0].
fn window_list(first: &str, names: &[&str]) -> Nested {
    let n = Nested::new(&[]);
    n.run(&["rename-window", "-t", ":0", first]);
    for name in names {
        n.run(&["new-window", "-d", "-n", name, "sleep 1000"]);
    }
    n.run(&["set", "-g", "status-left", "[L] "]);
    n.run(&["set", "-g", "status-right", "R"]);
    n.run(&["set", "-g", "status-format[0]", STATUS_FORMAT_38]);
    n.wait_line(STATUS, |l| l.starts_with("[L] "));
    n
}

#[test]
fn window_list_that_fits() {
    let n = window_list("aaa", &["bbb", "ccc"]);
    n.wait_line(STATUS, |l| l == "[L] 0:aaa* 1:bbb  2:ccc                R");
}

#[test]
fn window_list_trimmed_around_the_current_window() {
    let names = ["w1xxxx", "w2xxxx", "w3xxxx", "w4xxxx", "w5xxxx", "w6xxxx"];
    let n = window_list("w0xxxx", &names);
    // The current window is first, so the list shows its start and the
    // right marker.
    n.wait_line(STATUS, |l| l == "[L] 0:w0xxxx* 1:w1xxxx  2:w2xxxx  3:w3>R");
    // In the middle: both markers, the current window in view.
    n.run(&["select-window", "-t", ":4"]);
    n.wait_line(STATUS, |l| l == "[L] <  3:w3xxxx  4:w4xxxx* 5:w5xxxx  6>R");
    // At the end: the left marker only.
    n.run(&["select-window", "-t", ":6"]);
    n.wait_line(STATUS, |l| l == "[L] <xxx  4:w4xxxx- 5:w5xxxx  6:w6xxxx*R");
}

#[test]
fn window_list_centred_and_right() {
    let n = window_list("aaa", &["bbb"]);
    n.run(&["set", "-g", "status-justify", "centre"]);
    n.wait_line(STATUS, |l| l == "[L]            0:aaa* 1:bbb            R");
    n.run(&["set", "-g", "status-justify", "right"]);
    n.wait_line(STATUS, |l| l == "[L]                       0:aaa* 1:bbb R");
    n.run(&["set", "-g", "status-justify", "absolute-centre"]);
    n.wait_line(STATUS, |l| l == "[L]          0:aaa* 1:bbb              R");
}

#[test]
fn alignments_and_fill() {
    let n = with_format(
        "#[fill=blue]L#[align=centre]CC#[align=right]RRR#[align=absolute-centre]A",
        "L",
    );
    // The centre is centred between the left and the right, the absolute
    // centre over everything, drawn last.
    n.wait_line(STATUS, |l| l == "L                 CA                 RRR");
    let styled = n.styled(STATUS);
    assert!(styled.contains("\x1b[44m"), "{styled:?}");
}

#[test]
fn trimming_keeps_the_left_and_the_end_of_the_right() {
    let left = "l".repeat(30);
    let right = format!("{}END", "r".repeat(12));
    let n = with_format(
        &format!("{left}#[align=centre]cccccccc#[align=right]{right}"),
        "lll",
    );
    let want = format!("{left}rrrrrrrEND");
    assert_eq!(want.len(), usize::from(WIDTH));
    n.wait_line(STATUS, |l| l == want);
}

#[test]
fn styles_push_pop_and_set_default() {
    let n = with_format(
        "#[fg=red]#[set-default]a#[fg=blue]b#[pop-default default]c #[fg=green]#[push-default]d#[fg=blue]e#[pop-default default]f#[bold]##[g] ###[default]h",
        "abc",
    );
    n.wait_line(STATUS, |l| l == "abc def#[g] #h");
    // a is red and set-default makes red the default, so default gives
    // red again for c and h. push-default makes green the default until
    // pop-default, which comes after its own default (green for f).
    let styled = n.styled(STATUS);
    assert_eq!(
        styled,
        "\u{1b}[31m\u{1b}[42ma\u{1b}[34mb\u{1b}[31mc \u{1b}[32md\u{1b}[34me\u{1b}[32mf\u{1b}[1m#[g] #\u{1b}[0m\u{1b}[31m\u{1b}[42mh\u{1b}[30m",
    );
}

#[test]
fn focus_outside_a_list_draws_the_rest() {
    let n = with_format("ab#[list=focus]cd#[list=left-marker]ef", "ab");
    n.wait_line(STATUS, |l| l == "abcdef");
}

/// Without the hyperlinks cargo feature tmux-rs sends no links to the
/// terminal at all (panes neither), so this passes with
/// `--features hyperlinks` or a C server.
#[test]
#[cfg_attr(not(feature = "hyperlinks"), ignore = "needs the hyperlinks feature")]
fn links_are_drawn() {
    let n = with_format("a#[link=https://example.com/x]bc#[nolink]d", "abcd");
    let styled = n.styled(STATUS);
    assert!(
        styled.contains("https://example.com/x"),
        "no link in {styled:?}"
    );
}

/// A click just after a window's range (on the separator) selects
/// nothing: the range ends where the next style starts (3.8 fixed it
/// being one column longer).
#[test]
fn ranges_end_where_their_text_ends() {
    let n = Nested::new(&[&["set", "-g", "mouse", "on"]]);
    n.run(&["new-window", "-d", "sleep 1000"]);
    n.run(&["select-window", "-t", ":1"]);
    n.run(&[
        "set",
        "-g",
        "status-format[0]",
        "#[range=window|0]AAA#[norange] #[range=window|1]BBB#[norange]",
    ]);
    n.wait_line(STATUS, |l| l == "AAA BBB");
    n.click(3, STATUS);
    // Give the click time to do anything, then check it did not.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(n.inner.query("#{window_index}"), "1");
    n.click(2, STATUS);
    n.inner.wait_for("#{window_index}", |v| v == "0", T);
}
