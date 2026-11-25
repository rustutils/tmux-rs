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
//! The adapter between the prompt object ([`crate::prompt`]) and the rest
//! of tmux, shared by every owner of a prompt (the client's status line,
//! a pane, a mode): reading the options a prompt draws with, expanding
//! its message through message-format and its style, drawing it with
//! `screen_write`, and what it reads while it handles a key. This is the part of 3.8's prompt.c that uses formats,
//! styles and screens.

use crate::options_::*;
use std::rc::Rc;

use crate::paste::paste_get_top;
use crate::prompt::{Align, CallbackResult, Fire, KeyResult, Layout, Prompt, PromptEnv, PromptLook, PromptSetup};
use crate::prompt_history::{HistoryPositions, PromptType, prompt_history_down, prompt_history_up};
use crate::*;

/// What a prompt reads from the rest of tmux while it handles a key, see
/// [`PromptEnv`]: the history and paste buffer through their bridges,
/// the command names from the table and the aliases read before the key.
pub struct PromptEnvAdapter {
    /// The command-alias names, read before the key when it is Tab.
    aliases: Vec<String>,
}

impl PromptEnv for PromptEnvAdapter {
    fn history_up(&mut self, ty: PromptType, positions: &mut HistoryPositions) -> Option<Vec<u8>> {
        prompt_history_up(ty, positions)
    }

    fn history_down(&mut self, ty: PromptType, positions: &mut HistoryPositions) -> Vec<u8> {
        prompt_history_down(ty, positions)
    }

    fn paste_top(&mut self) -> Option<Rc<[u8]>> {
        paste_get_top().map(|pb| pb.data().clone())
    }

    fn complete_commands(&mut self, prefix: &[u8]) -> Vec<String> {
        let mut list: Vec<String> = Vec::new();
        let names = CMD_TABLE.iter().map(|e| e.name).chain(self.aliases.iter().map(String::as_str));
        for name in names {
            if name.as_bytes().starts_with(prefix) && !list.iter().any(|l| l == name) {
                list.push(name.to_string());
            }
        }
        list
    }
}

/// The names of the command-alias option (the part before "=").
///
/// # Safety
/// The global options must exist.
unsafe fn prompt_aliases() -> Vec<String> {
    unsafe {
        let o = options_get_only(GLOBAL_OPTIONS, "command-alias");
        if o.is_null() {
            return Vec::new();
        }
        options_array_strings(o)
            .into_iter()
            .filter_map(|(_, value)| {
                let value = value.to_string_lossy();
                value.split_once('=').map(|(name, _)| name.to_string())
            })
            .collect()
    }
}

/// The env for handling `key`: the aliases are read only for Tab.
///
/// # Safety
/// The global options must exist.
pub unsafe fn prompt_env(key: key_code) -> PromptEnvAdapter {
    unsafe {
        let aliases = if key & KEYC_MASK_KEY == b'\t' as key_code {
            prompt_aliases()
        } else {
            Vec::new()
        };
        PromptEnvAdapter { aliases }
    }
}

/// Reads the options a prompt draws with from `s`, or the global session
/// options without a session (3.8's `prompt_set_options`).
///
/// # Safety
/// `s` must be null or a valid session, and the global options must
/// exist.
pub unsafe fn prompt_look(s: *mut session) -> PromptLook {
    unsafe {
        let oo = if s.is_null() { GLOBAL_S_OPTIONS } else { (*s).options };
        let bytes = |name: &str| CStr::from_ptr(options_get_string_(oo, name).cast()).to_bytes().to_vec();
        let mut style: GridCell = GRID_DEFAULT_CELL;
        style_apply(&raw mut style, oo, c!("message-style"), null_mut());
        let mut command_style: GridCell = GRID_DEFAULT_CELL;
        style_apply(&raw mut command_style, oo, c!("message-command-style"), null_mut());
        // 3.8 applies the colours as styles, so an empty or default
        // colour leaves the cursor colour alone (-1).
        let colour = |name: &str| {
            let value = String::from_utf8_lossy(&bytes(name)).into_owned();
            match colour_fromstring(&value) {
                8 => -1,
                c => c,
            }
        };
        let cursor_colour = colour("prompt-cursor-colour");
        let command_cursor_colour = colour("prompt-command-cursor-colour");
        PromptLook {
            message_format: bytes("message-format"),
            style,
            style_str: bytes("message-style"),
            command_style,
            command_style_str: bytes("message-command-style"),
            cursor_style: options_get_number_(oo, "prompt-cursor-style") as u32,
            command_cursor_style: options_get_number_(oo, "prompt-command-cursor-style") as u32,
            cursor_colour,
            command_cursor_colour,
        }
    }
}

