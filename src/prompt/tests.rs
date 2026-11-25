use super::*;
use crate::prompt_history::PromptHistory;

/// History and paste buffers without a server.
#[derive(Default)]
struct Env {
    history: PromptHistory,
    paste: Option<Rc<[u8]>>,
    commands: Vec<&'static str>,
}

impl PromptEnv for Env {
    fn history_up(&mut self, ty: PromptType, positions: &mut HistoryPositions) -> Option<Vec<u8>> {
        self.history.up(ty, positions).map(<[u8]>::to_vec)
    }
    fn history_down(&mut self, ty: PromptType, positions: &mut HistoryPositions) -> Vec<u8> {
        self.history.down(ty, positions).to_vec()
    }
    fn paste_top(&mut self) -> Option<Rc<[u8]>> {
        self.paste.clone()
    }
    fn complete_commands(&mut self, prefix: &[u8]) -> Vec<String> {
        self.commands
            .iter()
            .filter(|c| c.as_bytes().starts_with(prefix))
            .map(ToString::to_string)
            .collect()
    }
}

fn prompt(flags: prompt_flags, keys: modekey, input: &str) -> Prompt {
    Prompt::new(PromptSetup {
        message: b"> ".to_vec(),
        input: input.as_bytes().to_vec(),
        ty: PromptType::Command,
        flags,
        keys,
        word_separators: b"!\"#$%&'()*+,-./:;<=>?@[\\]^`{|}~".to_vec(),
        look: PromptLook::default(),
    })
}

fn emacs(input: &str) -> Prompt {
    prompt(prompt_flags::empty(), modekey::MODEKEY_EMACS, input)
}

fn vi(input: &str) -> Prompt {
    prompt(prompt_flags::empty(), modekey::MODEKEY_VI, input)
}

const ENTER: key_code = b'\r' as key_code;

/// Types `keys` and returns the outcome of the last.
fn press(p: &mut Prompt, env: &mut Env, keys: &[key_code]) -> KeyOutcome {
    let mut last = None;
    for &k in keys {
        last = Some(p.key(k, env));
    }
    last.expect("a key")
}

fn text(keys: &str) -> Vec<key_code> {
    keys.chars().map(|c| c as key_code).collect()
}

fn input(p: &Prompt) -> String {
    String::from_utf8(p.input()).unwrap()
}

/// Presses Enter and returns what was submitted.
fn submit(p: &mut Prompt, env: &mut Env) -> String {
    let out = p.key(ENTER, env);
    assert_eq!(out.fires.len(), 1);
    String::from_utf8(out.fires[0].text.clone().unwrap()).unwrap()
}

#[test]
fn typing_and_submitting() {
    let mut env = Env::default();
    let mut p = emacs("");
    press(&mut p, &mut env, &text("hi there"));
    let out = p.key(ENTER, &mut env);
    assert_eq!(out.result, KeyResult::Handled);
    assert_eq!(out.history, Some((PromptType::Command, b"hi there".to_vec())));
    let fire = &out.fires[0];
    assert_eq!(fire.text.as_deref(), Some(&b"hi there"[..]));
    assert_eq!(fire.kind, KeyResult::Close);
    // The callback decides whether the prompt closes.
    assert_eq!(p.fired(fire, CallbackResult::Close, out.result), KeyResult::Close);
    assert!(p.closed());

    // An empty input is not added to the history.
    let mut p = emacs("");
    assert_eq!(p.key(ENTER, &mut env).history, None);
}

#[test]
fn cancel() {
    let mut env = Env::default();
    for key in [ESCAPE, ctrl(b'c'), ctrl(b'g'), ctrl(b'[')] {
        let mut p = emacs("abc");
        let out = p.key(key, &mut env);
        assert_eq!(out.fires[0].text, None, "{key:#x}");
        assert_eq!(out.fires[0].kind, KeyResult::Close);
    }
}

