//! The command prompt, its history and the fuzzy format modifiers,
//! checked against tmux 3.8.
//!
//! Each prompt test attaches a client whose pane runs sleep, opens a
//! command-prompt on it that stores what is submitted in @r, types keys
//! one at a time and reads @r back.

use std::time::Duration;

use harness::{PtyClient, TmuxTestHarness};

use super::harness;

/// What the prompt runs: the input lands in @r.
const STORE: &str = "set -g @r '%%'";

/// A server with an attached client.
struct Prompt {
    tmux: TmuxTestHarness,
    client: PtyClient,
    name: String,
}

impl Prompt {
    /// Starts a server, runs `setup` commands and attaches a client.
    fn new(setup: &[&[&str]]) -> Self {
        let mut tmux = TmuxTestHarness::new();
        tmux.new_session()
            .args(["sleep 1000"])
            .run()
            .assert_success();
        tmux.wait_ready(Duration::from_secs(5));
        tmux.cmd()
            .args(["set", "-s", "escape-time", "10"])
            .run()
            .assert_success();
        for args in setup {
            tmux.cmd().args(*args).run().assert_success();
        }
        let client = PtyClient::attach(&tmux, 80, 24);
        tmux.wait_for("#{session_attached}", |v| v == "1", Duration::from_secs(5));
        let name = tmux
            .cmd()
            .args(["list-clients", "-F", "#{client_name}"])
            .run()
            .stdout_trimmed();
        Self { tmux, client, name }
    }

    /// Opens a command-prompt with `args` and waits until `shown` is on
    /// the screen.
    fn open(&mut self, args: &[&str], shown: &str) {
        self.client.read_raw();
        self.tmux
            .cmd()
            .args(["command-prompt", "-b", "-t", &self.name])
            .args(args)
            .run()
            .assert_success();
        self.client.wait_for_text(shown, Duration::from_secs(5));
    }

    /// Opens the default test prompt, which stores its input in @r.
    fn open_store(&mut self) {
        self.tmux
            .cmd()
            .args(["set", "-gu", "@r"])
            .run()
            .assert_success();
        self.open(&["-p", "P>", STORE], "P>");
    }

    /// Types `keys`, each written on its own. Escape gets a longer pause,
    /// so it is not taken as the start of the next key: tmux waits 500ms
    /// after an escape while the terminal has not answered its queries,
    /// and this one never does.
    fn keys(&mut self, keys: &[&[u8]]) {
        for key in keys {
            self.client.write(key);
            let pause = if *key == ESCAPE { 700 } else { 30 };
            std::thread::sleep(Duration::from_millis(pause));
        }
    }

    /// Types `text` one character at a time.
    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            let mut buf = [0; 4];
            self.keys(&[c.encode_utf8(&mut buf).as_bytes()]);
        }
    }

    /// Waits until @r is set and returns it.
    fn result(&self) -> String {
        self.tmux
            .wait_for("#{?@r,1,0}", |v| v == "1", Duration::from_secs(5));
        self.tmux.query("#{@r}")
    }

    /// Opens the test prompt, types `text` then `keys`, presses Enter and
    /// returns what was submitted.
    fn submit(&mut self, text: &str, keys: &[&[u8]]) -> String {
        self.open_store();
        self.type_text(text);
        self.keys(keys);
        self.keys(&[b"\r"]);
        self.result()
    }
}

const LEFT: &[u8] = b"\x1b[D";
const RIGHT: &[u8] = b"\x1b[C";
const UP: &[u8] = b"\x1b[A";
const DOWN: &[u8] = b"\x1b[B";
const HOME: &[u8] = b"\x1b[1~";
const END: &[u8] = b"\x1b[4~";
const BSPACE: &[u8] = b"\x7f";
const DELETE: &[u8] = b"\x1b[3~";
const ESCAPE: &[u8] = b"\x1b";

#[test]
fn typing_and_submitting() {
    let mut p = Prompt::new(&[]);
    assert_eq!(p.submit("hello world", &[]), "hello world");
}

