// Copyright (c) 2007 Nicholas Marriott <nicholas.marriott@gmail.com>
//
// Permission to use, copy, modify, and distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
// IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
// OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
//! A prompt being edited (prompt.c): its text, cursor, history position,
//! completion and the editing keys of tmux 3.8.
//!
//! The prompt knows nothing about clients or panes. Its owner creates it
//! with the message and input already expanded, feeds it keys with
//! [`Prompt::key`] and draws it from its accessors. A key never calls the
//! owner's callback. It returns a [`KeyOutcome`] listing the callbacks to
//! fire ([`Fire`]), and the owner fires them in order and reports back
//! with [`Prompt::fired`]. What the prompt needs from the rest of tmux
//! (history, the newest paste buffer, command names) comes through
//! [`PromptEnv`], and what it adds to the history is returned.

#![forbid(unsafe_code)]

use std::rc::Rc;

use crate::key_string::key_string_lookup_key;
use crate::prompt_history::{HistoryPositions, PromptType};
use crate::utf8::{Utf8Data, Utf8State};
use crate::{
    KEYC_CTRL, KEYC_IS_UNICODE, KEYC_MASK_FLAGS, KEYC_MASK_KEY, KEYC_MASK_MODIFIERS, KEYC_META,
    KEYC_VI, key_code, keyc, modekey, prompt_flags,
};

/// What a key did, as the owner sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyResult {
    /// The prompt did not use the key, so it goes to the key bindings.
    NotHandled,
    /// The prompt used the key.
    Handled,
    /// The prompt is finished.
    Close,
    /// A cursor key in an incremental prompt: the prompt closes and the
    /// key goes on to the key bindings (3.8).
    Move,
}

/// What a callback says after it is fired.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallbackResult {
    /// Keep the prompt open.
    Continue,
    /// Close the prompt.
    Close,
}

/// A callback the owner fires for a key: the text (`None` when the
/// prompt was cancelled) and the kind of key.
#[derive(Debug, PartialEq, Eq)]
pub struct Fire {
    pub text: Option<Vec<u8>>,
    pub kind: KeyResult,
    after: AfterFire,
}

/// How the key's result depends on what the callback said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AfterFire {
    /// The prompt is done (Enter, Escape, a single key): Close if the
    /// callback closed it, otherwise Handled.
    Done,
    /// The prompt closes whatever the callback says, with this result.
    Closes(KeyResult),
    /// A cursor key in an incremental prompt: Close if the callback
    /// closed it, otherwise Move.
    Move,
    /// An incremental update, the result is already decided.
    Update,
}

/// What a key did.
#[derive(Debug)]
pub struct KeyOutcome {
    /// The result, before any callbacks are fired.
    pub result: KeyResult,
    /// Whether the prompt has to be drawn again.
    pub redraw: bool,
    /// Callbacks to fire, in order.
    pub fires: Vec<Fire>,
    /// A line to add to the history of the prompt's type.
    pub history: Option<(PromptType, Vec<u8>)>,
}

impl KeyOutcome {
    fn new(result: KeyResult) -> Self {
        Self {
            result,
            redraw: false,
            fires: Vec::new(),
            history: None,
        }
    }
}

/// What the prompt reads from the rest of tmux while handling a key.
pub trait PromptEnv {
    /// The history entry before the one at `positions`, see
    /// [`crate::prompt_history::PromptHistory::up`].
    fn history_up(&mut self, ty: PromptType, positions: &mut HistoryPositions) -> Option<Vec<u8>>;
    /// The history entry after the one at `positions`.
    fn history_down(&mut self, ty: PromptType, positions: &mut HistoryPositions) -> Vec<u8>;
    /// The newest paste buffer's data, if there is one.
    fn paste_top(&mut self) -> Option<Rc<[u8]>>;
    /// Command names and command-alias names that start with `prefix`,
    /// each once.
    fn complete_commands(&mut self, prefix: &[u8]) -> Vec<String>;
}

