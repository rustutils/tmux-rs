use std::path::PathBuf;
use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

fn regress_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("regress")
}

// Ported from regress/utf8-test.sh
// Plays UTF-8-test.txt into a pane via cat, then captures and compares
// against utf8-test.result.
#[test]
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
// Tests rendering of combining characters and compares capture-pane -pe output.
#[test]
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
printf '\\e[8;1H\\360\\237\\244\\267\\342\\200\\215\\342\\231\\202\\357\\270\\2175\\n'
printf '\\e[9;1H\\360\\237\\244\\267\\e[9;1H\\342\\200\\215\\342\\231\\202\\357\\270\\2176\\n'
printf '\\e[9;1H\\360\\237\\244\\267\\e[9;1H\\342\\200\\215\\342\\231\\202\\357\\270\\2177\\n'
printf '\\e[10;1H\\360\\237\\244\\267\\e[10;3H\\342\\200\\215\\342\\231\\202\\357\\270\\2178\\n'
printf '\\e[11;1H\\360\\237\\244\\267\\e[11;3H\\342\\200\\215\\e[11;3H\\342\\231\\202\\357\\270\\2179\\n'
printf '\\e[12;1H\\360\\237\\244\\267\\e[12;3H\\342\\200\\215\\342\\231\\202\\357\\270\\21710\\n'
printf '\\e[13;1H\\360\\237\\207\\25211\\n'
printf '\\e[14;1H\\360\\237\\207\\270\\360\\237\\207\\25212\\n'
printf '\\e[15;1H\\360\\237\\207\\270  \\010\\010\\360\\237\\207\\25213\\n'
sleep 5
";
    tmux.cmd()
        .args(["-f/dev/null", "new", "-d", script])
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    std::thread::sleep(Duration::from_secs(1));

    let captured = tmux.cmd().args(["capture-pane", "-pe"]).run().stdout_str();
    assert_eq!(captured, expected);
}

/// Start a pane that prints `printf_arg` with printf and then waits.
/// Returns once the pane has printed a trailing marker "|".
fn print_in_pane(printf_arg: &str) -> TmuxTestHarness {
    let t = Duration::from_secs(5);
    let mut tmux = TmuxTestHarness::new();
    let cmd = format!("printf '{printf_arg}|'; exec sleep 1000");
    tmux.new_session()
        .args(["-x", "40", "-y", "5", &cmd])
        .run()
        .assert_success();
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

/// Hangul Jamo compose into syllables: an initial consonant starts a cell,
/// a vowel and a final consonant join it (upstream 206f4924).
#[test]
fn hangul_jamo_compose_into_one_cell() {
    // U+1100 (choseong), U+1161 (jungseong), U+11A8 (jongseong).
    let tmux = print_in_pane(r"\341\204\200\341\205\241\341\206\250x");
    assert_eq!(tmux.query("#{cursor_x}"), "4");
    assert_eq!(
        tmux.capture_pane().lines().next().unwrap(),
        "\u{1100}\u{1161}\u{11a8}x|"
    );
}

/// Two regional indicators make one flag, a third starts a new cell.
#[test]
fn regional_indicators_pair_up() {
    // U+1F1EC U+1F1E7 (GB flag, width 2) then U+1F1EB (width 1), then the
    // marker.
    let tmux = print_in_pane(r"\360\237\207\254\360\237\207\247\360\237\207\253");
    assert_eq!(tmux.query("#{cursor_x}"), "4");
}