#[test]
fn emacs_cursor_movement_and_deletion() {
    let mut p = Prompt::new(&[]);
    // C-a and C-e, arrows, Home and End.
    assert_eq!(
        p.submit("bcd", &[b"\x01", b"a", b"\x05", b"e", LEFT, LEFT, b"X"]),
        "abcXde"
    );
    assert_eq!(
        p.submit("bc", &[HOME, b"a", END, b"d", LEFT, RIGHT, b"e"]),
        "abcde"
    );
    // Backspace and Delete, C-h and C-d.
    assert_eq!(
        p.submit("abcdef", &[BSPACE, b"\x08", HOME, DELETE, b"\x04"]),
        "cd"
    );
    // Word movement: M-b, M-f.
    assert_eq!(
        p.submit("one two three", &[b"\x1bb", b"\x1bb", b"X", b"\x1bf", b"Y"]),
        "one XtwoY three"
    );
}

#[test]
fn emacs_kill_and_yank() {
    let mut p = Prompt::new(&[]);
    // C-u clears the whole line, wherever the cursor is.
    assert_eq!(p.submit("abc def", &[LEFT, LEFT, b"\x15", b"z"]), "z");
    // C-k deletes from the cursor to the end.
    assert_eq!(
        p.submit("abc def", &[LEFT, LEFT, LEFT, b"\x0b", b"g"]),
        "abc g"
    );
    // C-w deletes the word before the cursor and C-y yanks it back.
    assert_eq!(
        p.submit("one two three", &[b"\x17", HOME, b"\x19"]),
        "threeone two"
    );
    // Without anything deleted by C-w, C-y pastes the newest buffer, up
    // to its first control character.
    let mut fresh = Prompt::new(&[&["set-buffer", "pas\u{e9}d\nmore"]]);
    assert_eq!(fresh.submit("x", &[b"\x19"]), "xpas\u{e9}d");
    // M-Backspace is not bound in the prompt.
    assert_eq!(p.submit("one two", &[b"\x1b\x7f"]), "one two");
    // C-t swaps the two characters before the cursor.
    assert_eq!(p.submit("abc", &[b"\x14"]), "acb");
}

#[test]
fn vi_editing() {
    let mut p = Prompt::new(&[&["set", "-g", "status-keys", "vi"]]);
    // Escape goes to command mode, then 0, $, x, i, a, A, I.
    assert_eq!(
        p.submit("bcd", &[ESCAPE, b"0", b"i", b"a", ESCAPE, b"$", b"a", b"e"]),
        "abcde"
    );
    assert_eq!(p.submit("abc", &[ESCAPE, b"0", b"x", b"A", b"d"]), "bcd");
    // dd clears the line, D deletes to the end.
    assert_eq!(p.submit("abc", &[ESCAPE, b"d", b"d", b"i", b"z"]), "z");
    assert_eq!(p.submit("abcd", &[ESCAPE, b"0", b"l", b"D"]), "a");
    // w and b move by words.
    assert_eq!(
        p.submit("one two", &[ESCAPE, b"0", b"w", b"i", b"X"]),
        "one Xtwo"
    );
}

/// 3.8: Escape into vi command mode moves the cursor left, as vi does, so
/// x deletes the last character.
#[test]
fn vi_escape_moves_left() {
    let mut p = Prompt::new(&[&["set", "-g", "status-keys", "vi"]]);
    assert_eq!(p.submit("abc", &[ESCAPE, b"x"]), "ab");
}

/// 3.8: C-v inserts the next key as it is.
#[test]
fn ctrl_v_inserts_the_next_key() {
    let mut p = Prompt::new(&[]);
    assert_eq!(p.submit("a", &[b"\x16", b"\x01", b"b"]), "a\u{1}b");
}

/// 3.8: keypad keys insert the text they show.
#[test]
fn keypad_keys_insert_text() {
    let mut p = Prompt::new(&[]);
    // KP1, KP+ and KP* in application keypad form.
    assert_eq!(p.submit("a", &[b"\x1bOq", b"\x1bOk", b"\x1bOj"]), "a1+*");
}