/// How a prompt starts.
pub struct PromptSetup {
    /// The message shown before the input, expanded.
    pub message: Vec<u8>,
    /// The initial input, expanded.
    pub input: Vec<u8>,
    pub ty: PromptType,
    pub flags: prompt_flags,
    /// The status-keys option.
    pub keys: modekey,
    /// The word-separators option.
    pub word_separators: Vec<u8>,
}

/// A prompt.
pub struct Prompt {
    message: Vec<u8>,
    buffer: Vec<Utf8Data>,
    index: usize,
    /// The input an incremental prompt started with, which C-r and C-s
    /// bring back.
    last: Option<Vec<u8>>,
    /// Text deleted with C-w, for C-y.
    copied: Option<Vec<Utf8Data>>,
    hindex: HistoryPositions,
    flags: prompt_flags,
    ty: PromptType,
    keys: modekey,
    word_separators: Vec<u8>,
    /// C-v was pressed: the next key is inserted as it is.
    quote_next: bool,
    /// In vi command mode.
    command_mode: bool,
    /// Completions shown after the input, when Tab found several.
    complete: Vec<String>,
    closed: bool,
}

/// Decodes `s` into characters as `utf8_fromcstr` does: bytes that are not
/// part of a valid sequence become characters of their own.
pub fn utf8_decode(s: &[u8]) -> Vec<Utf8Data> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        let mut ud = Utf8Data::empty();
        let mut more = ud.open(s[i]);
        if more == Utf8State::More {
            let mut j = i + 1;
            while j < s.len() && more == Utf8State::More {
                more = ud.append(s[j]);
                j += 1;
            }
            if more == Utf8State::Done {
                out.push(ud);
                i = j;
                continue;
            }
        }
        out.push(Utf8Data::single(s[i]));
        i += 1;
    }
    out
}

/// The bytes of `buffer`.
pub fn utf8_encode(buffer: &[Utf8Data]) -> Vec<u8> {
    buffer
        .iter()
        .flat_map(|ud| ud.data[..ud.size as usize].iter().copied())
        .collect()
}

/// The display width of `buffer`.
pub fn utf8_width(buffer: &[Utf8Data]) -> u32 {
    buffer.iter().map(|ud| u32::from(ud.width)).sum()
}

/// Whether `ud` is one of the `separators`.
fn in_list(separators: &[u8], ud: Option<&Utf8Data>) -> bool {
    ud.is_some_and(|ud| ud.size == 1 && ud.width == 1 && separators.contains(&ud.data[0]))
}

/// Whether `ud` is a space.
fn is_space(ud: Option<&Utf8Data>) -> bool {
    ud.is_some_and(|ud| ud.size == 1 && ud.width == 1 && ud.data[0] == b' ')
}

/// The text a keypad key inserts, or the key itself.
fn keypad_key(key: key_code) -> key_code {
    if key & KEYC_MASK_MODIFIERS != 0 {
        return key;
    }
    let text = match key {
        k if k == keyc::KEYC_KP_SLASH as key_code => b'/',
        k if k == keyc::KEYC_KP_STAR as key_code => b'*',
        k if k == keyc::KEYC_KP_MINUS as key_code => b'-',
        k if k == keyc::KEYC_KP_SEVEN as key_code => b'7',
        k if k == keyc::KEYC_KP_EIGHT as key_code => b'8',
        k if k == keyc::KEYC_KP_NINE as key_code => b'9',
        k if k == keyc::KEYC_KP_PLUS as key_code => b'+',
        k if k == keyc::KEYC_KP_FOUR as key_code => b'4',
        k if k == keyc::KEYC_KP_FIVE as key_code => b'5',
        k if k == keyc::KEYC_KP_SIX as key_code => b'6',
        k if k == keyc::KEYC_KP_ONE as key_code => b'1',
        k if k == keyc::KEYC_KP_TWO as key_code => b'2',
        k if k == keyc::KEYC_KP_THREE as key_code => b'3',
        k if k == keyc::KEYC_KP_ENTER as key_code => b'\r',
        k if k == keyc::KEYC_KP_ZERO as key_code => b'0',
        k if k == keyc::KEYC_KP_PERIOD as key_code => b'.',
        _ => return key,
    };
    key_code::from(text)
}

