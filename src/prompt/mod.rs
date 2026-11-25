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
//! with the input already expanded and the options it uses read into a
//! [`PromptLook`], feeds it keys with [`Prompt::key`] and draws it: the
//! owner expands the message through message-format, then
//! [`Prompt::layout`] says where the parts go and [`Prompt::input_cells`]
//! and [`Prompt::completion_cells`] what to write there. A key never calls the
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
    GRID_DEFAULT_CELL, GridCell, KEYC_CTRL, KEYC_IS_UNICODE, KEYC_MASK_FLAGS, KEYC_MASK_KEY, KEYC_MASK_MODIFIERS, KEYC_META,
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

/// The options a prompt draws with, read when it opens (3.8's
/// `prompt_set_options`). Changing them later does not change an open
/// prompt.
#[derive(Clone)]
pub struct PromptLook {
    /// message-format, which shows the message as #{message}.
    pub message_format: Vec<u8>,
    /// message-style applied, used when the expanded style string does
    /// not parse.
    pub style: GridCell,
    /// message-style as a string, expanded with the prompt's formats
    /// when drawn.
    pub style_str: Vec<u8>,
    /// message-command-style applied, for vi command mode.
    pub command_style: GridCell,
    /// message-command-style as a string.
    pub command_style_str: Vec<u8>,
    /// prompt-cursor-style, as the choice number.
    pub cursor_style: u32,
    /// prompt-command-cursor-style.
    pub command_cursor_style: u32,
    /// prompt-cursor-colour.
    pub cursor_colour: i32,
    /// prompt-command-cursor-colour.
    pub command_cursor_colour: i32,
}

impl Default for PromptLook {
    fn default() -> Self {
        Self {
            message_format: b"#{message}".to_vec(),
            style: GRID_DEFAULT_CELL,
            style_str: Vec::new(),
            command_style: GRID_DEFAULT_CELL,
            command_style_str: Vec::new(),
            cursor_style: 0,
            command_cursor_style: 0,
            cursor_colour: -1,
            command_cursor_colour: -1,
        }
    }
}

/// How the prompt's content is aligned in its area, from the align
/// directive of the message style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
}

/// Where the parts of a prompt go in its area (3.8's `prompt_layout`). All
/// columns are absolute, like the area's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub area_x: u32,
    pub area_width: u32,
    /// Width of the expanded message-format.
    pub label_width: u32,
    /// Where the label and input start, after alignment.
    pub content_x: u32,
    /// Width of the label, the visible input and the completions.
    pub content_width: u32,
    /// Where the input starts.
    pub input_x: u32,
    /// How many columns of the input are scrolled off to the left.
    pub input_offset: u32,
    /// How many columns of the input are visible.
    pub input_width: u32,
    /// The cursor.
    pub cursor_x: u32,
}

/// How a prompt starts.
pub struct PromptSetup {
    /// The message shown before the input, not expanded: it is expanded
    /// when the prompt is drawn, so it can show the input.
    pub message: Vec<u8>,
    /// The initial input, expanded.
    pub input: Vec<u8>,
    pub ty: PromptType,
    pub flags: prompt_flags,
    /// The status-keys option.
    pub keys: modekey,
    /// The word-separators option.
    pub word_separators: Vec<u8>,
    pub look: PromptLook,
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
    look: PromptLook,
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

/// The input cells being collected for drawing, see
/// [`Prompt::input_cells`].
struct Cells {
    /// Columns of the input scrolled off to the left.
    offset: u32,
    /// Columns of the input that fit.
    width_limit: u32,
    /// Columns of the input passed so far, scrolled off or not.
    width: u32,
    /// Columns written, where the screen's cursor would be.
    drawn: u32,
    out: Vec<Utf8Data>,
}

impl Cells {
    /// Adds `ud` if it is in the visible part (3.8's
    /// `prompt_redraw_character`). Returns false when the area is full.
    fn push(&mut self, ud: &Utf8Data) -> bool {
        if self.width < self.offset {
            self.width += u32::from(ud.width);
            return true;
        }
        if self.width >= self.offset + self.width_limit {
            return false;
        }
        self.width += u32::from(ud.width);
        if self.width > self.offset + self.width_limit {
            return false;
        }
        let ch = ud.data[0];
        let cell = if ud.size == 1 && (ch <= 0x1f || ch == 0x7f) {
            let mut cell = Utf8Data::single(b'^');
            cell.data[1] = if ch == 0x7f { b'?' } else { ch | 0x40 };
            cell.size = 2;
            cell.have = 2;
            cell.width = 2;
            cell
        } else {
            *ud
        };
        self.drawn += u32::from(cell.width);
        self.out.push(cell);
        true
    }
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
            look: setup.look,
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

    /// The message shown before the input, not expanded.
    pub fn message(&self) -> &[u8] {
        &self.message
    }

    /// The options the prompt draws with.
    pub fn look(&self) -> &PromptLook {
        &self.look
    }

