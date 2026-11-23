// Copyright (c) 2008 Nicholas Marriott <nicholas.marriott@gmail.com>
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

//! What has to happen after an option changes, as `options_push_changes`
//! in options.c. Callers that set options run it once they are done.
//!
//! This reaches into many subsystems, which is why it is apart from the
//! store. 3.8 has more cases for features tmux-rs does not have yet
//! (themes, pane scrollbars, the redraw scene, codepoint-widths and
//! input-buffer-size). They are added with those features.

use crate::options_::options_get_number_;
use crate::*;

/// Applies a change to option `name`: rebuilds what depends on it, then
/// updates the status line caches, recalculates sizes and redraws every
/// attached client.
pub unsafe fn options_push_changes(name: &str) {
    unsafe {
        log_debug!("options_push_changes: {}", name);

        if name == "automatic-rename" {
            for w in windows_iter() {
                let active = window_active_pane(w);
                if active.is_null() {
                    continue;
                }
                if options_get_number_((*w).options, name) != 0 {
                    (*active).flags |= window_pane_flags::PANE_CHANGED;
                }
            }
        }

        if name == "cursor-colour" || name == "cursor-style" {
            for wp in all_panes() {
                window_pane_default_cursor(wp);
            }
        }

        if name == "fill-character" {
            for w in windows_iter() {
                window_set_fill_character(NonNull::new(w).unwrap());
            }
        }

        if name == "key-table" {
            for loop_ in clients_iter() {
                server_client_set_key_table(loop_, null_mut());
            }
        }

        if name == "user-keys" {
            for loop_ in clients_iter() {
                if (*loop_).tty.flags.intersects(tty_flags::TTY_OPENED) {
                    tty_keys_build(&mut (*loop_).tty);
                }
            }
        }

        if name == "status" || name == "status-interval" {
            status_timer_start_all();
        }

        if name == "monitor-silence" {
            alerts_reset_all();
        }

        if name == "window-style" || name == "window-active-style" {
            for wp in all_panes() {
                (*wp).flags |= window_pane_flags::PANE_STYLECHANGED;
            }
        }

        // A style can refer to a user option, so recheck styles when one
        // changes.
        if name.starts_with('@') {
            for wp in all_panes() {
                (*wp).flags |= window_pane_flags::PANE_STYLECHANGED;
            }
        }

        if name == "pane-colours" {
            for wp in all_panes() {
                colour_palette_from_option(Some(&mut (*wp).palette), (*wp).options);
            }
        }

        if name == "pane-border-status" {
            for w in windows_iter() {
                layout_fix_panes(&*w, null_mut());
            }
        }

        if name == "history-limit" {
            for s in sessions_iter() {
                session_update_history(s);
            }
        }

        for s in sessions_iter() {
            status_update_cache(s);
        }

        recalculate_sizes();

        for loop_ in clients_iter() {
            if !client_get_session(loop_).is_null() {
                server_redraw_client(loop_);
            }
        }
    }
}

/// Every pane, collected first so the loops above can change panes.
unsafe fn all_panes() -> Vec<*mut window_pane> {
    unsafe { (*(&raw mut ALL_WINDOW_PANES)).values().copied().collect() }
}
