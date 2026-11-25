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
//! Prompts owned by a pane (command-prompt -P), drawn over the pane's
//! last line, or its first with the status line at the top (3.8's
//! `window_pane_set_prompt` and friends in window.c, and
//! `redraw_draw_pane_prompt` in screen-redraw.c).
//!
//! The pane holds an [`OwnedPrompt`]: the prompt object and its owner's
//! callback. Keys come from `server_client.rs`, which offers them to the
//! active pane's prompt, or the first visible pane with one, after the
//! status line prompt. Opening and closing fire pane-prompt-opened and
//! pane-prompt-closed, except when the pane is destroyed.

use crate::events::{events_fire, payload_for_pane};
use crate::prompt::KeyResult;
use crate::prompt_draw::{OwnedPrompt, prompt_call, prompt_draw, prompt_env, prompt_layout, prompt_setup};
use crate::prompt_history::{PromptType, prompt_history_add};
use crate::*;

/// Fires pane-prompt-opened or pane-prompt-closed for `wp`.
///
/// # Safety
/// `wp` must be a valid pane in a valid window.
unsafe fn window_fire_pane_prompt(name: &str, wp: *mut window_pane, ty: PromptType) {
    unsafe {
        let mut payload = payload_for_pane(wp);
        payload.set_string("prompt_type", ty.name());
        events_fire(name, payload, null_mut());
    }
}

/// Opens a prompt on `wp` for client `c` (null for none), replacing any
/// prompt it has. The callback gets `data` and returns 0 to close, the
/// free function is called when the prompt goes.
///
/// # Safety
/// `wp` must be a valid pane in a valid window, `c` null or a valid
/// client, `fs` null or a find state, `msg` and `input` null or NUL
/// terminated, and the callbacks must accept `data`.
#[expect(clippy::too_many_arguments, reason = "3.8's window_pane_set_prompt")]
pub unsafe fn window_pane_set_prompt<T>(
    wp: *mut window_pane,
    c: *mut client,
    fs: *mut cmd_find_state,
    msg: *const u8,
    input: *const u8,
    inputcb: unsafe fn(*mut client, NonNull<T>, *const u8, KeyResult) -> i32,
    freecb: unsafe fn(NonNull<T>),
    data: *mut T,
    flags: prompt_flags,
    ty: PromptType,
) {
    unsafe {
        let s = if c.is_null() { null_mut() } else { client_get_session(c) };
        window_pane_clear_prompt(wp);

        let setup = prompt_setup(s, fs, msg, input, ty, flags);
        (*wp).prompt = Some(Box::new(OwnedPrompt::new(setup, c, fs, inputcb, freecb, data)));
        (*wp).flags |= window_pane_flags::PANE_REDRAW;

        let pane = PaneId((*wp).id);
        let start = (*wp).prompt.as_ref().and_then(|pp| pp.prompt.incremental_start());
        if let Some(fire) = start {
            window_pane_prompt_fire(pane, fire, KeyResult::Handled);
        }
        if let Some(wp) = pane_from_id(pane) {
            window_fire_pane_prompt("pane-prompt-opened", wp, ty);
        }
    }
}

/// Closes the prompt of `wp`, if it has one, and fires
/// pane-prompt-closed.
///
/// # Safety
/// `wp` must be a valid pane in a valid window.
pub unsafe fn window_pane_clear_prompt(wp: *mut window_pane) {
    unsafe { window_pane_remove_prompt(wp, true) }
}

/// Removes the prompt of `wp` and calls its free function, firing
/// pane-prompt-closed if `fire` (not when the pane is being destroyed).
///
/// # Safety
/// `wp` must be a valid pane, in a valid window if `fire`.
pub unsafe fn window_pane_remove_prompt(wp: *mut window_pane, fire: bool) {
    unsafe {
        let Some(pp) = (*wp).prompt.take() else {
            return;
        };
        (*wp).flags |= window_pane_flags::PANE_REDRAW;
        let ty = pp.ty;
        pp.release();
        if fire && pane_from_id(PaneId((*wp).id)).is_some() {
            window_fire_pane_prompt("pane-prompt-closed", wp, ty);
        }
    }
}

/// Whether `wp` has an open prompt.
///
/// # Safety
/// `wp` must be a valid pane.
pub unsafe fn window_pane_has_prompt(wp: *const window_pane) -> bool {
    unsafe { (*wp).prompt.is_some() }
}