/// How a prompt opened by `s` (or with the global options) starts: the
/// input expanded with `fs` unless `PROMPT_NOFORMAT`, the message kept to
/// be expanded when drawn, and the options read (3.8's `prompt_create`).
///
/// # Safety
/// `s` must be null or a valid session, `fs` null or a valid find state,
/// `msg` and `input` null or NUL terminated.
pub unsafe fn prompt_setup(
    s: *mut session,
    fs: *mut cmd_find_state,
    msg: *const u8,
    input: *const u8,
    ty: PromptType,
    flags: prompt_flags,
) -> PromptSetup {
    unsafe {
        let oo = if s.is_null() { GLOBAL_S_OPTIONS } else { (*s).options };
        let message = if msg.is_null() {
            Vec::new()
        } else {
            CStr::from_ptr(msg.cast()).to_bytes().to_vec()
        };
        let input = if input.is_null() {
            Vec::new()
        } else if flags.intersects(prompt_flags::PROMPT_NOFORMAT) {
            CStr::from_ptr(input.cast()).to_bytes().to_vec()
        } else {
            let ft = prompt_state_format_tree(fs);
            let expanded = format_expand_time(ft, input);
            let out = CStr::from_ptr(expanded.cast()).to_bytes().to_vec();
            free_(expanded);
            format_free(ft);
            out
        };
        let keys = if options_get_number_(oo, "status-keys") == modekey::MODEKEY_VI as i64 {
            modekey::MODEKEY_VI
        } else {
            modekey::MODEKEY_EMACS
        };
        let separators = CStr::from_ptr(options_get_string_(oo, "word-separators").cast());
        PromptSetup {
            message,
            input,
            ty,
            flags,
            keys,
            word_separators: separators.to_bytes().to_vec(),
            look: prompt_look(s),
        }
    }
}

/// A format tree for the prompt's target: from `fs` if it is still valid,
/// otherwise the defaults. No client, as in 3.8.
///
/// # Safety
/// `fs` must be null or point to a find state.
unsafe fn prompt_state_format_tree(fs: *mut cmd_find_state) -> *mut format_tree {
    unsafe {
        if !fs.is_null() && cmd_find_valid_state(fs) {
            format_create_from_state(null_mut(), null_mut(), fs)
        } else {
            format_create_defaults(null_mut(), null_mut(), null_mut(), null_mut(), null_mut())
        }
    }
}

/// Expands `s` with `ft` into bytes.
///
/// # Safety
/// `ft` must be a valid format tree.
unsafe fn prompt_expand_bytes(ft: *mut format_tree, s: &[u8]) -> Vec<u8> {
    unsafe {
        let Ok(s) = CString::new(s) else {
            return Vec::new();
        };
        let expanded = format_expand_time(ft, s.as_ptr().cast());
        let out = CStr::from_ptr(expanded.cast()).to_bytes().to_vec();
        free_(expanded);
        out
    }
}

/// The expanded message-format of a prompt and its style (3.8's
/// `prompt_format_tree`, `prompt_expand1` and `prompt_effective_style`).
struct Expanded {
    label: String,
    style: style,
}

