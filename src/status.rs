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
use std::time::Duration;

use std::rc::Rc;

use crate::prompt::{CallbackResult, Fire, KeyResult, Prompt, PromptEnv};
use crate::prompt_draw::{prompt_draw, prompt_setup};
use crate::prompt_history::{
    HistoryPositions, prompt_history_add, prompt_history_down, prompt_history_up,
};
use crate::*;
use crate::options_::*;


/// Fire the status timer for a client: redraw the status line and re-arm.
unsafe fn status_timer_fire(cid: ClientId) {
    unsafe {
        let Some(c) = client_from_id(cid) else { return };
        let s: *mut session = client_get_session(c);

        // Cancel existing timer before potentially re-arming.
        (*c).status.timer = None;

        if s.is_null() {
            return;
        }

        if (*c).message_string.is_none() && (*c).prompt.is_none() {
            (*c).flags |= client_flag::REDRAWSTATUS;
        }

        let interval = options_get_number_((*s).options, "status-interval");
        if interval != 0 {
            (*c).status.timer = timer_add(
                Duration::from_secs(interval as u64),
                Box::new(move || status_timer_fire(cid)),
            );
        }
        log_debug!("client {:p}, status interval {}", c, interval);
    }
}

/// Start status timer for client.
pub unsafe fn status_timer_start(c: NonNull<client>) {
    unsafe {
        let c = c.as_ptr();
        let s: *mut session = client_get_session(c);

        // Cancel any existing timer.
        (*c).status.timer = None;

        if !s.is_null() && options_get_number_((*s).options, "status") != 0 {
            status_timer_fire((*c).id);
        }
    }
}

/// Start status timer for all clients.
pub unsafe fn status_timer_start_all() {
    unsafe {
        for c in clients_iter() {
            status_timer_start(NonNull::new_unchecked(c));
        }
    }
}

/// Update status cache.
pub unsafe fn status_update_cache(s: *mut session) {
    unsafe {
        (*s).statuslines = options_get_number_((*s).options, "status") as u32;
        if (*s).statuslines == 0 {
            (*s).statusat = -1;
        } else if options_get_number_((*s).options, "status-position") == 0 {
            (*s).statusat = 0;
        } else {
            (*s).statusat = 1;
        }
    }
}

/// Get screen line of status line. -1 means off.
pub unsafe fn status_at_line(c: *mut client) -> i32 {
    unsafe {
        let s: *mut session = client_get_session(c);

        if (*c)
            .flags
            .intersects(client_flag::STATUSOFF | client_flag::CONTROL)
        {
            return -1;
        }
        if (*s).statusat != 1 {
            return (*s).statusat;
        }
        (*c).tty.sy as i32 - status_line_size(c) as i32
    }
}

/// Get size of status line for client's session. 0 means off.
pub unsafe fn status_line_size(c: *mut client) -> u32 {
    unsafe {
        let s: *mut session = client_get_session(c);

        if (*c)
            .flags
            .intersects(client_flag::STATUSOFF | client_flag::CONTROL)
        {
            return 0;
        }
        if s.is_null() {
            return options_get_number_(GLOBAL_S_OPTIONS, "status") as u32;
        }
        (*s).statuslines
    }
}

/// Get the prompt line number for client's session. 1 means at the bottom.
unsafe fn status_prompt_line_at(c: *mut client) -> u32 {
    unsafe {
        let s = client_get_session(c);

        if (*c)
            .flags
            .intersects(client_flag::STATUSOFF | client_flag::CONTROL)
        {
            return 1;
        }
        options_get_number_((*s).options, "message-line") as u32
    }
}

/// Get window at window list position.
pub unsafe fn status_get_range(c: *mut client, x: u32, y: u32) -> *mut style_range {
    unsafe {
        let sl = &raw mut (*c).status;

        if y >= (*sl).entries.len() as u32 {
            return null_mut();
        }
        for sr in &mut (*sl).entries[y as usize].ranges {
            if x >= sr.start && x < sr.end {
                return sr as *mut style_range;
            }
        }
        null_mut()
    }
}