/// A key with Ctrl.
const fn ctrl(c: u8) -> key_code {
    c as key_code | KEYC_CTRL
}

/// A key with Meta.
const fn meta(c: u8) -> key_code {
    c as key_code | KEYC_META
}

/// The vi translation of a key.
enum Translated {
    /// Drop the key.
    Drop,
    /// Handle this key as an emacs key.
    Process(key_code),
    /// Insert this key.
    Append(key_code),
}

const UP: key_code = keyc::KEYC_UP as key_code;
const DOWN: key_code = keyc::KEYC_DOWN as key_code;
const LEFT: key_code = keyc::KEYC_LEFT as key_code;
const RIGHT: key_code = keyc::KEYC_RIGHT as key_code;
const HOME: key_code = keyc::KEYC_HOME as key_code;
const END: key_code = keyc::KEYC_END as key_code;
const BSPACE: key_code = keyc::KEYC_BSPACE as key_code;
const DC: key_code = keyc::KEYC_DC as key_code;
const PPAGE: key_code = keyc::KEYC_PPAGE as key_code;
const NPAGE: key_code = keyc::KEYC_NPAGE as key_code;
const ESCAPE: key_code = 0x1b;

impl Prompt {
    pub fn new(setup: PromptSetup) -> Self {
        let incremental = setup.flags.intersects(prompt_flags::PROMPT_INCREMENTAL);
        let (buffer, last) = if incremental {
            (Vec::new(), Some(setup.input))
        } else {
            (utf8_decode(&setup.input), None)
        };
        Self {
            message: setup.message,
            index: buffer.len(),
            buffer,
            last,
            copied: None,
            hindex: HistoryPositions::default(),
            flags: setup.flags,
            ty: setup.ty,
            keys: setup.keys,
            word_separators: setup.word_separators,
            quote_next: false,
            command_mode: false,
            complete: Vec::new(),
            closed: false,
        }
    }

    /// The callback an incremental prompt fires when it opens, with "="
    /// and the input.
    pub fn incremental_start(&self) -> Option<Fire> {
        self.flags
            .intersects(prompt_flags::PROMPT_INCREMENTAL)
            .then(|| self.update_fire(b'='))
    }

    /// Replaces the message and input (the next of several prompts).
    pub fn update(&mut self, message: Vec<u8>, input: &[u8]) {
        self.message = message;
        self.buffer = utf8_decode(input);
        self.index = self.buffer.len();
        self.hindex = HistoryPositions::default();
        self.closed = false;
        self.complete.clear();
    }

    /// The message shown before the input.
    pub fn message(&self) -> &[u8] {
        &self.message
    }

    /// The input, one entry per character.
    pub fn buffer(&self) -> &[Utf8Data] {
        &self.buffer
    }

    /// The input as bytes.
    pub fn input(&self) -> Vec<u8> {
        utf8_encode(&self.buffer)
    }

    /// The cursor, as an index into [`Prompt::buffer`].
    pub fn index(&self) -> usize {
        self.index
    }

    /// Whether vi command mode is on (the prompt uses
    /// message-command-style).
    pub fn command_mode(&self) -> bool {
        self.command_mode
    }

    /// Whether C-v is waiting for the key to insert.
    pub fn quote_next(&self) -> bool {
        self.quote_next
    }

    /// Whether the prompt is finished and its owner should remove it.
    pub fn closed(&self) -> bool {
        self.closed
    }