#[test]
fn emacs_editing() {
    let mut env = Env::default();
    let mut p = emacs("bcd");
    press(&mut p, &mut env, &[ctrl(b'a'), b'a' as key_code, ctrl(b'e'), b'e' as key_code, LEFT, LEFT]);
    press(&mut p, &mut env, &text("X"));
    assert_eq!(input(&p), "abcXde");
    assert_eq!(p.index(), 4);

    let mut p = emacs("abcdef");
    press(&mut p, &mut env, &[BSPACE, ctrl(b'h'), HOME, DC, ctrl(b'd')]);
    assert_eq!(input(&p), "cd");
    // Nothing to delete at either end.
    press(&mut p, &mut env, &[BSPACE, END, DC]);
    assert_eq!(input(&p), "cd");

    let mut p = emacs("one two three");
    press(&mut p, &mut env, &[meta(b'b'), meta(b'b'), b'X' as key_code, meta(b'f'), b'Y' as key_code]);
    assert_eq!(input(&p), "one XtwoY three");
    // Back two words to the start, forward to the end of "one".
    press(&mut p, &mut env, &[LEFT | KEYC_CTRL, LEFT | KEYC_CTRL, RIGHT | KEYC_CTRL]);
    assert_eq!(p.index(), 3);

    // Separators are words of their own.
    let mut p = emacs("a-b");
    press(&mut p, &mut env, &[meta(b'b')]);
    assert_eq!(p.index(), 2);
    press(&mut p, &mut env, &[meta(b'b')]);
    assert_eq!(p.index(), 1);
}

#[test]
fn emacs_kill_yank_and_transpose() {
    let mut env = Env::default();
    let mut p = emacs("abc def");
    press(&mut p, &mut env, &[LEFT, LEFT, ctrl(b'u')]);
    assert_eq!((input(&p), p.index()), (String::new(), 0));

    let mut p = emacs("abc def");
    press(&mut p, &mut env, &[LEFT, LEFT, LEFT, ctrl(b'k')]);
    assert_eq!(input(&p), "abc ");

    let mut p = emacs("one two three");
    press(&mut p, &mut env, &[ctrl(b'w'), HOME, ctrl(b'y')]);
    assert_eq!(input(&p), "threeone two ");

    // Without a C-w, C-y pastes the newest buffer up to a control
    // character, and nothing without one.
    let mut p = emacs("x");
    assert!(p.key(ctrl(b'y'), &mut env).fires.is_empty());
    assert_eq!(input(&p), "x");
    env.paste = Some(Rc::from(&b"pasted\nmore"[..]));
    press(&mut p, &mut env, &[ctrl(b'y')]);
    assert_eq!(input(&p), "xpasted");

    let mut p = emacs("abc");
    press(&mut p, &mut env, &[ctrl(b't')]);
    assert_eq!(input(&p), "acb");
    press(&mut p, &mut env, &[HOME, RIGHT, ctrl(b't')]);
    assert_eq!((input(&p), p.index()), ("cab".to_string(), 2));
}

#[test]
fn vi_editing() {
    let mut env = Env::default();
    let mut p = vi("bcd");
    // Escape goes to command mode and moves left (3.8).
    let out = p.key(ESCAPE, &mut env);
    assert!(out.redraw);
    assert!(p.command_mode());
    assert_eq!(p.index(), 2);
    press(&mut p, &mut env, &text("0ia"));
    assert!(!p.command_mode());
    press(&mut p, &mut env, &[ESCAPE]);
    press(&mut p, &mut env, &text("$ae"));
    assert_eq!(input(&p), "abcde");

    let mut p = vi("abc");
    press(&mut p, &mut env, &[ESCAPE]);
    press(&mut p, &mut env, &text("x"));
    assert_eq!(input(&p), "ab");

    let mut p = vi("abcd");
    press(&mut p, &mut env, &[ESCAPE]);
    press(&mut p, &mut env, &text("0lD"));
    assert_eq!(input(&p), "a");

    let mut p = vi("one two");
    press(&mut p, &mut env, &[ESCAPE]);
    press(&mut p, &mut env, &text("0wiX"));
    assert_eq!(input(&p), "one Xtwo");

    let mut p = vi("abc");
    press(&mut p, &mut env, &[ESCAPE]);
    press(&mut p, &mut env, &text("ddiz"));
    assert_eq!(input(&p), "z");

    // Unknown keys in command mode are dropped.
    let mut p = vi("abc");
    press(&mut p, &mut env, &[ESCAPE]);
    press(&mut p, &mut env, &text("zZ"));
    assert_eq!(input(&p), "abc");
}

#[test]
fn quoted_and_keypad_keys() {
    let mut env = Env::default();
    let mut p = emacs("a");
    press(&mut p, &mut env, &[ctrl(b'v')]);
    assert!(p.quote_next());
    press(&mut p, &mut env, &[ctrl(b'a'), b'b' as key_code]);
    assert_eq!(input(&p), "a\u{1}b");
    assert_eq!(p.buffer()[1].width, 2, "drawn as ^A");

    let mut p = emacs("");
    press(
        &mut p,
        &mut env,
        &[keyc::KEYC_KP_ONE as key_code, keyc::KEYC_KP_PLUS as key_code, keyc::KEYC_KP_PERIOD as key_code],
    );
    assert_eq!(input(&p), "1+.");
}