/// Expands the prompt's message and message-format, and its style, with
/// the prompt formats (`prompt_input`, `prompt_flags`, `prompt_type`,
/// `command_prompt` and message).
///
/// # Safety
/// `pr` must be valid for the call and not change during it, `fs` null or
/// a find state.
unsafe fn prompt_expand(pr: *const Prompt, fs: *mut cmd_find_state) -> Expanded {
    unsafe {
        let input = (*pr).input();
        let flags = (*pr).flags_string();
        let ty = (*pr).prompt_type().name();
        let command_mode = (*pr).command_mode();
        let (gc, style_str) = {
            let (gc, s) = (*pr).style();
            (*gc, s.to_vec())
        };
        let message = (*pr).message().to_vec();
        let message_format = (*pr).look().message_format.clone();

        let ft = prompt_state_format_tree(fs);
        format_add!(ft, "prompt_input", "{}", String::from_utf8_lossy(&input));
        format_add!(ft, "prompt_flags", "{flags}");
        format_add!(ft, "prompt_type", "{ty}");
        format_add!(ft, "command_prompt", "{}", if command_mode { "1" } else { "0" });

        let mut sy: style = STYLE_DEFAULT;
        style_set(&raw mut sy, &raw const gc);
        if !style_str.is_empty() {
            let expanded = prompt_expand_bytes(ft, &style_str);
            let expanded = CString::new(expanded).unwrap_or_default();
            if style_parse(&raw mut sy, &raw const GRID_DEFAULT_CELL, expanded.as_ptr().cast()) != 0 {
                style_set(&raw mut sy, &raw const gc);
            }
        }

        let message = prompt_expand_bytes(ft, &message);
        format_add!(ft, "message", "{}", String::from_utf8_lossy(&message));
        let label = prompt_expand_bytes(ft, &message_format);
        format_free(ft);

        Expanded {
            label: String::from_utf8_lossy(&label).into_owned(),
            style: sy,
        }
    }
}

/// How the message style aligns the prompt.
fn prompt_align(sy: &style) -> Align {
    match sy.align {
        style_align::STYLE_ALIGN_CENTRE | style_align::STYLE_ALIGN_ABSOLUTE_CENTRE => Align::Centre,
        style_align::STYLE_ALIGN_RIGHT => Align::Right,
        _ => Align::Left,
    }
}

/// Where the parts of the prompt go in an area of `aw` columns from `ax`
/// (3.8's `prompt_layout`). Mouse handling uses it, drawing uses
/// [`prompt_draw`].
///
/// # Safety
/// As [`prompt_expand`].
pub unsafe fn prompt_layout(pr: *const Prompt, fs: *mut cmd_find_state, ax: u32, aw: u32) -> Layout {
    unsafe {
        let e = prompt_expand(pr, fs);
        let label_width = if aw == 0 { 0 } else { format_width(&e.label) };
        (*pr).layout(ax, aw, label_width, prompt_align(&e.style))
    }
}

/// Draws the prompt on line `py` of the screen of `ctx`, in an area of
/// `aw` columns from `ax`, and returns the cursor column (3.8's
/// `prompt_draw`). It also sets the screen's default cursor style and
/// colour from the prompt cursor options, so the terminal cursor shows
/// them while the prompt has it.
///
/// # Safety
/// `pr` must be valid for the call and not change during it, `fs` null or
/// a find state, `ctx` a started `screen_write` context.
pub unsafe fn prompt_draw(
    pr: *const Prompt,
    fs: *mut cmd_find_state,
    ctx: *mut screen_write_ctx,
    ax: u32,
    aw: u32,
    py: u32,
) -> u32 {
    unsafe {
        let s = (*ctx).s;
        let (cursor_style, cursor_colour) = (*pr).cursor();
        (*s).default_cstyle = screen_cursor_style::SCREEN_CURSOR_DEFAULT;
        (*s).default_mode = mode_flag::empty();
        screen_set_cursor_style(cursor_style, &raw mut (*s).default_cstyle, &raw mut (*s).default_mode);
        (*s).default_ccolour = cursor_colour;

        let e = prompt_expand(pr, fs);
        let label_width = if aw == 0 { 0 } else { format_width(&e.label) };
        let pl = (*pr).layout(ax, aw, label_width, prompt_align(&e.style));
        let input = (*pr).input_cells(&pl);
        let completions = (*pr).completion_cells(&pl);
        let mut gc = e.style.gc;

        screen_write_cursormove(ctx, ax as i32, py as i32, 0);
        if e.style.fill != 8 {
            screen_write_clearcharacter(ctx, aw, e.style.fill as u32);
        }

        if pl.content_width != 0 {
            screen_write_cursormove(ctx, pl.content_x as i32, py as i32, 0);
            if pl.label_width != 0 {
                format_draw(ctx, &raw const gc, pl.label_width, &e.label, null_mut(), 0);
            }
            screen_write_cursormove(ctx, pl.input_x as i32, py as i32, 0);
            for ud in input {
                gc.data = ud;
                screen_write_cell(ctx, &raw const gc);
            }
            if let Some(cells) = completions {
                let mut cgc = e.style.gc;
                cgc.attr |= GridAttr::GRID_ATTR_UNDERSCORE;
                screen_write_cursormove(ctx, pl.cursor_x as i32, py as i32, 0);
                for ud in cells {
                    cgc.data = ud;
                    screen_write_cell(ctx, &raw const cgc);
                }
            }
        }
        pl.cursor_x
    }
}