/// Free all ranges.
unsafe fn status_free_ranges(srs: *mut style_ranges) {
    unsafe {
        (*srs).clear();
    }
}

/// Drop ranges Vec (for use when the owning struct is being freed).
/// After dropping, writes an empty Vec so a later automatic Drop
/// (e.g. when the owning `client` Box is freed) is a no-op.
unsafe fn status_drop_ranges(srs: *mut style_ranges) {
    unsafe {
        std::ptr::drop_in_place(srs);
        std::ptr::write(srs, Vec::new());
    }
}

/// Save old status line.
unsafe fn status_push_screen(c: *mut client) {
    unsafe {
        let sl = &raw mut (*c).status;

        if (*sl).active == &raw mut (*sl).screen {
            (*sl).active = Box::into_raw(Box::<screen>::new_uninit()).cast::<screen>();
            screen_init((*sl).active, (*c).tty.sx, status_line_size(c), 0);
        }
        (*sl).references += 1;
    }
}

/// Restore old status line.
unsafe fn status_pop_screen(c: *mut client) {
    unsafe {
        let sl = &raw mut (*c).status;

        (*sl).references -= 1;
        if (*sl).references == 0 {
            screen_free((*sl).active);
            free_((*sl).active);
            (*sl).active = &raw mut (*sl).screen;
        }
    }
}

/// Initialize status line.
pub unsafe fn status_init(c: *mut client) {
    unsafe {
        let sl = &raw mut (*c).status;

        std::ptr::write(&raw mut (*sl).timer, None);
        for i in 0..(*sl).entries.len() {
            std::ptr::write(&raw mut (*sl).entries[i].ranges, Vec::new());
        }

        screen_init(&raw mut (*sl).screen, (*c).tty.sx, 1, 0);
        (*sl).active = &raw mut (*sl).screen;
    }
}

/// Free status line.
pub unsafe fn status_free(c: *mut client) {
    unsafe {
        let sl = &raw mut (*c).status;

        for i in 0..(*sl).entries.len() {
            status_drop_ranges(&raw mut (*sl).entries[i].ranges);
            free_((*sl).entries[i].expanded);
        }

        (*sl).timer = None;

        if (*sl).active != &raw mut (*sl).screen {
            screen_free((*sl).active);
            free_((*sl).active);
        }
        screen_free(&raw mut (*sl).screen);
    }
}