/// Replaces the message and input of the open prompt of `wp` (the next
/// of several prompts).
///
/// # Safety
/// `wp` must be a valid pane, `msg` and `input` null or NUL terminated.
pub unsafe fn window_pane_update_prompt(wp: *mut window_pane, msg: *const u8, input: *const u8) {
    unsafe {
        let Some(pp) = (*wp).prompt.as_mut() else {
            return;
        };
        let state = &raw mut pp.state;
        let setup = prompt_setup(null_mut(), state, msg, input, PromptType::Command, prompt_flags::empty());
        if let Some(pp) = (*wp).prompt.as_mut() {
            pp.prompt.update(setup.message, &setup.input);
        }
        (*wp).flags |= window_pane_flags::PANE_REDRAW;
    }
}

/// Fires the callback of the prompt of pane `pane` for `fire` and
/// returns the key's result. The callback may update, replace or close
/// the prompt, or destroy the pane.
///
/// # Safety
/// The callback must be safe to call with the prompt's data.
unsafe fn window_pane_prompt_fire(pane: PaneId, fire: prompt::Fire, result: KeyResult) -> KeyResult {
    unsafe {
        let Some(wp) = pane_from_id(pane) else {
            return result;
        };
        let Some((cb, c, data)) = (*wp).prompt.as_ref().map(|pp| pp.callback()) else {
            return result;
        };
        let said = prompt_call(cb, c, data, &fire);

        let Some(wp) = pane_from_id(pane) else {
            return result;
        };
        match (*wp).prompt.as_mut() {
            Some(pp) if pp.data() == data => {
                let result = pp.prompt.fired(&fire, said, result);
                (*wp).flags |= window_pane_flags::PANE_REDRAW;
                result
            }
            _ => result,
        }
    }
}

/// The line of `wp` the prompt is on: the last, or the first when the
/// status line of `c` is at the top.
///
/// # Safety
/// `c` must be null or a valid client with a session.
unsafe fn window_pane_prompt_line(wp: *const window_pane, c: *mut client) -> u32 {
    unsafe {
        if !c.is_null() && status_at_line(c) == 0 {
            0
        } else {
            (*wp).sy.saturating_sub(1)
        }
    }
}

/// Passes a key from client `c` to the prompt of `wp` (3.8's
/// `window_pane_prompt_key`). A mouse key is a left click on the prompt
/// line. The client is given to the callback for the key only.
///
/// # Safety
/// `wp` must be a valid pane in a valid window, `c` a valid client with
/// a session, `m` null or the key's mouse event.
pub unsafe fn window_pane_prompt_key(
    wp: *mut window_pane,
    c: *mut client,
    key: key_code,
    m: *const mouse_event,
) -> KeyResult {
    unsafe {
        let pane = PaneId((*wp).id);
        let Some(pp) = (*wp).prompt.as_deref_mut() else {
            return KeyResult::NotHandled;
        };
        pp.client = Some((*c).id);
        let data = pp.data();

        let out = if KEYC_IS_MOUSE(key) {
            let mut x = 0;
            let mut y = 0;
            if m.is_null()
                || MOUSE_BUTTONS((*m).b) != MOUSE_BUTTON_1
                || MOUSE_DRAG((*m).b)
                || MOUSE_RELEASE((*m).b)
                || cmd_mouse_at(wp, &*m, &raw mut x, &raw mut y, 0) != 0
                || y != window_pane_prompt_line(wp, c)
            {
                None
            } else {
                // The prompt is not changed while it is laid out.
                let pr = (*wp).prompt.as_ref().map_or(null(), |pp| &raw const pp.prompt);
                let pl = prompt_layout(pr, window_pane_prompt_state(wp), 0, (*wp).sx);
                (*wp).prompt.as_mut().map(|pp| pp.prompt.mouse(x, &pl))
            }
        } else {
            let mut env = prompt_env(key);
            (*wp).prompt.as_mut().map(|pp| pp.prompt.key(key, &mut env))
        };
        let Some(out) = out else {
            if let Some(pp) = (*wp).prompt.as_mut() {
                pp.client = None;
            }
            return KeyResult::NotHandled;
        };

        if let Some((ty, line)) = &out.history {
            prompt_history_add(*ty, line);
        }
        let mut result = out.result;
        for fire in out.fires {
            result = window_pane_prompt_fire(pane, fire, result);
        }

        let Some(wp) = pane_from_id(pane) else {
            return result;
        };
        let same = (*wp).prompt.as_ref().is_some_and(|pp| pp.data() == data);
        if same && let Some(pp) = (*wp).prompt.as_mut() {
            pp.client = None;
        }
        // Only an explicit close or the prompt marking itself closed ends
        // it, cursor movement and editing keep it open.
        if same
            && (result == KeyResult::Close
                || (*wp).prompt.as_ref().is_some_and(|pp| pp.prompt.closed()))
        {
            window_pane_clear_prompt(wp);
        }
        if out.redraw || !same {
            (*wp).flags |= window_pane_flags::PANE_REDRAW;
        }
        result
    }
}