    /// The completions to show after the input, a space before each, if
    /// Tab found several and the cursor is at the end.
    pub fn completion_display(&self) -> Option<String> {
        if self.complete.is_empty() || self.index != self.buffer.len() {
            return None;
        }
        Some(self.complete.iter().map(|c| format!(" {c}")).collect())
    }

    /// Reports what the callback of `fire` said and returns the key's
    /// result. Call it for each fire in order, with the result so far.
    pub fn fired(&mut self, fire: &Fire, said: CallbackResult, result: KeyResult) -> KeyResult {
        let closed = said == CallbackResult::Close;
        if closed {
            self.closed = true;
        }
        match fire.after {
            AfterFire::Done if closed => KeyResult::Close,
            AfterFire::Done => KeyResult::Handled,
            AfterFire::Closes(r) => {
                self.closed = true;
                r
            }
            AfterFire::Move if closed => KeyResult::Close,
            AfterFire::Move => KeyResult::Move,
            AfterFire::Update => result,
        }
    }

    /// The fire of an incremental update: the prefix ('=' for the whole
    /// text, '+' or '-' to search on) and the input.
    fn update_fire(&self, prefix: u8) -> Fire {
        let mut text = vec![prefix];
        text.extend(self.input());
        Fire {
            text: Some(text),
            kind: KeyResult::Handled,
            after: AfterFire::Update,
        }
    }

    /// The fire when the prompt is done with `text`.
    fn done(text: Option<Vec<u8>>) -> Fire {
        Fire {
            text,
            kind: KeyResult::Close,
            after: AfterFire::Done,
        }
    }

    /// Translates a vi key. Switching between insert and command mode
    /// asks for a redraw.
    fn translate_vi(&mut self, key: key_code, redraw: &mut bool) -> Translated {
        if !self.command_mode {
            return match key {
                k if [
                    ctrl(b'a'),
                    ctrl(b'c'),
                    ctrl(b'e'),
                    ctrl(b'g'),
                    ctrl(b'h'),
                    b'\t' as key_code,
                    ctrl(b'k'),
                    ctrl(b'n'),
                    ctrl(b'p'),
                    ctrl(b't'),
                    ctrl(b'u'),
                    ctrl(b'v'),
                    ctrl(b'w'),
                    ctrl(b'y'),
                    b'\n' as key_code,
                    b'\r' as key_code,
                    LEFT | KEYC_CTRL,
                    RIGHT | KEYC_CTRL,
                    BSPACE,
                    DC,
                    DOWN,
                    END,
                    HOME,
                    LEFT,
                    RIGHT,
                    UP,
                ]
                .contains(&k) =>
                {
                    Translated::Process(k)
                }
                k if k == ESCAPE || k == ctrl(b'[') => {
                    self.command_mode = true;
                    self.index = self.index.saturating_sub(1);
                    *redraw = true;
                    Translated::Drop
                }
                k => Translated::Append(k),
            };
        }

        match key {
            BSPACE => return Translated::Process(LEFT),
            k if k == ESCAPE || k == ctrl(b'[') => return Translated::Drop,
            k if k == b'i' as key_code => {
                self.command_mode = false;
                *redraw = true;
                return Translated::Drop;
            }
            k if k == b'S' as key_code => {
                self.command_mode = false;
                *redraw = true;
                return Translated::Process(ctrl(b'u'));
            }
            k if b"AICsa".map(key_code::from).contains(&k) => {
                self.command_mode = false;
                *redraw = true;
            }
            _ => {}
        }

        let process = match key {
            k if k == b'A' as key_code || k == b'$' as key_code => END,
            k if k == b'I' as key_code || k == b'0' as key_code || k == b'^' as key_code => HOME,
            k if k == b'C' as key_code || k == b'D' as key_code => ctrl(b'k'),
            k if k == b'X' as key_code => BSPACE,
            k if k == b'b' as key_code => meta(b'b'),
            k if k == b'B' as key_code => b'B' as key_code | KEYC_VI,
            k if k == b'd' as key_code => ctrl(b'u'),
            k if k == b'e' as key_code => b'e' as key_code | KEYC_VI,
            k if k == b'E' as key_code => b'E' as key_code | KEYC_VI,
            k if k == b'w' as key_code => b'w' as key_code | KEYC_VI,
            k if k == b'W' as key_code => b'W' as key_code | KEYC_VI,
            k if k == b'p' as key_code => ctrl(b'y'),
            k if k == b'q' as key_code => ctrl(b'c'),
            k if k == b's' as key_code || k == DC || k == b'x' as key_code => DC,
            k if k == DOWN || k == b'j' as key_code => DOWN,
            k if k == LEFT || k == b'h' as key_code => LEFT,
            k if k == b'a' as key_code || k == RIGHT || k == b'l' as key_code => RIGHT,
            k if k == UP || k == b'k' as key_code => UP,
            k if k == ctrl(b'h') || k == ctrl(b'c') || k == b'\n' as key_code || k == b'\r' as key_code => k,
            _ => return Translated::Drop,
        };
        Translated::Process(process)
    }