/// Draw status line for client.
pub unsafe fn status_redraw(c: *mut client) -> i32 {
    unsafe {
        let sl = &raw mut (*c).status;
        // status_line_entry *sle;
        let s = client_get_session(c);
        let mut ctx: screen_write_ctx = zeroed();
        let mut gc: GridCell = zeroed();

        // u_int lines, i, n;

        let width = (*c).tty.sx;

        let mut force = false;
        let mut changed = false;

        // int flags, force = 0, changed = 0, fg, bg;

        // struct options_entry *o;
        // union options_value *ov;
        // struct format_tree *ft;
        // char *expanded;

        log_debug!("status_redraw enter");

        // Shouldn't get here if not the active screen.
        if (*sl).active != &raw mut (*sl).screen {
            fatalx("not the active screen");
        }

        // No status line?
        let lines = status_line_size(c);
        if (*c).tty.sy == 0 || lines == 0 {
            return 1;
        }

        // Create format tree.
        let mut flags = format_flags::FORMAT_STATUS;
        if (*c).flags.intersects(client_flag::STATUSFORCE) {
            flags |= format_flags::FORMAT_FORCE;
        }
        let ft = format_create(c, null_mut(), FORMAT_NONE, flags);
        format_defaults(ft, c, None, None, None);

        // Set up default colour.
        style_apply(&raw mut gc, (*s).options, c!("status-style"), ft);
        let fg = options_get_number_((*s).options, "status-fg") as i32;
        if !COLOUR_DEFAULT(fg) {
            gc.fg = fg;
        }
        let bg = options_get_number_((*s).options, "status-bg") as i32;
        if !COLOUR_DEFAULT(bg) {
            gc.bg = bg;
        }
        if !grid_cells_equal(&gc, &(*sl).style) {
            force = true;
            memcpy__(&raw mut (*sl).style, &raw mut gc);
        }

        // Resize the target screen.
        if screen_size_x(&raw mut (*sl).screen) != width
            || screen_size_y(&raw mut (*sl).screen) != lines
        {
            screen_resize(&raw mut (*sl).screen, width, lines, 0);
            changed = true;
            force = true;
        }
        screen_write_start(&raw mut ctx, &raw mut (*sl).screen);

        // Write the status lines.
        let o = options_get(&mut *(*s).options, "status-format");
        if o.is_null() {
            for _ in 0..(width * lines) {
                screen_write_putc(&raw mut ctx, &raw mut gc, b' ');
            }
        } else {
            for i in 0..lines {
                screen_write_cursormove(&raw mut ctx, 0, i as i32, 0);

                let Some(value) = options_array_string(o, &ArrayKey::Number(i)) else {
                    for _ in 0..width {
                        screen_write_putc(&raw mut ctx, &raw mut gc, b' ');
                    }
                    continue;
                };
                let sle = &raw mut (*sl).entries[i as usize];

                let expanded = format_expand_time(ft, value.as_ptr().cast());
                if !force
                    && !(*sle).expanded.is_null()
                    && libc::strcmp(expanded, (*sle).expanded) == 0
                {
                    free_(expanded);
                    continue;
                }
                changed = true;

                for _ in 0..width {
                    screen_write_putc(&raw mut ctx, &raw mut gc, b' ');
                }
                screen_write_cursormove(&raw mut ctx, 0, i as i32, 0);

                status_free_ranges(&raw mut (*sle).ranges);
                format_draw(
                    &raw mut ctx,
                    &raw mut gc,
                    width,
                    cstr_to_str(expanded),
                    &raw mut (*sle).ranges,
                    0,
                );

                free_((*sle).expanded);
                (*sle).expanded = expanded;
            }
        }
        screen_write_stop(&raw mut ctx);

        // Free the format tree.
        format_free(ft);

        // Return if the status line has changed.
        // log_debug("%s exit: force=%d, changed=%d", __func__, force, changed);
        (force || changed) as i32
    }
}

macro_rules! status_message_set {
   ($c:expr, $delay:expr, $ignore_styles:expr, $ignore_keys:expr, $fmt:literal $(, $args:expr)* $(,)?) => {
        crate::status::status_message_set_($c, $delay, $ignore_styles, $ignore_keys, format_args!($fmt $(, $args)*))
    };
}
pub(crate) use status_message_set;

/// Set a status line message.
pub unsafe fn status_message_set_(
    c: *mut client,
    mut delay: i32,
    ignore_styles: i32,
    ignore_keys: bool,
    args: std::fmt::Arguments,
) {
    unsafe {
        let s = args.to_string();

        // log_debug("%s: %s", __func__, s);

        if c.is_null() {
            server_add_message!("message: {}", s);
            return;
        }

        status_message_clear(NonNull::new_unchecked(c));
        status_push_screen(c);
        server_add_message!("{} message: {}", _s((*c).name), s);
        (*c).message_string = Some(s);

        // With delay -1, the display-time option is used; zero means wait for
        // key press; more than zero is the actual delay time in milliseconds.
        if delay == -1 {
            delay = options_get_number_((*client_get_session(c)).options, "display-time") as i32;
        }
        if delay > 0 {
            (*c).message_timer = None;
            let cid = (*c).id;
            (*c).message_timer = timer_add(
                Duration::from_millis(delay as u64),
                Box::new(move || status_message_timer_fire(cid)),
            );
        }

        if delay != 0 {
            (*c).message_ignore_keys = ignore_keys as i32;
        }
        (*c).message_ignore_styles = ignore_styles;

        (*c).tty.flags |= tty_flags::TTY_NOCURSOR | tty_flags::TTY_FREEZE;
        (*c).flags |= client_flag::REDRAWSTATUS;
    }
}