    /// The style to draw with, as the applied cell and the string to
    /// expand: message-command-style in vi command mode, otherwise
    /// message-style.
    pub fn style(&self) -> (&GridCell, &[u8]) {
        if self.command_mode {
            (&self.look.command_style, &self.look.command_style_str)
        } else {
            (&self.look.style, &self.look.style_str)
        }
    }

    /// The cursor style choice and colour to show: the command variants
    /// in vi command mode.
    pub fn cursor(&self) -> (u32, i32) {
        if self.command_mode {
            (self.look.command_cursor_style, self.look.command_cursor_colour)
        } else {
            (self.look.cursor_style, self.look.cursor_colour)
        }
    }

    /// The prompt's type.
    pub fn prompt_type(&self) -> PromptType {
        self.ty
    }

    /// The prompt's flags as `#{prompt_flags}` shows them: names separated
    /// by commas, with QUOTENEXT and COMMANDMODE while they apply.
    pub fn flags_string(&self) -> String {
        let f = |flag: prompt_flags| self.flags.intersects(flag);
        let names = [
            (f(prompt_flags::PROMPT_SINGLE), "SINGLE"),
            (f(prompt_flags::PROMPT_NUMERIC), "NUMERIC"),
            (f(prompt_flags::PROMPT_INCREMENTAL), "INCREMENTAL"),
            (f(prompt_flags::PROMPT_NOFORMAT), "NOFORMAT"),
            (f(prompt_flags::PROMPT_KEY), "KEY"),
            (f(prompt_flags::PROMPT_ACCEPT), "ACCEPT"),
            (self.quote_next, "QUOTENEXT"),
            (f(prompt_flags::PROMPT_BSPACE_EXIT), "BSPACE_EXIT"),
            (f(prompt_flags::PROMPT_NOFREEZE), "NOFREEZE"),
            (self.command_mode, "COMMANDMODE"),
            (f(prompt_flags::PROMPT_ISPANE), "ISPANE"),
            (f(prompt_flags::PROMPT_ISMODE), "ISMODE"),
            (f(prompt_flags::PROMPT_EDITARROWS), "EDITARROWS"),
        ];
        let set: Vec<&str> = names.iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect();
        set.join(",")
    }

    /// The width of the input and of the input before the cursor. The
    /// ^ shown while C-v waits counts in the width.
    fn widths(&self) -> (u32, u32) {
        let pcursor = utf8_width(&self.buffer[..self.index]);
        let mut pwidth = utf8_width(&self.buffer);
        if self.quote_next {
            pwidth += 1;
        }
        (pcursor, pwidth)
    }

    /// Works out where the parts go in an area of `area_width` columns
    /// from `area_x`, when the expanded message-format is `label_width`
    /// wide. The input scrolls so the cursor stays visible.
    pub fn layout(&self, area_x: u32, area_width: u32, label_width: u32, align: Align) -> Layout {
        let mut pl = Layout {
            area_x,
            area_width,
            ..Layout::default()
        };
        if area_width == 0 {
            return pl;
        }
        pl.label_width = label_width.min(area_width);

        let (pcursor, pwidth) = self.widths();
        let avail = area_width - pl.label_width;
        if avail == 0 {
            pl.cursor_x = pl.label_width;
        } else {
            let (offset, width) = if pcursor >= avail {
                ((pcursor - avail) + 1, avail)
            } else {
                (0, pwidth)
            };
            pl.input_offset = offset;
            pl.input_width = width.min(avail);
            pl.cursor_x = pl.label_width + pcursor - offset;
        }

        pl.content_width = pl.label_width + pl.input_width;
        if let Some(display) = self.completion_display()
            && pl.cursor_x < area_width
        {
            let width = utf8_width(&utf8_decode(display.as_bytes())).min(area_width - pl.cursor_x);
            pl.content_width = pl.content_width.max(pl.cursor_x + width);
        }
        pl.content_width = pl.content_width.min(area_width);

        pl.content_x = match align {
            Align::Centre => area_x + (area_width - pl.content_width) / 2,
            Align::Right => area_x + area_width - pl.content_width,
            Align::Left => area_x,
        };
        pl.input_x = pl.content_x + pl.label_width;
        pl.cursor_x += pl.content_x;
        pl
    }

    /// The cells to write from `layout.input_x`: the visible part of the
    /// input, control characters as ^X (one cell two columns wide), and a
    /// ^ at the cursor while C-v waits for a key. Like 3.8, the scroll
    /// counts a control character as one column.
    pub fn input_cells(&self, layout: &Layout) -> Vec<Utf8Data> {
        let mut cells = Cells {
            offset: layout.input_offset,
            width_limit: layout.input_width,
            width: 0,
            drawn: 0,
            out: Vec::new(),
        };
        if layout.content_width == 0 {
            return cells.out;
        }
        let (pcursor, _) = self.widths();
        let quote = Utf8Data::single(b'^');
        // 3.8's prompt_redraw_quote: the ^ goes where the cursor is.
        let quote_here = |cells: &mut Cells| {
            if self.quote_next && pcursor >= cells.offset && cells.drawn == pcursor - cells.offset {
                return cells.push(&quote);
            }
            true
        };
        for ud in &self.buffer {
            if !quote_here(&mut cells) || !cells.push(ud) {
                return cells.out;
            }
        }
        quote_here(&mut cells);
        cells.out
    }