    /// A cursor key in an incremental prompt fires a move.
    fn check_move(&self, key: key_code) -> Option<Fire> {
        if !self.flags.intersects(prompt_flags::PROMPT_INCREMENTAL) {
            return None;
        }
        if ![UP, DOWN, PPAGE, NPAGE, LEFT, RIGHT].contains(&key) {
            return None;
        }
        Some(Fire {
            text: Some(self.input()),
            kind: KeyResult::Move,
            after: AfterFire::Move,
        })
    }

    /// Handles a key.
    pub fn key(&mut self, key: key_code, env: &mut dyn PromptEnv) -> KeyOutcome {
        self.closed = false;
        // Tab builds the completions again if it still applies.
        self.complete.clear();

        if self.flags.intersects(prompt_flags::PROMPT_KEY) {
            let mut out = KeyOutcome::new(KeyResult::Close);
            out.fires.push(Fire {
                text: Some(key_string_lookup_key(key, 0).into_bytes()),
                kind: KeyResult::Close,
                after: AfterFire::Closes(KeyResult::Close),
            });
            return out;
        }

        let mut key = keypad_key(key & !KEYC_MASK_FLAGS);

        if self.flags.intersects(prompt_flags::PROMPT_NUMERIC) {
            if (key_code::from(b'0')..=key_code::from(b'9')).contains(&key) {
                return self.append(key, b'=');
            }
            let mut out = KeyOutcome::new(KeyResult::NotHandled);
            out.fires.push(Fire {
                text: Some(self.input()),
                kind: KeyResult::Close,
                after: AfterFire::Closes(KeyResult::NotHandled),
            });
            return out;
        }

        if self.flags.intersects(prompt_flags::PROMPT_SINGLE) || self.quote_next {
            if key & KEYC_MASK_KEY == BSPACE {
                key = 0x7f;
            } else if key & KEYC_MASK_KEY > 0x7f {
                if !KEYC_IS_UNICODE(key) {
                    return KeyOutcome::new(KeyResult::Handled);
                }
                key &= KEYC_MASK_KEY;
            } else if key & KEYC_CTRL != 0 {
                key &= 0x1f;
            } else {
                key &= KEYC_MASK_KEY;
            }
            self.quote_next = false;
            return self.append(key, b'=');
        }

        let mut redraw = false;
        if self.keys == modekey::MODEKEY_VI {
            match self.translate_vi(key, &mut redraw) {
                Translated::Process(k) => key = k,
                Translated::Append(k) => return self.append(k, b'='),
                Translated::Drop => {
                    let mut out = KeyOutcome::new(KeyResult::Handled);
                    out.redraw = redraw;
                    return out;
                }
            }
        }

        if let Some(fire) = self.check_move(key) {
            let mut out = KeyOutcome::new(KeyResult::Move);
            out.fires.push(fire);
            return out;
        }

        let size = self.buffer.len();
        let mut prefix = b'=';
        let changed = match key {
            k if k == LEFT || k == ctrl(b'b') => {
                self.index = self.index.saturating_sub(1);
                false
            }
            k if k == RIGHT || k == ctrl(b'f') => {
                if self.index < size {
                    self.index += 1;
                }
                false
            }
            k if k == HOME || k == ctrl(b'a') => {
                self.index = 0;
                false
            }
            k if k == END || k == ctrl(b'e') => {
                self.index = size;
                false
            }
            k if k == b'\t' as key_code => self.replace_complete(None, env),
            k if k == BSPACE || k == ctrl(b'h') => {
                if self.index != 0 {
                    self.index -= 1;
                    self.buffer.remove(self.index);
                    true
                } else {
                    false
                }
            }
            k if k == DC || k == ctrl(b'd') => {
                if self.index != size {
                    self.buffer.remove(self.index);
                    true
                } else {
                    false
                }
            }
            k if k == ctrl(b'u') => {
                self.buffer.clear();
                self.index = 0;
                true
            }
            k if k == ctrl(b'k') => {
                if self.index < size {
                    self.buffer.truncate(self.index);
                    true
                } else {
                    false
                }
            }
            k if k == ctrl(b'w') => {
                self.delete_word_before();
                true
            }
            k if k == RIGHT | KEYC_CTRL || k == meta(b'f') => {
                let separators = self.word_separators.clone();
                self.forward_word(false, &separators);
                true
            }
            k if k == b'E' as key_code | KEYC_VI => {
                self.end_word(b"");
                true
            }
            k if k == b'e' as key_code | KEYC_VI => {
                let separators = self.word_separators.clone();
                self.end_word(&separators);
                true
            }
            k if k == b'W' as key_code | KEYC_VI => {
                self.forward_word(true, b"");
                true
            }
            k if k == b'w' as key_code | KEYC_VI => {
                let separators = self.word_separators.clone();
                self.forward_word(true, &separators);
                true
            }
            k if k == b'B' as key_code | KEYC_VI => {
                self.backward_word(b"");
                true
            }
            k if k == LEFT | KEYC_CTRL || k == meta(b'b') => {
                let separators = self.word_separators.clone();
                self.backward_word(&separators);
                true
            }
            k if k == UP || k == ctrl(b'p') => match env.history_up(self.ty, &mut self.hindex) {
                Some(line) => {
                    self.set_input(&line);
                    true
                }
                None => false,
            },
            k if k == DOWN || k == ctrl(b'n') => {
                let line = env.history_down(self.ty, &mut self.hindex);
                self.set_input(&line);
                true
            }
            k if k == ctrl(b'y') => self.paste(env),
            k if k == ctrl(b't') => {
                let idx = if self.index < size { self.index + 1 } else { self.index };
                if idx >= 2 {
                    self.buffer.swap(idx - 2, idx - 1);
                    self.index = idx;
                    true
                } else {
                    false
                }
            }
            k if k == b'\r' as key_code || k == b'\n' as key_code => {
                let text = self.input();
                let mut out = KeyOutcome::new(KeyResult::Handled);
                if !text.is_empty() {
                    out.history = Some((self.ty, text.clone()));
                }
                out.fires.push(Self::done(Some(text)));
                return out;
            }
            k if k == ESCAPE || k == ctrl(b'[') || k == ctrl(b'c') || k == ctrl(b'g') => {
                let mut out = KeyOutcome::new(KeyResult::Handled);
                out.fires.push(Self::done(None));
                return out;
            }
            k if (k == ctrl(b'r') || k == ctrl(b's'))
                && self.flags.intersects(prompt_flags::PROMPT_INCREMENTAL) =>
            {
                if self.buffer.is_empty() {
                    let last = self.last.clone().unwrap_or_default();
                    self.set_input(&last);
                } else {
                    prefix = if k == ctrl(b'r') { b'-' } else { b'+' };
                }
                true
            }
            k if k == ctrl(b'r') || k == ctrl(b's') => false,
            k if k == ctrl(b'v') => {
                self.quote_next = true;
                false
            }
            _ => return self.append(key, b'='),
        };

        let mut out = KeyOutcome::new(KeyResult::Handled);
        out.redraw = true;
        if changed {
            self.push_update(&mut out, prefix);
        }
        out
    }