/// The find state of the prompt of `wp`, null without a prompt.
///
/// # Safety
/// `wp` must be a valid pane. The pointer is valid while the prompt is.
unsafe fn window_pane_prompt_state(wp: *mut window_pane) -> *mut cmd_find_state {
    unsafe {
        match (*wp).prompt.as_mut() {
            Some(pp) => &raw mut pp.state,
            None => null_mut(),
        }
    }
}

/// Draws the prompt of `wp` over its line for the client of `ctx`
/// (3.8's `redraw_draw_pane_prompt`), and remembers its cursor column.
///
/// # Safety
/// `ctx` must be a set up redraw context and `wp` a visible pane of its
/// client's window.
pub unsafe fn screen_redraw_draw_pane_prompt(ctx: *mut screen_redraw_ctx, wp: *mut window_pane) {
    unsafe {
        let Some(pr) = (*wp).prompt.as_ref().map(|pp| &raw const pp.prompt) else {
            return;
        };
        if (*wp).sx == 0 || (*wp).sy == 0 {
            return;
        }
        let c = (*ctx).c;
        let (ox, oy, sx, sy) = ((*ctx).ox, (*ctx).oy, (*ctx).sx, (*ctx).sy);

        let wy = (*wp).yoff + window_pane_prompt_line(wp, c);
        if wy < oy || wy >= oy + sy {
            return;
        }
        let line = wy - oy;
        let cy = if (*ctx).statustop != 0 { (*ctx).statuslines + line } else { line };
        if (*wp).xoff + (*wp).sx <= ox || (*wp).xoff >= ox + sx {
            return;
        }
        let (offset, px) = if (*wp).xoff < ox { (ox - (*wp).xoff, 0) } else { (0, (*wp).xoff - ox) };
        let width = ((*wp).sx - offset).min(sx - px);

        let mut screen = screen_placeholder();
        screen_init(&raw mut screen, (*wp).sx, 1, 0);
        let mut sctx: screen_write_ctx = zeroed();
        screen_write_start(&raw mut sctx, &raw mut screen);
        // The prompt is not changed while it is drawn.
        (*wp).prompt_cx = prompt_draw(pr, window_pane_prompt_state(wp), &raw mut sctx, 0, (*wp).sx, 0);
        screen_write_stop(&raw mut sctx);

        tty_draw_line(
            &raw mut (*c).tty,
            &raw mut screen,
            offset,
            0,
            width,
            px,
            cy,
            &GRID_DEFAULT_CELL,
            null(),
        );
        screen_free(&raw mut screen);
    }
}

/// Where the terminal cursor goes for the prompt of `wp` (3.8's
/// `server_client_prompt_cursor`): Some with the position and whether it
/// is shown if `wp` has a prompt, None otherwise.
///
/// # Safety
/// `c` must be a valid client with a session and `wp` a valid pane of
/// its window.
pub unsafe fn server_client_prompt_cursor(c: *mut client, wp: *mut window_pane) -> Option<(u32, u32, bool)> {
    unsafe {
        if !window_pane_has_prompt(wp) {
            return None;
        }
        let mut ox = 0;
        let mut oy = 0;
        let mut sx = 0;
        let mut sy = 0;
        tty_window_offset(&raw mut (*c).tty, &raw mut ox, &raw mut oy, &raw mut sx, &raw mut sy);
        let py = (*wp).yoff + window_pane_prompt_line(wp, c);
        let px = (*wp).xoff + (*wp).prompt_cx;
        if px < ox || px > ox + sx || py < oy || py > oy + sy {
            return Some((0, 0, false));
        }
        let mut cy = py - oy;
        if status_at_line(c) == 0 {
            cy += status_line_size(c);
        }
        Some((px - ox, cy, true))
    }
}