#[test]
fn history() {
    let mut env = Env::default();
    env.history.add(PromptType::Command, b"first", 10);
    env.history.add(PromptType::Command, b"second", 10);
    let mut p = emacs("typed");
    press(&mut p, &mut env, &[UP]);
    assert_eq!(input(&p), "second");
    press(&mut p, &mut env, &[ctrl(b'p'), ctrl(b'p')]);
    assert_eq!(input(&p), "first");
    press(&mut p, &mut env, &[DOWN]);
    assert_eq!(input(&p), "second");
    press(&mut p, &mut env, &[ctrl(b'n')]);
    assert_eq!(input(&p), "");
}

#[test]
fn completion() {
    let mut env = Env {
        commands: vec!["list-buffers", "list-clients", "set-buffer", "show-buffer"],
        ..Env::default()
    };
    let mut p = emacs("set-b");
    press(&mut p, &mut env, &[b'\t' as key_code]);
    assert_eq!(submit(&mut p, &mut env), "set-buffer ");

    // Several matches with a longer common prefix complete to it.
    let mut p = emacs("li");
    press(&mut p, &mut env, &[b'\t' as key_code]);
    assert_eq!(input(&p), "list-");
    assert_eq!(p.completion_display(), None);
    // Then nothing more to insert: they are shown after the input.
    press(&mut p, &mut env, &[b'\t' as key_code]);
    assert_eq!(input(&p), "list-");
    assert_eq!(p.completion_display().as_deref(), Some(" list-buffers list-clients"));
    // Any key clears them.
    press(&mut p, &mut env, &[LEFT]);
    assert_eq!(p.completion_display(), None);

    // Only the first word of a command prompt completes.
    let mut p = emacs("set-buffer set-b");
    press(&mut p, &mut env, &[b'\t' as key_code]);
    assert_eq!(input(&p), "set-buffer set-b");
    let mut p = prompt(prompt_flags::empty(), modekey::MODEKEY_EMACS, "");
    p.ty = PromptType::Search;
    press(&mut p, &mut env, &text("set-b\t"));
    assert_eq!(input(&p), "set-b", "Tab does nothing in a search prompt");
}

#[test]
fn single_numeric_and_key_prompts() {
    let mut env = Env::default();
    let mut p = prompt(prompt_flags::PROMPT_SINGLE, modekey::MODEKEY_EMACS, "");
    let out = p.key(b'q' as key_code, &mut env);
    assert_eq!(out.fires[0].text.as_deref(), Some(&b"q"[..]));
    assert_eq!(p.fired(&out.fires[0], CallbackResult::Continue, out.result), KeyResult::Handled);
    // Control keys come through as their character.
    let mut p = prompt(prompt_flags::PROMPT_SINGLE, modekey::MODEKEY_EMACS, "");
    let out = p.key(ctrl(b'a'), &mut env);
    assert_eq!(out.fires[0].text.as_deref(), Some(&b"\x01"[..]));

    let mut p = prompt(prompt_flags::PROMPT_NUMERIC, modekey::MODEKEY_EMACS, "");
    press(&mut p, &mut env, &text("42"));
    let out = p.key(b'x' as key_code, &mut env);
    assert_eq!(out.fires[0].text.as_deref(), Some(&b"42"[..]));
    assert_eq!(p.fired(&out.fires[0], CallbackResult::Continue, out.result), KeyResult::NotHandled);
    assert!(p.closed(), "closes whatever the callback says");

    let mut p = prompt(prompt_flags::PROMPT_KEY, modekey::MODEKEY_EMACS, "");
    let out = p.key(ctrl(b'x'), &mut env);
    assert_eq!(out.fires[0].text.as_deref(), Some(&b"C-x"[..]));
    assert_eq!(p.fired(&out.fires[0], CallbackResult::Continue, out.result), KeyResult::Close);
}