    /// Fires the incremental update after a change.
    fn push_update(&self, out: &mut KeyOutcome, prefix: u8) {
        if self.flags.intersects(prompt_flags::PROMPT_INCREMENTAL) {
            out.fires.push(self.update_fire(prefix));
        }
    }

    /// Replaces the input and puts the cursor at its end.
    fn set_input(&mut self, input: &[u8]) {
        self.buffer = utf8_decode(input);
        self.index = self.buffer.len();
    }

    /// Inserts `key` at the cursor. A single key prompt is then done.
    fn append(&mut self, key: key_code, prefix: u8) -> KeyOutcome {
        let ud = if key <= 0x7f {
            let mut ud = Utf8Data::single(key as u8);
            if key <= 0x1f || key == 0x7f {
                // Drawn as ^X.
                ud.width = 2;
            }
            ud
        } else if KEYC_IS_UNICODE(key) {
            let ud = Utf8Data::from_char(key as u32);
            if ud.size == 0 {
                return KeyOutcome::new(KeyResult::Handled);
            }
            ud
        } else {
            return KeyOutcome::new(KeyResult::Handled);
        };
        self.buffer.insert(self.index, ud);
        self.index += 1;

        let mut out = KeyOutcome::new(KeyResult::Handled);
        if self.flags.intersects(prompt_flags::PROMPT_SINGLE) {
            if self.buffer.len() != 1 {
                self.closed = true;
                out.result = KeyResult::Close;
            } else {
                out.fires.push(Self::done(Some(self.input())));
            }
        }
        out.redraw = true;
        self.push_update(&mut out, prefix);
        out
    }

