//! How prompts are drawn, where their cursor is, and prompts in panes and
//! modes, checked against tmux 3.8.
//!
//! The client of the server under test runs in a pane of an outer server,
//! which is always /usr/bin/tmux and serves as the terminal: capture-pane
//! and the cursor formats of the outer pane show what the client drew.
//! Keys are sent to the outer pane, so they reach the client as typed
//! keys. The outer pane runs the tmux client, never a shell.

use std::time::{Duration, Instant};

use harness::TmuxTestHarness;

use super::harness;

const T: Duration = Duration::from_secs(5);
const WIDTH: u16 = 40;
const HEIGHT: u16 = 12;

/// The server under test with a client attached in an outer server's pane.
struct Nested {
    inner: TmuxTestHarness,
    outer: TmuxTestHarness,
    name: String,
}

impl Nested {
    /// Starts the inner server with a pane running sleep, runs `setup`
    /// on it and attaches a client from a 40x12 outer pane.
    fn new(setup: &[&[&str]]) -> Self {
        let mut inner = TmuxTestHarness::new();
        inner
            .new_session()
            .args(["-x", "40", "-y", "12", "sleep 1000"])
            .run()
            .assert_success();
        inner.wait_ready(T);
        inner
            .cmd()
            .args(["set", "-s", "escape-time", "10"])
            .run()
            .assert_success();
        for args in setup {
            inner.cmd().args(*args).run().assert_success();
        }

        let mut outer = TmuxTestHarness::with_bin("/usr/bin/tmux");
        let attach = format!(
            "env TERM=tmux-256color {} -S {} attach",
            harness::client_bin().display(),
            inner.socket_path()
        );
        outer
            .new_session()
            .args([
                "-x",
                &WIDTH.to_string(),
                "-y",
                &HEIGHT.to_string(),
                &attach,
                ";",
                "set",
                "-g",
                "status",
                "off",
            ])
            .run()
            .assert_success();
        inner.wait_for("#{session_attached}", |v| v == "1", T);
        let name = inner
            .cmd()
            .args(["list-clients", "-F", "#{client_name}"])
            .run()
            .stdout_trimmed();
        let n = Self { inner, outer, name };
        // Wait until the client has drawn its status line at full size.
        n.wait_line(HEIGHT - 1, |l| l.starts_with("[0]"));
        n
    }

    /// Runs a command on the inner server.
    fn run(&self, args: &[&str]) {
        self.inner.cmd().args(args).run().assert_success();
    }

    /// Opens a command-prompt with `args` on the client.
    fn prompt(&self, args: &[&str]) {
        self.inner
            .cmd()
            .args(["command-prompt", "-b", "-t", &self.name])
            .args(args)
            .run()
            .assert_success();
    }