#[test]
fn incremental() {
    let mut env = Env::default();
    let mut p = prompt(prompt_flags::PROMPT_INCREMENTAL, modekey::MODEKEY_EMACS, "old");
    assert_eq!(input(&p), "", "the input is kept for C-r and C-s");
    assert_eq!(p.incremental_start().unwrap().text.as_deref(), Some(&b"="[..]));

    let out = p.key(b'a' as key_code, &mut env);
    assert_eq!(out.fires[0].text.as_deref(), Some(&b"=a"[..]));
    assert_eq!(p.fired(&out.fires[0], CallbackResult::Continue, out.result), KeyResult::Handled);
    let out = p.key(ctrl(b'r'), &mut env);
    assert_eq!(out.fires[0].text.as_deref(), Some(&b"-a"[..]));
    let out = p.key(ctrl(b's'), &mut env);
    assert_eq!(out.fires[0].text.as_deref(), Some(&b"+a"[..]));

    // C-r on an empty prompt brings back the input it started with.
    let mut p = prompt(prompt_flags::PROMPT_INCREMENTAL, modekey::MODEKEY_EMACS, "old");
    let out = p.key(ctrl(b'r'), &mut env);
    assert_eq!(out.fires[0].text.as_deref(), Some(&b"=old"[..]));

    // Cursor keys leave the prompt and go on to the key bindings (3.8).
    for key in [UP, DOWN, LEFT, RIGHT, PPAGE, NPAGE] {
        let mut p = prompt(prompt_flags::PROMPT_INCREMENTAL, modekey::MODEKEY_EMACS, "");
        let out = p.key(key, &mut env);
        assert_eq!(out.result, KeyResult::Move);
        assert_eq!(out.fires[0].kind, KeyResult::Move);
        assert_eq!(p.fired(&out.fires[0], CallbackResult::Continue, out.result), KeyResult::Move);
        assert_eq!(p.fired(&out.fires[0], CallbackResult::Close, out.result), KeyResult::Close);
    }
    // Other prompts keep them.
    let mut p = emacs("ab");
    assert_eq!(p.key(LEFT, &mut env).result, KeyResult::Handled);
}

#[test]
fn update_starts_again() {
    let mut env = Env::default();
    env.history.add(PromptType::Command, b"h", 10);
    let mut p = emacs("abc");
    press(&mut p, &mut env, &[UP]);
    p.update(b"next> ".to_vec(), b"xy");
    assert_eq!((p.message(), input(&p).as_str(), p.index()), (&b"next> "[..], "xy", 2));
    press(&mut p, &mut env, &[UP]);
    assert_eq!(input(&p), "h", "history starts from the newest again");
}

#[test]
fn decoding() {
    // Valid UTF-8 is one character only under a UTF-8 locale (mbtowc), so
    // only the round trip is checked here.
    let s = "a\u{e9}\u{4e2d}".as_bytes();
    assert_eq!(utf8_encode(&utf8_decode(s)), s);
    // Invalid bytes stay, each on its own.
    let ud = utf8_decode(b"a\xff\xc3");
    assert_eq!(ud.len(), 3);
    assert_eq!(utf8_encode(&ud), b"a\xff\xc3");
}

/// The text of drawn cells.
fn cells_text(cells: &[Utf8Data]) -> String {
    String::from_utf8_lossy(&utf8_encode(cells)).into_owned()
}

#[test]
fn layout_scrolls_to_the_cursor() {
    let mut env = Env::default();
    let mut p = emacs("0123456789");
    // 8 columns, 3 for the label: 5 for the input, the cursor at the end.
    let pl = p.layout(0, 8, 3, Align::Left);
    assert_eq!(
        (pl.input_x, pl.input_offset, pl.input_width, pl.cursor_x),
        (3, 6, 5, 7)
    );
    assert_eq!(cells_text(&p.input_cells(&pl)), "6789");

    press(&mut p, &mut env, &[ctrl(b'a')]);
    let pl = p.layout(0, 8, 3, Align::Left);
    assert_eq!((pl.input_offset, pl.input_width, pl.cursor_x), (0, 5, 3));
    assert_eq!(cells_text(&p.input_cells(&pl)), "01234");
}

#[test]
fn layout_aligns_and_clips() {
    let p = emacs("ab");
    let pl = p.layout(10, 20, 3, Align::Right);
    assert_eq!((pl.content_x, pl.content_width, pl.input_x, pl.cursor_x), (25, 5, 28, 30));
    let pl = p.layout(10, 20, 3, Align::Centre);
    assert_eq!((pl.content_x, pl.cursor_x), (17, 22));

    // No room: nothing but the area.
    assert_eq!(
        p.layout(5, 0, 3, Align::Left),
        Layout {
            area_x: 5,
            ..Layout::default()
        }
    );
    // A label wider than the area leaves no room for the input.
    let pl = p.layout(0, 4, 10, Align::Left);
    assert_eq!((pl.label_width, pl.input_width, pl.cursor_x), (4, 0, 4));
    assert!(p.input_cells(&pl).is_empty());
}