    /// C-w: deletes the word before the cursor and keeps it for C-y.
    fn delete_word_before(&mut self) {
        let mut idx = self.index;
        while idx != 0 {
            idx -= 1;
            if !is_space(self.buffer.get(idx)) {
                break;
            }
        }
        let word_is_separators = in_list(&self.word_separators, self.buffer.get(idx));
        while idx != 0 {
            idx -= 1;
            if is_space(self.buffer.get(idx))
                || word_is_separators != in_list(&self.word_separators, self.buffer.get(idx))
            {
                idx += 1;
                break;
            }
        }
        self.copied = Some(self.buffer.drain(idx..self.index).collect());
        self.index = idx;
    }

    /// Moves to the start of the next word. Emacs mode skips spaces
    /// first, vi mode after.
    fn forward_word(&mut self, vi: bool, separators: &[u8]) {
        let size = self.buffer.len();
        let mut idx = self.index;
        if !vi {
            while idx != size && is_space(self.buffer.get(idx)) {
                idx += 1;
            }
        }
        if idx == size {
            self.index = idx;
            return;
        }
        let word_is_separators =
            in_list(separators, self.buffer.get(idx)) && !is_space(self.buffer.get(idx));
        loop {
            idx += 1;
            if is_space(self.buffer.get(idx)) {
                if vi {
                    while idx != size && is_space(self.buffer.get(idx)) {
                        idx += 1;
                    }
                }
                break;
            }
            if idx == size || word_is_separators != in_list(separators, self.buffer.get(idx)) {
                break;
            }
        }
        self.index = idx;
    }