/// Clear status line message.
pub unsafe fn status_message_clear(c: NonNull<client>) {
    unsafe {
        let c = c.as_ptr();
        if (*c).message_string.is_none() {
            return;
        }

        (*c).message_string = None;

        if (*c).prompt.is_none() {
            (*c).tty.flags &= !(tty_flags::TTY_NOCURSOR | tty_flags::TTY_FREEZE);
        }
        (*c).flags |= CLIENT_ALLREDRAWFLAGS; /* was frozen and may have changed */

        status_pop_screen(c);
    }
}

/// Clear status line message after timer expires.
unsafe fn status_message_timer_fire(cid: ClientId) {
    unsafe {
        let Some(c) = client_from_id(cid) else { return };
        status_message_clear(NonNull::new_unchecked(c));
    }
}

/// Draw client message on status line of present else on last line.
pub unsafe fn status_message_redraw(c: *mut client) -> i32 {
    unsafe {
        let sl = &raw mut (*c).status;
        let mut ctx: screen_write_ctx = zeroed();
        let s = client_get_session(c);
        // size_t len;
        // u_int lines, offset, messageline;
        let mut gc: GridCell = zeroed();
        // struct format_tree *ft;

        if (*c).tty.sx == 0 || (*c).tty.sy == 0 {
            return 0;
        }
        // Snapshot the old Grid before reinit overwrites it.
        let old_grid = std::mem::replace(&mut (*(*sl).active).grid, Box::new(Grid::new(0, 0, 0)));

        let mut lines = status_line_size(c);
        if lines <= 1 {
            lines = 1;
        }
        screen_init((*sl).active, (*c).tty.sx, lines, 0);

        let mut messageline = status_prompt_line_at(c);
        if messageline > lines - 1 {
            messageline = lines - 1;
        }

        let (ax, aw) = status_message_area(c);

        let ft = format_create_defaults(null_mut(), c, null_mut(), null_mut(), null_mut());
        style_apply(&raw mut gc, (*s).options, c!("message-style"), ft);

        // With styles ignored, # is doubled so format_draw shows the
        // message as it is.
        let msg = (*c).message_string.as_deref().unwrap_or("");
        if (*c).message_ignore_styles != 0 {
            format_add!(ft, "message", "{}", msg.replace('#', "##"));
        } else {
            format_add!(ft, "message", "{msg}");
        }
        format_add!(ft, "command_prompt", "0");
        let expanded = format_expand_time(ft, options_get_string_((*s).options, "message-format"));
        let expanded_str = cstr_to_string_lossy(expanded);
        free_(expanded);
        format_free(ft);

        screen_write_start(&raw mut ctx, (*sl).active);
        screen_write_fast_copy(
            &raw mut ctx,
            &raw mut (*sl).screen,
            0,
            0,
            (*c).tty.sx,
            lines,
        );
        screen_write_cursormove(&raw mut ctx, ax as i32, messageline as i32, 0);
        format_draw(&raw mut ctx, &raw const gc, aw, &expanded_str, null_mut(), 0);
        screen_write_stop(&raw mut ctx);

        if (*(*sl).active).grid.visible_eq(&old_grid) {
            return 0;
        }
        1
    }
}

/// What the status line prompt reads from the rest of tmux, see
/// [`PromptEnv`].
struct StatusPromptEnv {
    /// The command-alias names, read before the key when it is Tab.
    aliases: Vec<String>,
}

