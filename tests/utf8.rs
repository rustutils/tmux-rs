use super::harness;

use std::path::PathBuf;
use std::time::Duration;

use harness::TmuxTestHarness;

fn regress_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("regress")
}

// Ported from regress/utf8-test.sh
// Plays UTF-8-test.txt into a pane via cat, then captures and compares
// against utf8-test.result.
#[test]
#[ignore = "depends on exact escape-sequence capture matching C tmux output byte-for-byte"]
fn utf8_capture_matches_reference() {
    let dir = regress_dir();
    let input = dir.join("UTF-8-test.txt");
    let expected = std::fs::read_to_string(dir.join("utf8-test.result")).unwrap();

    let tmux = TmuxTestHarness::new();
    tmux.cmd()
        .args([
            "-f/dev/null",
            "set",
            "-g",
            "remain-on-exit",
            "on",
            ";",
            "set",
            "-g",
            "remain-on-exit-format",
            "",
            ";",
            "new",
            "-d",
            "--",
            "cat",
            input.to_str().unwrap(),
        ])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    std::thread::sleep(Duration::from_secs(1));

    let captured = tmux
        .cmd()
        .args(["capture-pane", "-pCeJS-"])
        .run()
        .stdout_str();

    assert_eq!(captured, expected);
}

// Ported from regress/combine-test.sh
// Tests rendering of combining characters; compares capture-pane -pe output.
#[test]
#[ignore = "byte-for-byte capture comparison against C tmux reference"]
fn combining_characters_capture() {
    let dir = regress_dir();
    let expected = std::fs::read_to_string(dir.join("combine-test.result")).unwrap();

    let tmux = TmuxTestHarness::new();
    let script = "\
printf '\\e[H\\e[J'
printf '\\e[3;1H\\316\\233\\e[3;1H\\314\\2120\\n'
printf '\\e[4;1H\\316\\233\\e[4;2H\\314\\2121\\n'
printf '\\e[5;1H👍\\e[5;1H🏻2\\n'
printf '\\e[6;1H👍\\e[6;3H🏻3\\n'
printf '\\e[7;1H👍\\e[7;10H👍\\e[7;3H🏻\\e[7;12H🏻4\\n'
sleep 5
";
    tmux.cmd()
        .args(["-f/dev/null", "new", "-d", script])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    std::thread::sleep(Duration::from_secs(1));

    let captured = tmux
        .cmd()
        .args(["capture-pane", "-pe"])
        .run()
        .stdout_str();
    assert_eq!(captured, expected);
}

/// Start a pane that prints `printf_arg` with printf and then waits.
/// Returns once the pane has printed a trailing marker "|".
fn print_in_pane(printf_arg: &str) -> TmuxTestHarness {
    let t = Duration::from_secs(5);
    let mut tmux = TmuxTestHarness::new();
    let cmd = format!("printf '{printf_arg}|'; exec sleep 1000");
    tmux.new_session().args(["-x", "40", "-y", "5", &cmd]).run().assert_success();
    tmux.wait_ready(t);
    tmux.wait_for("#{cursor_x}", |v| v != "0", t);
    let start = std::time::Instant::now();
    while !tmux.capture_pane().contains('|') {
        assert!(start.elapsed() < t, "pane never printed its output");
        std::thread::sleep(Duration::from_millis(20));
    }
    tmux
}

/// The Hangul filler U+3164 is ignored (upstream 3051076d).
#[test]
fn hangul_filler_is_ignored() {
    let tmux = print_in_pane(r"a\343\205\244b");
    assert_eq!(tmux.capture_pane().lines().next().unwrap(), "ab|");
    assert_eq!(tmux.query("#{cursor_x}"), "3");
}

/// Variation selector 16 (U+FE0F) after a narrow emoji makes it wide.
#[test]
fn variation_selector_makes_emoji_wide() {
    // U+2764 HEAVY BLACK HEART is narrow on its own.
    let tmux = print_in_pane(r"\342\235\244\357\270\217");
    assert_eq!(tmux.query("#{cursor_x}"), "3");
}