    /// The completions to write from `layout.cursor_x`, underlined, if
    /// Tab found several and the cursor is at the end of the input.
    pub fn completion_cells(&self, layout: &Layout) -> Option<Vec<Utf8Data>> {
        let display = self.completion_display()?;
        let (ax, aw, cx) = (layout.content_x, layout.content_width, layout.cursor_x);
        if cx < ax || cx - ax >= aw {
            return None;
        }
        let avail = aw - (cx - ax);
        let mut width = 0;
        let mut out = Vec::new();
        for ud in utf8_decode(display.as_bytes()) {
            if width + u32::from(ud.width) > avail {
                break;
            }
            width += u32::from(ud.width);
            out.push(ud);
        }
        Some(out)
    }

    /// A left click at column `x` (3.8's `prompt_mouse`), with the prompt
    /// laid out as `pl`: a click on a completion picks it, a click in the
    /// input moves the cursor there, a click on the message moves it to
    /// the start of what is shown. Clicks outside the area are not
    /// handled.
    pub fn mouse(&mut self, x: u32, pl: &Layout) -> KeyOutcome {
        if x < pl.area_x || x >= pl.area_x + pl.area_width {
            return KeyOutcome::new(KeyResult::NotHandled);
        }
        let mut out = KeyOutcome::new(KeyResult::Handled);
        if pl.input_width == 0 {
            return out;
        }
        if let Some(out) = self.mouse_complete(x, pl) {
            return out;
        }

        let (_, pwidth) = self.widths();
        let target = if x <= pl.input_x {
            pl.input_offset
        } else {
            pl.input_offset + x - pl.input_x
        }
        .min(pwidth);
        let mut width = 0;
        let mut idx = 0;
        while idx < self.buffer.len() && width < target {
            width += u32::from(self.buffer[idx].width);
            idx += 1;
        }
        if idx != self.index {
            self.index = idx;
            self.complete.clear();
            out.redraw = true;
        }
        out
    }

    /// A click on the completions shown after the input (3.8's
    /// `prompt_mouse_complete`). None if the click is not on them.
    fn mouse_complete(&mut self, x: u32, pl: &Layout) -> Option<KeyOutcome> {
        let display = self.completion_display()?;
        let (cx, ax, aw) = (pl.cursor_x, pl.content_x, pl.content_width);
        if cx < ax || cx - ax >= aw || x < cx {
            return None;
        }
        let avail = aw - (cx - ax);
        let clicked = x - cx;
        let width = utf8_width(&utf8_decode(display.as_bytes())).min(avail);
        if clicked >= width {
            return None;
        }

        let mut out = KeyOutcome::new(KeyResult::Handled);
        let mut end = 0;
        let mut picked = None;
        for item in &self.complete {
            let start = end + 1;
            end = start + utf8_width(&utf8_decode(item.as_bytes()));
            if (start..end).contains(&clicked) {
                picked = Some(format!("{item} "));
                break;
            }
        }
        if let Some(replace) = picked
            && self.replace_complete(Some(&replace), None)
        {
            self.complete.clear();
            out.redraw = true;
        }
        Some(out)
    }

    /// The input, one entry per character.
    #[cfg(test)]
    pub fn buffer(&self) -> &[Utf8Data] {
        &self.buffer
    }

    /// The input as bytes.
    pub fn input(&self) -> Vec<u8> {
        utf8_encode(&self.buffer)
    }

    /// The cursor, as an index into [`Prompt::buffer`].
    #[cfg(test)]
    pub fn index(&self) -> usize {
        self.index
    }

    /// Whether vi command mode is on (the prompt uses
    /// message-command-style).
    pub fn command_mode(&self) -> bool {
        self.command_mode
    }

    /// Whether C-v is waiting for the key to insert.
    #[cfg(test)]
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
            k if k == b'\t' as key_code => self.replace_complete(None, Some(env)),
            k if (k == BSPACE || k == ctrl(b'h'))
                && size == 0
                && self.flags.intersects(prompt_flags::PROMPT_BSPACE_EXIT) =>
            {
                // command-prompt -e: Backspace on an empty prompt cancels it.
                let mut out = KeyOutcome::new(KeyResult::Handled);
                out.fires.push(Self::done(None));
                return out;
            }
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
    /// Completing reads the command names from `env`.
    fn replace_complete(&mut self, with: Option<&str>, env: Option<&mut dyn PromptEnv>) -> bool {
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
                let Some(env) = env else {
                    return false;
                };
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