#[test]
fn escape_and_ctrl_c_cancel() {
    let mut p = Prompt::new(&[]);
    for cancel in [ESCAPE, b"\x03"] {
        p.open_store();
        p.type_text("abc");
        p.keys(&[cancel]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(p.tmux.query("#{?@r,set,unset}"), "unset");
    }
}

#[test]
fn history_up_and_down() {
    let mut p = Prompt::new(&[]);
    assert_eq!(p.submit("first", &[]), "first");
    assert_eq!(p.submit("second", &[]), "second");
    assert_eq!(p.submit("", &[UP]), "second");
    // The same entry twice in a row is kept once, so this is first,
    // second, first now.
    assert_eq!(p.submit("", &[UP, UP]), "first");
    assert_eq!(p.submit("", &[UP, UP, DOWN]), "first");
    // C-p and C-n too. Down past the newest entry gives an empty line.
    assert_eq!(p.submit("typed", &[b"\x10", b"\x0e", b"z"]), "z");

    // Search prompts have their own history.
    p.tmux
        .cmd()
        .args(["set", "-gu", "@r"])
        .run()
        .assert_success();
    p.open(&["-T", "search", "-p", "S>", STORE], "S>");
    p.keys(&[UP, b"z", b"\r"]);
    assert_eq!(p.result(), "z");
}

#[test]
fn initial_text_and_several_prompts() {
    let mut p = Prompt::new(&[]);
    p.tmux
        .cmd()
        .args(["set", "-gu", "@r"])
        .run()
        .assert_success();
    p.open(&["-I", "init", "-p", "I>", STORE], "I>");
    p.type_text("ial");
    p.keys(&[b"\r"]);
    assert_eq!(p.result(), "initial");

    // Several prompts, each input replacing %1, %2.
    p.tmux
        .cmd()
        .args(["set", "-gu", "@r"])
        .run()
        .assert_success();
    p.open(&["-p", "A>,B>", "-I", "x,", "set -g @r '%1-%2'"], "A>");
    p.type_text("1");
    p.keys(&[b"\r"]);
    p.client.wait_for_text("B>", Duration::from_secs(5));
    p.type_text("2");
    p.keys(&[b"\r"]);
    assert_eq!(p.result(), "x1-2");
}

#[test]
fn single_key_and_numeric_prompts() {
    let mut p = Prompt::new(&[]);
    // -1 takes one key and submits it.
    p.tmux
        .cmd()
        .args(["set", "-gu", "@r"])
        .run()
        .assert_success();
    p.open(&["-1", "-p", "K>", STORE], "K>");
    p.keys(&[b"q"]);
    assert_eq!(p.result(), "q");

    // -N accepts only digits.
    p.tmux
        .cmd()
        .args(["set", "-gu", "@r"])
        .run()
        .assert_success();
    p.open(&["-N", "-p", "N>", STORE], "N>");
    p.type_text("4x2");
    p.keys(&[b"\r"]);
    assert_eq!(p.result(), "4");
}

#[test]
fn confirm_before() {
    let mut p = Prompt::new(&[]);
    for (key, expected) in [(b"n", "unset"), (b"y", "yes")] {
        p.tmux
            .cmd()
            .args(["set", "-gu", "@r"])
            .run()
            .assert_success();
        p.client.read_raw();
        p.tmux
            .cmd()
            .args([
                "confirm-before",
                "-b",
                "-t",
                &p.name,
                "-p",
                "sure?",
                "set -g @r yes",
            ])
            .run()
            .assert_success();
        p.client.wait_for_text("sure?", Duration::from_secs(5));
        p.keys(&[key]);
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(p.tmux.query("#{?@r,#{@r},unset}"), expected);
    }
}

#[test]
fn tab_completes_commands() {
    let mut p = Prompt::new(&[]);
    // A unique prefix is completed. Completion adds a space, but
    // trailing spaces never reach the command (see emacs_kill_and_yank).
    assert_eq!(p.submit("set-buf", &[b"\t"]), "set-buffer");
    assert_eq!(p.submit("show-buf", &[b"\t", b"x"]), "show-buffer x");
    // Several candidates leave the text as it is.
    assert_eq!(p.submit("list-", &[b"\t", b"\t"]), "list-");
}

/// 3.8 shows the candidates inline after the text instead of in a menu.
#[test]
fn tab_shows_candidates_inline() {
    let mut p = Prompt::new(&[]);
    p.open_store();
    p.type_text("list-");
    p.client.read_raw();
    p.keys(&[b"\t"]);
    p.client
        .wait_for_text("list- list-buffers list-clients", Duration::from_secs(5));
}

/// 3.8 completes command names only, not aliases.
#[test]
fn tab_does_not_complete_aliases() {
    let mut p = Prompt::new(&[]);
    // "splitw" is an alias of split-window, nothing else starts with it.
    assert_eq!(p.submit("splitw", &[b"\t"]), "splitw");
}

#[test]
fn history_file_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let history = dir.path().join("history");
    let config = dir.path().join("tmux.conf");
    std::fs::write(
        &config,
        format!("set -g history-file '{}'\n", history.display()),
    )
    .unwrap();
    let config_arg = format!("-f{}", config.display());

    {
        let mut tmux = TmuxTestHarness::new();
        tmux.new_session()
            .args(["sleep 1000"])
            .run()
            .assert_success();
        tmux.wait_ready(Duration::from_secs(5));
        tmux.cmd()
            .args(["source-file", config.to_str().unwrap()])
            .run()
            .assert_success();
        let client = PtyClient::attach(&tmux, 80, 24);
        tmux.wait_for("#{session_attached}", |v| v == "1", Duration::from_secs(5));
        let name = tmux
            .cmd()
            .args(["list-clients", "-F", "#{client_name}"])
            .run()
            .stdout_trimmed();
        let mut p = Prompt { tmux, client, name };
        assert_eq!(p.submit("remembered", &[]), "remembered");
        drop(p.client);
        p.tmux.kill_server();
        let start = std::time::Instant::now();
        while !history.exists() && start.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    let saved = std::fs::read_to_string(&history).expect("history file written at exit");
    assert!(saved.contains("command:remembered"), "file: {saved:?}");

    // The history is loaded with the configuration, so -f comes before
    // the command that starts the server.
    let tmux = TmuxTestHarness::new();
    tmux.cmd()
        .args([config_arg.as_str(), "new-session", "-d", "sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    let out = tmux
        .cmd()
        .args(["show-prompt-history", "-T", "command"])
        .run();
    tmux.kill_server();
    out.assert_success();
    assert_eq!(
        out.stdout_str(),
        "History for command:\n\n1: remembered\n\n"
    );
}

#[test]
fn show_and_clear_prompt_history() {
    let mut p = Prompt::new(&[]);
    p.submit("one", &[]);
    p.submit("two", &[]);

    let out = p
        .tmux
        .cmd()
        .args(["show-prompt-history", "-T", "command"])
        .run();
    out.assert_success();
    assert_eq!(
        out.stdout_str(),
        "History for command:\n\n1: one\n2: two\n\n"
    );

    let out = p.tmux.cmd().args(["show-prompt-history"]).run();
    out.assert_success();
    assert_eq!(
        out.stdout_str(),
        "History for command:\n\n1: one\n2: two\n\n\
         History for search:\n\n\n"
    );

    for command in ["show-prompt-history", "clear-prompt-history"] {
        let out = p.tmux.cmd().args([command, "-T", "bogus"]).run();
        out.assert_failure();
        assert_eq!(out.stderr_str().trim(), "invalid type: bogus");
    }

    p.tmux
        .cmd()
        .args(["clear-prompt-history", "-T", "search"])
        .run()
        .assert_success();
    let out = p.tmux.cmd().args(["showphist", "-T", "command"]).run();
    assert_eq!(
        out.stdout_str(),
        "History for command:\n\n1: one\n2: two\n\n"
    );
    p.tmux.cmd().args(["clearphist"]).run().assert_success();
    let out = p.tmux.cmd().args(["showphist", "-T", "command"]).run();
    assert_eq!(out.stdout_str(), "History for command:\n\n\n");
}

/// #{m/z:} is a fuzzy match, #{m/p:} gives the matched columns.
#[test]
fn fuzzy_format_modifiers() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session()
        .args(["sleep 1000"])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    let cases = [
        ("#{m/z:dev bash,dev:1 bash}", "1"),
        ("#{m/z:^dev | ^prod,prod:1 ssh}", "1"),
        ("#{m/z:^dev,prod:1 ssh}", "0"),
        ("#{m/z:dvb,dev:1 bash}", "1"),
        ("#{m/z:bvd,dev:1 bash}", "0"),
        ("#{m/z:'v:1,dev:1 bash}", "1"),
        ("#{m/z:'vb,dev:1 bash}", "0"),
        ("#{m/z:sh$,dev:1 bash}", "1"),
        ("#{m/z:!ssh,dev:1 bash}", "1"),
        ("#{m/z:!bash,dev:1 bash}", "0"),
        ("#{m/z:DEV,dev}", "0"),
        ("#{m/z:dev,DEV}", "1"),
        ("#{m/p:dvb,dev:1 bash}", "0,2,6"),
        ("#{m/p:bash,dev:1 bash}", "6,7,8,9"),
        ("#{m/p:zz,dev}", ""),
        ("#{m/p:b,#[fg=red]ab}", "1"),
        ("#{m/p:\u{e9}b,a\u{e9}b}", "1,2"),
        ("#{m/p:z,abc#[align=right]xyz}", "5"),
        ("#{m/p:b,a#[align=centre]b}", "1"),
        ("#{m/p:ab,a a b}", "2,4"),
        ("#{m/p:^a$,a}", "0"),
        ("#{m/p:a,\u{4e2d}a}", "2"),
        ("#{m/p:\u{4e2d},a\u{4e2d}}", "1,2"),
        ("#{m/z:!,a}", "0"),
        ("#{m/z:,a}", "1"),
        ("#{m/p:,a}", ""),
    ];
    for (format, expected) in cases {
        assert_eq!(tmux.query(format), expected, "{format}");
    }
}

/// 3.8 has only the command and search prompt types. The target types
/// went with target completion.
#[test]
fn only_command_and_search_types() {
    let p = Prompt::new(&[]);
    for (ty, ok) in [
        ("command", true),
        ("search", true),
        ("target", false),
        ("window-target", false),
    ] {
        let out = p
            .tmux
            .cmd()
            .args([
                "command-prompt",
                "-b",
                "-t",
                &p.name,
                "-T",
                ty,
                "-p",
                "T>",
                STORE,
            ])
            .run();
        if ok {
            out.assert_success();
            p.tmux
                .cmd()
                .args(["send-keys", "-K", "-c", &p.name, "Escape"])
                .run();
        } else {
            out.assert_failure();
            assert_eq!(out.stderr_str().trim(), format!("unknown type: {ty}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn backspace_on_empty_prompt_closes_with_dash_e() {
    let mut p = Prompt::new(&[]);
    p.tmux
        .cmd()
        .args(["set", "-gu", "@r"])
        .run()
        .assert_success();
    p.open(&["-e", "-p", "E>", "-I", "ab", "set -g @r '[%%]'"], "E> ab");
    p.keys(&[BSPACE, BSPACE, BSPACE]);
    // The prompt is closed, so these keys go to the pane (sleep) and
    // nothing is stored.
    p.keys(&[b"q", b"\r"]);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(p.tmux.query("#{@r}"), "");

    // Without -e Backspace on an empty prompt does nothing.
    p.open(&["-p", "F>", "set -g @r '[%%]'"], "F>");
    p.keys(&[BSPACE, b"q", b"\r"]);
    assert_eq!(p.result(), "[q]");
}

#[test]
fn dash_l_keeps_commas() {
    let mut p = Prompt::new(&[]);
    p.tmux
        .cmd()
        .args(["set", "-gu", "@r"])
        .run()
        .assert_success();
    // One prompt with the commas, and no space added after it.
    p.open(
        &["-l", "-p", "A>,B>", "-I", "x,y", "set -g @r '[%%|%2]'"],
        "A>,B>x,y",
    );
    p.keys(&[b"\r"]);
    assert_eq!(p.result(), "[x,y|%2]");

    p.tmux
        .cmd()
        .args(["set", "-gu", "@r"])
        .run()
        .assert_success();
    p.open(&["-p", "A>,B>", "-I", "x,y", "set -g @r '[%1|%2]'"], "A> x");
    p.keys(&[b"\r"]);
    p.client.wait_for_text("B> y", Duration::from_secs(5));
    p.keys(&[b"\r"]);
    assert_eq!(p.result(), "[x|y]");
}