#[test]
fn input_cells_show_control_characters_and_the_quote() {
    let mut env = Env::default();
    let p = emacs("a\u{1}b");
    let pl = p.layout(0, 20, 2, Align::Left);
    let cells = p.input_cells(&pl);
    assert_eq!(cells_text(&cells), "a^Ab");
    assert_eq!(cells[1].width, 2);

    // C-v shows a ^ at the cursor and widens the input by one.
    let mut p = emacs("ab");
    press(&mut p, &mut env, &[LEFT, ctrl(b'v')]);
    let pl = p.layout(0, 20, 2, Align::Left);
    assert_eq!(pl.input_width, 3);
    assert_eq!(cells_text(&p.input_cells(&pl)), "a^b");
}

#[test]
fn completion_cells_follow_the_cursor() {
    let mut env = Env {
        commands: vec!["list-buffers", "link-window"],
        ..Env::default()
    };
    let mut p = emacs("li");
    press(&mut p, &mut env, &[b'\t' as key_code]);
    let pl = p.layout(0, 40, 2, Align::Left);
    assert_eq!(pl.cursor_x, 4);
    assert_eq!(pl.content_width, 4 + " link-window list-buffers".len() as u32);
    let cells = p.completion_cells(&pl).expect("completions");
    assert_eq!(cells_text(&cells), " link-window list-buffers");

    // Clipped to the area.
    let pl = p.layout(0, 10, 2, Align::Left);
    assert_eq!(cells_text(&p.completion_cells(&pl).expect("completions")), " link-");

    // Not shown when the cursor is not at the end.
    press(&mut p, &mut env, &[b'\t' as key_code, LEFT]);
    assert!(p.completion_cells(&p.layout(0, 40, 2, Align::Left)).is_none());
}

#[test]
fn flags_string_names_flags_and_modes() {
    let p = prompt(
        prompt_flags::PROMPT_SINGLE | prompt_flags::PROMPT_ISPANE,
        modekey::MODEKEY_EMACS,
        "",
    );
    assert_eq!(p.flags_string(), "SINGLE,ISPANE");

    let mut env = Env::default();
    let mut p = vi("x");
    assert_eq!(p.flags_string(), "");
    press(&mut p, &mut env, &[ESCAPE]);
    assert_eq!(p.flags_string(), "COMMANDMODE");
    assert_eq!(p.cursor(), (0, -1));
}

#[test]
fn mouse_moves_the_cursor() {
    let mut p = emacs("abcdef");
    let pl = p.layout(0, 40, 3, Align::Left);
    // Between b and c.
    let out = p.mouse(5, &pl);
    assert_eq!((out.result, out.redraw, p.index()), (KeyResult::Handled, true, 2));
    // On the message: the start of the input.
    let out = p.mouse(1, &p.layout(0, 40, 3, Align::Left));
    assert_eq!((out.result, p.index()), (KeyResult::Handled, 0));
    // After the input: its end.
    p.mouse(30, &p.layout(0, 40, 3, Align::Left));
    assert_eq!(p.index(), 6);
    // Outside the area.
    let out = p.mouse(40, &p.layout(0, 40, 3, Align::Left));
    assert_eq!(out.result, KeyResult::NotHandled);

    // An empty input takes the click and nothing changes.
    let mut p = emacs("");
    let out = p.mouse(5, &p.layout(0, 40, 3, Align::Left));
    assert_eq!((out.result, out.redraw), (KeyResult::Handled, false));
}

#[test]
fn mouse_picks_a_completion() {
    let mut env = Env {
        commands: vec!["list-buffers", "link-window"],
        ..Env::default()
    };
    let mut p = emacs("li");
    press(&mut p, &mut env, &[b'\t' as key_code]);
    // " link-window list-buffers" from column 4: list-buffers is 17 to 28.
    let pl = p.layout(0, 40, 2, Align::Left);
    let out = p.mouse(19, &pl);
    assert_eq!((out.result, out.redraw), (KeyResult::Handled, true));
    assert_eq!(input(&p), "list-buffers ");
    assert!(p.completion_display().is_none());

    // The space between two completions is taken but picks nothing.
    let mut p = emacs("li");
    press(&mut p, &mut env, &[b'\t' as key_code]);
    let out = p.mouse(16, &p.layout(0, 40, 2, Align::Left));
    assert_eq!((out.result, input(&p).as_str()), (KeyResult::Handled, "li"));
}