    /// Sends keys (tmux key names) to the outer pane, so they reach the
    /// client.
    fn keys(&self, keys: &[&str]) {
        for key in keys {
            self.outer
                .cmd()
                .args(["send-keys", key])
                .run()
                .assert_success();
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// Clicks the left mouse button at column `x` and row `y` of the
    /// client (both from 0).
    fn click(&self, x: u16, y: u16) {
        let seq = format!("\x1b[<0;{};{}M\x1b[<0;{};{}m", x + 1, y + 1, x + 1, y + 1);
        self.outer
            .cmd()
            .args(["send-keys", "-l", &seq])
            .run()
            .assert_success();
        std::thread::sleep(Duration::from_millis(30));
    }

    /// The client's screen as text.
    fn screen(&self) -> Vec<String> {
        let out = self
            .outer
            .cmd()
            .args(["capture-pane", "-p"])
            .run()
            .stdout_str();
        out.lines().map(str::to_string).collect()
    }

    /// Row `y` of the client's screen with its escape sequences.
    fn styled(&self, y: u16) -> String {
        let out = self
            .outer
            .cmd()
            .args([
                "capture-pane",
                "-pe",
                "-S",
                &y.to_string(),
                "-E",
                &y.to_string(),
            ])
            .run()
            .stdout_str();
        out.trim_end_matches('\n').to_string()
    }

    /// Waits until row `y` matches `pred` and returns it.
    fn wait_line(&self, y: u16, pred: impl Fn(&str) -> bool) -> String {
        let start = Instant::now();
        loop {
            let screen = self.screen();
            let line = screen.get(usize::from(y)).cloned().unwrap_or_default();
            if pred(&line) {
                return line;
            }
            assert!(
                start.elapsed() < T,
                "row {y} is {line:?}, screen:\n{}",
                screen.join("\n")
            );
            std::thread::sleep(Duration::from_millis(30));
        }
    }

    /// Waits until the outer pane's `format` (cursor formats) is `want`.
    fn wait_outer(&self, format: &str, want: &str) {
        self.outer.wait_for(format, |v| v == want, T);
    }

    /// Waits for the cursor to be visible at column `x` and row `y`.
    fn wait_cursor(&self, x: u16, y: u16) {
        self.wait_outer(
            "#{cursor_x},#{cursor_y},#{cursor_flag}",
            &format!("{x},{y},1"),
        );
    }

    /// Waits until the inner option `option` is `want`.
    fn wait_option(&self, option: &str, want: &str) {
        self.inner
            .wait_for(&format!("#{{{option}}}"), |v| v == want, T);
    }

    /// Clears the inner option `option`.
    fn unset(&self, option: &str) {
        self.run(&["set", "-gu", option]);
    }
}

/// The status line row.
const STATUS: u16 = HEIGHT - 1;
/// The pane's last row, above the status line.
const PANE_LAST: u16 = HEIGHT - 2;

#[test]
fn message_format_wraps_the_prompt() {
    let n = Nested::new(&[
        &["set", "-g", "message-style", "bg=red,fill=red"],
        &[
            "set",
            "-g",
            "message-format",
            "[#{prompt_type}|#{prompt_flags}|#{command_prompt}]#{message}",
        ],
    ]);
    n.prompt(&["-1", "-p", "P>", "-I", "hi", "set -g @r '%%'"]);
    n.wait_line(STATUS, |l| l.starts_with("[command|SINGLE|0]P> hi"));
    n.keys(&["C-c"]);

    n.prompt(&["-T", "search", "-p", "S>", "set -g @r '%%'"]);
    n.wait_line(STATUS, |l| l == "[search||0]S>");
    n.keys(&["C-c"]);

    // In vi command mode #{command_prompt} is 1.
    n.run(&["set", "-g", "status-keys", "vi"]);
    n.prompt(&["-p", "V>", "set -g @r '%%'"]);
    n.wait_line(STATUS, |l| l == "[command||0]V>");
    n.keys(&["Escape"]);
    n.wait_line(STATUS, |l| l == "[command|COMMANDMODE|1]V>");
}

#[test]
fn message_format_wraps_messages() {
    let n = Nested::new(&[&["set", "-g", "message-format", "<#{message}>"]]);
    n.run(&["display-message", "-t", &n.name, "some message"]);
    n.wait_line(STATUS, |l| l.starts_with("<some message>"));
}

#[test]
fn message_style_fills_the_line() {
    let n = Nested::new(&[
        &["set", "-g", "message-style", "bg=red"],
        &["set", "-g", "status-right", "RIGHT"],
    ]);
    // Without fill the status line shows through after the prompt.
    n.prompt(&["-p", "P>", "set -g @r '%%'"]);
    n.wait_line(STATUS, |l| l.starts_with("P>  ") && l.ends_with("RIGHT"));
    n.keys(&["C-c"]);
    n.run(&["set", "-g", "message-style", "bg=red,fill=red"]);
    n.prompt(&["-p", "P>", "set -g @r '%%'"]);
    n.wait_line(STATUS, |l| l == "P>");
    let styled = n.styled(STATUS);
    assert!(styled.contains("\x1b[41m"), "{styled:?}");
}

#[test]
fn prompt_shows_the_terminal_cursor() {
    let n = Nested::new(&[]);
    n.prompt(&["-p", "P>", "-I", "hello", "set -g @r '%%'"]);
    n.wait_line(STATUS, |l| l.starts_with("P> hello"));
    n.wait_cursor(8, STATUS);
    n.keys(&["Left", "Left"]);
    n.wait_cursor(6, STATUS);
    n.keys(&["C-c"]);
    // Back on the pane, whose program has not shown a cursor.
    n.wait_line(STATUS, |l| l.starts_with("[0]"));
}

#[test]
fn prompt_cursor_style_and_colour() {
    let n = Nested::new(&[
        &["set", "-g", "prompt-cursor-style", "blinking-bar"],
        &["set", "-g", "prompt-cursor-colour", "green"],
        &["set", "-g", "prompt-command-cursor-style", "block"],
        &["set", "-g", "prompt-command-cursor-colour", "blue"],
        &["set", "-g", "status-keys", "vi"],
    ]);
    n.prompt(&["-p", "P>", "-I", "ab", "set -g @r '%%'"]);
    n.wait_cursor(5, STATUS);
    n.wait_outer(
        "#{cursor_shape},#{cursor_colour},#{cursor_blinking}",
        "bar,#008000,1",
    );
    n.keys(&["Escape"]);
    n.wait_outer(
        "#{cursor_shape},#{cursor_colour},#{cursor_blinking}",
        "block,#000080,0",
    );
}

#[test]
fn long_input_scrolls() {
    let n = Nested::new(&[&["set", "-g", "message-style", "fill=red"]]);
    n.prompt(&[
        "-p",
        "P>",
        "-I",
        "0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJ",
        "set -g @r '%%'",
    ]);
    n.wait_line(STATUS, |l| l == "P> abcdefghijklmnopqrstuvwxyzABCDEFGHIJ");
    n.wait_cursor(39, STATUS);
    for _ in 0..30 {
        n.keys(&["Left"]);
    }
    n.wait_line(STATUS, |l| l == "P> 0123456789abcdefghijklmnopqrstuvwxyzA");
    n.wait_cursor(19, STATUS);
}

#[test]
fn mouse_click_moves_the_cursor() {
    let n = Nested::new(&[&["set", "-g", "mouse", "on"]]);
    n.prompt(&["-p", "P>", "-I", "abcdef", "set -g @r '[%%]'"]);
    n.wait_cursor(9, STATUS);
    n.click(5, STATUS);
    n.wait_cursor(5, STATUS);
    n.keys(&["X", "Enter"]);
    n.wait_option("@r", "[abXcdef]");

    // A click on the message goes to the start of the input.
    n.prompt(&["-p", "P>", "-I", "abcdef", "set -g @r '[%%]'"]);
    n.wait_cursor(9, STATUS);
    n.click(0, STATUS);
    n.wait_cursor(3, STATUS);
    n.keys(&["X", "Enter"]);
    n.wait_option("@r", "[Xabcdef]");
}

#[test]
fn mouse_click_picks_a_completion() {
    let n = Nested::new(&[&["set", "-g", "mouse", "on"]]);
    n.prompt(&["-p", "P>", "set -g @r '[%%]'"]);
    n.wait_cursor(3, STATUS);
    n.keys(&["l", "i", "Tab"]);
    n.wait_line(STATUS, |l| l.starts_with("P> li link-window list-buffers"));
    n.click(19, STATUS);
    n.wait_line(STATUS, |l| l.starts_with("P> list-buffers"));
    n.keys(&["Enter"]);
    n.wait_option("@r", "[list-buffers ]");
}

#[test]
fn prompt_freezes_the_pane_unless_dash_c() {
    let n = Nested::new(&[&[
        "respawn-pane",
        "-k",
        "sh -c 'i=0; while :; do i=$((i+1)); echo $i; sleep .1; done'",
    ]]);
    let last = |n: &Nested| -> String {
        n.screen()
            .iter()
            .take(usize::from(PANE_LAST) + 1)
            .rfind(|l| !l.is_empty())
            .cloned()
            .unwrap_or_default()
    };
    n.wait_line(0, |l| !l.is_empty());

    n.prompt(&["-p", "P>", "set -g @r '%%'"]);
    n.wait_line(STATUS, |l| l.starts_with("P>"));
    std::thread::sleep(Duration::from_millis(200));
    let before = last(&n);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(last(&n), before, "the pane is frozen under the prompt");
    n.keys(&["C-c"]);

    n.prompt(&["-C", "-p", "P>", "set -g @r '%%'"]);
    n.wait_line(STATUS, |l| l.starts_with("P>"));
    let before = last(&n);
    std::thread::sleep(Duration::from_millis(500));
    assert_ne!(last(&n), before, "with -C the pane keeps updating");
}

#[test]
fn pane_prompt_is_drawn_over_the_pane() {
    let n = Nested::new(&[]);
    n.prompt(&["-P", "-p", "PANE>", "-I", "z", "set -g @r '[%%]'"]);
    n.wait_line(PANE_LAST, |l| l.starts_with("PANE> z"));
    n.wait_cursor(7, PANE_LAST);
    // The status line stays.
    n.wait_line(STATUS, |l| l.starts_with("[0]"));
    n.keys(&["q", "Enter"]);
    n.wait_option("@r", "[zq]");
    n.wait_line(PANE_LAST, str::is_empty);
}

#[test]
fn pane_prompt_hooks() {
    let n = Nested::new(&[
        &[
            "set-hook",
            "-g",
            "pane-prompt-opened",
            "set -gF @o '#{hook_pane}:#{hook_prompt_type}'",
        ],
        &[
            "set-hook",
            "-g",
            "pane-prompt-closed",
            "set -gF @c '#{hook_pane}:#{hook_prompt_type}:#{hook_window}'",
        ],
        &["split-window", "-h", "sleep 1000"],
    ]);
    let pane = n.inner.query("#{pane_id}");
    let window = n.inner.query("#{window_id}");
    n.prompt(&["-P", "-T", "search", "-p", "P>", "set -g @r '%%'"]);
    n.wait_option("@o", &format!("{pane}:search"));
    n.keys(&["C-c"]);
    n.wait_option("@c", &format!("{pane}:search:{window}"));

    // Killing the pane closes its prompt without the hook.
    n.unset("@c");
    n.prompt(&["-P", "-p", "P>", "set -g @r '%%'"]);
    n.wait_option("@o", &format!("{pane}:command"));
    n.run(&["kill-pane", "-t", &pane]);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(n.inner.query("#{@c}"), "");
}

#[test]
fn pane_prompt_gets_keys_when_another_pane_is_active() {
    let n = Nested::new(&[&["split-window", "-h", "sleep 1000"]]);
    n.run(&["select-pane", "-t", "0"]);
    n.prompt(&["-P", "-p", "PP>", "set -g @r '[%%]'"]);
    n.run(&["select-pane", "-t", "1"]);
    n.wait_line(PANE_LAST, |l| l.starts_with("PP>"));
    n.keys(&["a", "b", "Enter"]);
    n.wait_option("@r", "[ab]");
    assert_eq!(n.inner.query("#{pane_index}"), "1");
}

#[test]
fn pane_prompt_mouse_click() {
    let n = Nested::new(&[&["set", "-g", "mouse", "on"]]);
    n.prompt(&["-P", "-p", "P>", "-I", "abcdef", "set -g @r '[%%]'"]);
    n.wait_cursor(9, PANE_LAST);
    n.click(5, PANE_LAST);
    n.wait_cursor(5, PANE_LAST);
    n.keys(&["Y", "Enter"]);
    n.wait_option("@r", "[abYcdef]");
}

#[test]
fn copy_mode_search_prompt_is_in_the_pane() {
    let n = Nested::new(&[
        &["set", "-g", "mode-keys", "emacs"],
        &[
            "respawn-pane",
            "-k",
            "printf 'one\\ntwo\\nthree\\n'; sleep 1000",
        ],
    ]);
    n.wait_line(2, |l| l == "three");
    n.run(&["copy-mode"]);
    n.keys(&["C-s"]);
    n.wait_line(PANE_LAST, |l| l.starts_with("(search down)"));
    n.wait_line(STATUS, |l| l.starts_with("[0]"));
    n.keys(&["C-c"]);
    n.wait_line(PANE_LAST, |l| !l.starts_with("(search down)"));

    n.keys(&["g"]);
    n.wait_line(PANE_LAST, |l| l.starts_with("(goto line)"));
}

#[test]
#[ignore = "mode prompts are not ported yet (run 13 task 6)"]
fn mode_tree_prompts_are_in_the_mode() {
    let n = Nested::new(&[]);
    n.run(&["choose-tree", "-t", "0"]);
    n.wait_line(0, |l| l.contains("1 windows"));
    n.keys(&["C-s"]);
    n.wait_line(PANE_LAST, |l| l.starts_with("(search)"));
    n.wait_cursor(9, PANE_LAST);
    n.wait_line(STATUS, |l| l.starts_with("[0]"));
    n.keys(&["C-c"]);
    n.wait_line(PANE_LAST, |l| !l.starts_with("(search)"));

    n.keys(&["f"]);
    n.wait_line(PANE_LAST, |l| l.starts_with("(filter)"));
    n.keys(&["C-c"]);

    // The kill prompt of tree mode too.
    n.keys(&["x"]);
    n.wait_line(PANE_LAST, |l| l.starts_with("Kill"));
    n.wait_line(STATUS, |l| l.starts_with("[0]"));
}