impl PromptEnv for StatusPromptEnv {
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
unsafe fn status_prompt_aliases() -> Vec<String> {
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

/// Enable status line prompt.
pub unsafe fn status_prompt_set<T>(
    c: *mut client,
    fs: *mut cmd_find_state,
    msg: *const u8,
    input: *const u8,
    inputcb: unsafe fn(*mut client, NonNull<T>, *const u8, KeyResult) -> i32,
    freecb: unsafe fn(NonNull<T>),
    data: *mut T,
    flags: prompt_flags,
    prompt_type: PromptType,
) {
    unsafe {
        server_client_clear_overlay(c);

        status_message_clear(NonNull::new_unchecked(c));
        status_prompt_clear(c);
        status_push_screen(c);

        let setup = prompt_setup(client_get_session(c), fs, msg, input, prompt_type, flags);
        (*c).prompt = Some(Box::new(Prompt::new(setup)));
        (*c).prompt_state = if fs.is_null() { cmd_find_state::default() } else { *fs };

        (*c).prompt_inputcb = Some(std::mem::transmute::<
            unsafe fn(*mut client, NonNull<T>, *const u8, KeyResult) -> i32,
            unsafe fn(*mut client, NonNull<c_void>, *const u8, KeyResult) -> i32,
        >(inputcb));
        (*c).prompt_freecb = Some(std::mem::transmute::<
            unsafe fn(NonNull<T>),
            unsafe fn(NonNull<c_void>),
        >(freecb));
        (*c).prompt_data = data.cast(); // note we know this is non null

        if !flags.intersects(prompt_flags::PROMPT_INCREMENTAL) {
            (*c).tty.flags |= tty_flags::TTY_FREEZE;
        }
        (*c).flags |= client_flag::REDRAWSTATUS;

        let start = (*c).prompt.as_ref().and_then(|p| p.incremental_start());
        if let Some(fire) = start {
            status_prompt_fire(c, fire, KeyResult::Handled);
        }
    }
}

/// Remove status line prompt.
pub unsafe fn status_prompt_clear(c: *mut client) {
    unsafe {
        if (*c).prompt.take().is_none() {
            return;
        }

        if let (Some(prompt_freecb), Some(prompt_data)) =
            ((*c).prompt_freecb, NonNull::new((*c).prompt_data))
        {
            prompt_freecb(prompt_data);
        }
        (*c).prompt_inputcb = None;
        (*c).prompt_freecb = None;
        (*c).prompt_data = null_mut();

        (*c).tty.flags &= !(tty_flags::TTY_NOCURSOR | tty_flags::TTY_FREEZE);
        (*c).flags |= CLIENT_ALLREDRAWFLAGS; /* was frozen and may have changed */

        status_pop_screen(c);
    }
}

/// Update status line prompt with a new prompt string.
pub unsafe fn status_prompt_update(c: *mut client, msg: *const u8, input: *const u8) {
    unsafe {
        if (*c).prompt.is_none() {
            return;
        }
        // The new input is expanded with the prompt's target, the
        // message when it is drawn (3.8's prompt_update).
        let setup = prompt_setup(
            client_get_session(c),
            &raw mut (*c).prompt_state,
            msg,
            input,
            PromptType::Command,
            prompt_flags::empty(),
        );
        if let Some(prompt) = (*c).prompt.as_mut() {
            prompt.update(setup.message, &setup.input);
        }
        (*c).flags |= client_flag::REDRAWSTATUS;
    }
}

/// Fire the prompt's callback for `fire` and returns the key's result.
/// The callback may update, replace or clear the prompt.
///
/// # Safety
/// `c` must be a valid client.
unsafe fn status_prompt_fire(c: *mut client, fire: Fire, result: KeyResult) -> KeyResult {
    unsafe {
        let said = match ((*c).prompt_inputcb, NonNull::new((*c).prompt_data)) {
            (Some(cb), Some(data)) => {
                let text = fire.text.as_ref().map(|t| CString::new(t.clone()).unwrap_or_default());
                let ptr = text.as_ref().map_or(null(), |t| t.as_ptr().cast());
                if cb(c, data, ptr, fire.kind) == 0 {
                    CallbackResult::Close
                } else {
                    CallbackResult::Continue
                }
            }
            _ => CallbackResult::Close,
        };
        match (*c).prompt.as_mut() {
            Some(prompt) => {
                let result = prompt.fired(&fire, said, result);
                if said == CallbackResult::Continue {
                    (*c).flags |= client_flag::REDRAWSTATUS;
                }
                result
            }
            None => result,
        }
    }
}

/// Handle a key in the prompt.
pub unsafe fn status_prompt_key(c: *mut client, key: key_code) -> KeyResult {
    unsafe {
        let aliases = if key & KEYC_MASK_KEY == b'\t' as key_code {
            status_prompt_aliases()
        } else {
            Vec::new()
        };
        let mut env = StatusPromptEnv { aliases };
        let Some(out) = (*c).prompt.as_mut().map(|p| p.key(key, &mut env)) else {
            return KeyResult::NotHandled;
        };

        if let Some((ty, line)) = &out.history {
            prompt_history_add(*ty, line);
        }
        let mut result = out.result;
        for fire in out.fires {
            result = status_prompt_fire(c, fire, result);
        }
        if out.redraw && (*c).prompt.is_some() {
            (*c).flags |= client_flag::REDRAWSTATUS;
        }
        if (*c).prompt.as_ref().is_some_and(|p| p.closed()) {
            status_prompt_clear(c);
        }
        result
    }
}

/// Draw client prompt on status line of present else on last line.
pub unsafe fn status_prompt_redraw(c: *mut client) -> i32 {
    unsafe {
        let sl = &raw mut (*c).status;
        let mut ctx: screen_write_ctx = zeroed();

        let Some(pr) = (*c).prompt.as_deref().map(|p| p as *const Prompt) else {
            return 0;
        };
        if (*c).tty.sx == 0 || (*c).tty.sy == 0 {
            return 0;
        }
        // Snapshot the old Grid before reinit overwrites it.
        let old_grid = std::mem::replace(&mut (*(*sl).active).grid, Box::new(Grid::new(0, 0, 0)));

        let mut lines = status_line_size(c);
        if lines <= 1 {
            lines = 1;
        }
        screen_init((*sl).active, (*c).tty.sx, lines, 0);

        let mut promptline = status_prompt_line_at(c);
        if promptline > lines - 1 {
            promptline = lines - 1;
        }
        let (ax, aw) = status_message_area(c);

        screen_write_start(&raw mut ctx, (*sl).active);
        screen_write_fast_copy(
            &raw mut ctx,
            &raw mut (*sl).screen,
            0,
            0,
            (*c).tty.sx,
            lines,
        );
        // The prompt is not changed while it is drawn.
        (*sl).prompt_cx = prompt_draw(pr, &raw mut (*c).prompt_state, &raw mut ctx, ax, aw, promptline);
        screen_write_stop(&raw mut ctx);

        if (*(*sl).active).grid.visible_eq(&old_grid) {
            return 0;
        }
        1
    }
}

/// The columns of the message and prompt area: the whole line, aligned
/// by the align directive of message-style (3.8's `status_message_area`).
/// 3.8 also takes a width from message-style, which waits for the width
/// directive of styles.
///
/// # Safety
/// `c` must be a valid client with a session.
unsafe fn status_message_area(c: *mut client) -> (u32, u32) {
    unsafe {
        let s = client_get_session(c);
        let w = (*c).tty.sx;
        let sy = options_string_to_style((*s).options, "message-style", null_mut());
        let ax = if sy.is_null() {
            0
        } else {
            match (*sy).align {
                style_align::STYLE_ALIGN_CENTRE | style_align::STYLE_ALIGN_ABSOLUTE_CENTRE => ((*c).tty.sx - w) / 2,
                style_align::STYLE_ALIGN_RIGHT => (*c).tty.sx - w,
                _ => 0,
            }
        };
        (ax, w)
    }
}