    /// Moves to the end of the next word.
    fn end_word(&mut self, separators: &[u8]) {
        let size = self.buffer.len();
        let mut idx = self.index;
        if idx == size {
            return;
        }
        loop {
            idx += 1;
            if idx == size {
                self.index = idx;
                return;
            }
            if !is_space(self.buffer.get(idx)) {
                break;
            }
        }
        let word_is_separators = in_list(separators, self.buffer.get(idx));
        loop {
            idx += 1;
            if idx == size
                || is_space(self.buffer.get(idx))
                || word_is_separators != in_list(separators, self.buffer.get(idx))
            {
                break;
            }
        }
        self.index = idx - 1;
    }

    /// Moves to the start of the previous word.
    fn backward_word(&mut self, separators: &[u8]) {
        let mut idx = self.index;
        while idx != 0 {
            idx -= 1;
            if !is_space(self.buffer.get(idx)) {
                break;
            }
        }
        let word_is_separators = in_list(separators, self.buffer.get(idx));
        while idx != 0 {
            idx -= 1;
            if is_space(self.buffer.get(idx))
                || word_is_separators != in_list(separators, self.buffer.get(idx))
            {
                idx += 1;
                break;
            }
        }
        self.index = idx;
    }

    /// C-y: inserts what C-w deleted, or else the newest paste buffer up to
    /// its first control character.
    fn paste(&mut self, env: &mut dyn PromptEnv) -> bool {
        let text = match &self.copied {
            Some(copied) => copied.clone(),
            None => {
                let Some(data) = env.paste_top() else {
                    return false;
                };
                let mut text = Vec::new();
                for ud in utf8_decode(&data) {
                    if ud.size == 1 && (ud.data[0] <= 31 || ud.data[0] >= 127) {
                        break;
                    }
                    text.push(ud);
                }
                text
            }
        };
        let n = text.len();
        self.buffer.splice(self.index..self.index, text);
        self.index += n;
        true
    }

    /// Tab: completes the word at the cursor, or replaces it with `with`.
    fn replace_complete(&mut self, with: Option<&str>, env: &mut dyn PromptEnv) -> bool {
        let size = self.buffer.len();
        let idx = self.index.saturating_sub(1);
        let space = |i: usize| is_space(self.buffer.get(i));

        let mut first = idx;
        while first > 0 && !space(first) {
            first -= 1;
        }
        while first < size && space(first) {
            first += 1;
        }
        let mut last = idx;
        while last < size && !space(last) {
            last += 1;
        }
        while last > 0 && space(last) {
            last -= 1;
        }
        if last < size {
            last += 1;
        }
        if last < first {
            return false;
        }

        let completed;
        let with = match with {
            Some(with) => with,
            None => {
                let word = utf8_encode(&self.buffer[first..last]);
                if word.len() >= 64 {
                    return false;
                }
                match self.complete_word(&word, first, env) {
                    Some(c) => {
                        completed = c;
                        &completed
                    }
                    None => return false,
                }
            }
        };

        let inserted: Vec<Utf8Data> = with.bytes().map(Utf8Data::single).collect();
        self.buffer.splice(first..last, inserted);
        self.index = first + with.len();
        true
    }

    /// Completes `word` at `offset`: command names only, at the start of
    /// the input. Returns what to insert, or keeps several matches to
    /// show.
    fn complete_word(&mut self, word: &[u8], offset: usize, env: &mut dyn PromptEnv) -> Option<String> {
        if self.ty != PromptType::Command || offset != 0 || word.is_empty() {
            return None;
        }
        let mut list = env.complete_commands(word);
        if list.is_empty() {
            return None;
        }
        list.sort_unstable();

        let out = if list.len() == 1 {
            Some(format!("{} ", list[0]))
        } else {
            let first = list[0].as_bytes();
            let len = list[1..].iter().fold(first.len(), |len, s| {
                len.min(first.iter().zip(s.as_bytes()).take_while(|(a, b)| a == b).count())
            });
            Some(list[0][..len].to_string())
        }
        .filter(|out| out.as_bytes() != word);

        if out.is_some() || list.len() <= 1 {
            return out;
        }
        self.complete = list;
        None
    }
}

#[cfg(test)]
mod tests;