/// Calls an owner's prompt callback for `fire` with client `c` (null for
/// none) and returns what it said: a callback returns 0 to close the
/// prompt. Without a callback the prompt closes.
///
/// # Safety
/// `cb` must accept `data` and `c`, and may change or free the owner of
/// the prompt: the caller looks the owner up again afterwards.
pub unsafe fn prompt_call(
    cb: prompt_input_cb,
    c: *mut client,
    data: *mut c_void,
    fire: &Fire,
) -> CallbackResult {
    unsafe {
        let (Some(cb), Some(data)) = (cb, NonNull::new(data)) else {
            return CallbackResult::Close;
        };
        let text = fire.text.as_ref().map(|t| CString::new(t.clone()).unwrap_or_default());
        let ptr = text.as_ref().map_or(null(), |t| t.as_ptr().cast());
        if cb(c, data, ptr, fire.kind) == 0 {
            CallbackResult::Close
        } else {
            CallbackResult::Continue
        }
    }
}

/// A prompt held by a pane or a mode, with what its owner gave it (3.8's
/// `window_pane_prompt` and `mode_tree_prompt` wrappers). The status line
/// prompt keeps the same in client fields.
pub struct OwnedPrompt {
    pub prompt: Prompt,
    /// The target, for expanding the message.
    pub state: cmd_find_state,
    /// The client passed to the callback: the one that opened the
    /// prompt, then the one typing during a key and none after it.
    pub client: Option<ClientId>,
    pub ty: PromptType,
    inputcb: prompt_input_cb,
    freecb: prompt_free_cb,
    data: *mut c_void,
}

impl OwnedPrompt {
    /// A prompt for client `c` (null for none) and target `fs` (null for
    /// none), whose callback gets `data` and returns 0 to close.
    ///
    /// # Safety
    /// `c` must be null or a valid client, `fs` null or a find state, and
    /// the callbacks must accept `data`.
    pub unsafe fn new<T>(
        setup: PromptSetup,
        c: *mut client,
        fs: *mut cmd_find_state,
        inputcb: unsafe fn(*mut client, NonNull<T>, *const u8, KeyResult) -> i32,
        freecb: unsafe fn(NonNull<T>),
        data: *mut T,
    ) -> Self {
        unsafe {
            let ty = setup.ty;
            Self {
                prompt: Prompt::new(setup),
                state: if fs.is_null() { cmd_find_state::default() } else { *fs },
                client: if c.is_null() { None } else { Some((*c).id) },
                ty,
                inputcb: Some(std::mem::transmute::<
                    unsafe fn(*mut client, NonNull<T>, *const u8, KeyResult) -> i32,
                    unsafe fn(*mut client, NonNull<c_void>, *const u8, KeyResult) -> i32,
                >(inputcb)),
                freecb: Some(std::mem::transmute::<unsafe fn(NonNull<T>), unsafe fn(NonNull<c_void>)>(
                    freecb,
                )),
                data: data.cast(),
            }
        }
    }

    /// The owner's data, which tells two prompts apart.
    pub fn data(&self) -> *mut c_void {
        self.data
    }

    /// The callback and what it gets, to call with [`prompt_call`] after
    /// the reference to the prompt is gone. The client is looked up, so
    /// one that went away is null.
    ///
    /// # Safety
    /// The client registry must be usable.
    pub unsafe fn callback(&self) -> (prompt_input_cb, *mut client, *mut c_void) {
        unsafe {
            let c = self.client.and_then(|id| client_from_id(id)).unwrap_or(null_mut());
            (self.inputcb, c, self.data)
        }
    }

    /// Calls the owner's free function: the prompt is gone.
    ///
    /// # Safety
    /// The free function must accept the data, which is not used again.
    pub unsafe fn release(self) {
        unsafe {
            if let (Some(freecb), Some(data)) = (self.freecb, NonNull::new(self.data)) {
                freecb(data);
            }
        }
    }
}
