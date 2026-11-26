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
use crate::compat::imsg::{IMSG_HEADER_SIZE, MAX_IMSGSIZE};
use crate::libc::{WEXITSTATUS, WIFEXITED, close, gettimeofday};
use crate::*;
use crate::sort::{SortCriteria, SortOrder};
use crate::options_::*;

pub unsafe fn server_redraw_client(c: *mut client) {
    unsafe {
        (*c).flags |= CLIENT_ALLREDRAWFLAGS;
    }
}

pub unsafe fn server_status_client(c: *mut client) {
    unsafe {
        (*c).flags |= client_flag::REDRAWSTATUS;
    }
}

pub unsafe fn server_redraw_session(s: *mut session) {
    unsafe {
        for c in clients_iter() {
            if client_get_session(c) == s {
                server_redraw_client(c);
            }
        }
    }
}

pub unsafe fn server_redraw_session_group(s: *mut session) {
    unsafe {
        match session_group_contains(s) {
            None => server_redraw_session(s),
            Some(name) => {
                for s in session_group_sessions(&name) {
                    server_redraw_session(s);
                }
            }
        }
    }
}

pub unsafe fn server_status_session(s: *mut session) {
    unsafe {
        for c in clients_iter() {
            if client_get_session(c) == s {
                server_status_client(c);
            }
        }
    }
}

pub unsafe fn server_status_session_group(s: *mut session) {
    unsafe {
        match session_group_contains(s) {
            None => server_status_session(s),
            Some(name) => {
                for s in session_group_sessions(&name) {
                    server_status_session(s);
                }
            }
        }
    }
}

pub unsafe fn server_redraw_window(w: *mut window) {
    unsafe {
        for c in clients_iter() {
            // A session can be without a current window (upstream 77ce0a83).
            let s = client_get_session(c);
            if !s.is_null()
                && let Some(curw) = (*s).curw()
                && winlink_window(curw) == w
            {
                server_redraw_client(c);
            }
        }
    }
}

pub unsafe fn server_redraw_window_borders(w: *mut window) {
    unsafe {
        for c in clients_iter() {
            // A session can be without a current window (upstream 77ce0a83).
            let s = client_get_session(c);
            if !s.is_null()
                && let Some(curw) = (*s).curw()
                && winlink_window(curw) == w
            {
                (*c).flags |= client_flag::REDRAWBORDERS;
            }
        }
    }
}

pub unsafe fn server_status_window(w: *mut window) {
    unsafe {
        // This is slightly different. We want to redraw the status line of any
        // clients containing this window rather than anywhere it is the
        // current window.

        for s in sessions_iter() {
            if session_has(s, &*w) {
                server_status_session(s);
            }
        }
    }
}

pub unsafe fn server_lock() {
    unsafe {
        for c in clients_iter() {
            if !client_get_session(c).is_null() {
                server_lock_client(c);
            }
        }
    }
}

pub unsafe fn server_lock_session(s: *mut session) {
    unsafe {
        for c in clients_iter() {
            if client_get_session(c) == s {
                server_lock_client(c);
            }
        }
    }
}

pub unsafe fn server_lock_client(c: *mut client) {
    unsafe {
        if (*c).flags.intersects(client_flag::CONTROL) {
            return;
        }

        if (*c).flags.intersects(client_flag::SUSPENDED) {
            return;
        }

        let cmd = options_get_string_((*client_get_session(c)).options, "lock-command");
        if *cmd == b'\0' || strlen(cmd) + 1 > MAX_IMSGSIZE - IMSG_HEADER_SIZE {
            return;
        }

        tty_stop_tty(&raw mut (*c).tty);
        tty_raw(
            &raw mut (*c).tty,
            tty_term_string((*c).tty.term, tty_code_code::TTYC_SMCUP),
        );
        tty_raw(
            &raw mut (*c).tty,
            tty_term_string((*c).tty.term, tty_code_code::TTYC_CLEAR),
        );
        tty_raw(
            &raw mut (*c).tty,
            tty_term_string((*c).tty.term, tty_code_code::TTYC_E3),
        );

        (*c).flags |= client_flag::SUSPENDED;
        proc_send(
            (*c).peer,
            msgtype::MSG_LOCK,
            -1,
            cmd.cast(),
            strlen(cmd) + 1,
        );
    }
}

pub unsafe fn server_kill_pane(wp: *mut window_pane) {
    unsafe {
        let w = window_pane_window(wp);

        if window_count_panes(&*w) == 1 {
            server_kill_window(w, 1);
            recalculate_sizes();
        } else {
            server_unzoom_window(w);
            layout_close_pane(wp);
            window_remove_pane(w, wp);
            server_redraw_window(w);
        }
    }
}

/// Removes window `w` from every session that has it, destroying sessions
/// left without windows. Holds a reference to the window meanwhile, as 3.8
/// does, so detaching its last winlink does not free it while the loop
/// still looks for more.
pub unsafe fn server_kill_window(w: *mut window, renumber: i32) {
    unsafe {
        window_add_ref(w, c!("server_kill_window"));
        let sessions: Vec<*mut session> =
            sessions_iter().collect();
        for s in sessions {
            if !session_has(s, &*w) {
                continue;
            }

            server_unzoom_window(w);
            while let Some(wl) = winlink_find_by_window((*s).sid(), WindowId((*w).id)) {
                if session_detach(s, wl.id) != 0 {
                    server_destroy_session_group(s);
                    break;
                }
                server_redraw_session_group(s);
            }

            if renumber != 0 {
                server_renumber_session(s);
            }
        }

        recalculate_sizes();
        window_remove_ref(w, c!("server_kill_window"));
    }
}

pub unsafe fn server_renumber_session(s: *mut session) {
    unsafe {
        if options_get_number_((*s).options, "renumber-windows") != 0 {
            match session_group_contains(s) {
                Some(name) => {
                    for s in session_group_sessions(&name) {
                        session_renumber_windows(s);
                    }
                }
                None => session_renumber_windows(s),
            }
        }
    }
}

pub unsafe fn server_renumber_all() {
    unsafe {
        for s in sessions_iter() {
            server_renumber_session(s);
        }
    }
}

pub unsafe fn server_link_window(
    src: *mut session,
    srcwl: Winlink,
    dst: *mut session,
    mut dstidx: i32,
    killflag: bool,
    mut selectflag: bool,
) -> Result<(), String> {
    unsafe {
        let srcsg = session_group_contains(src);
        let dstsg = session_group_contains(dst);
        if src != dst && srcsg.is_some() && srcsg == dstsg {
            return Err("sessions are grouped".to_string());
        }

        let mut dstwl = None;
        if dstidx != -1 {
            dstwl = winlink_find_by_index((*dst).sid(), dstidx);
        }
        if let Some(dstwl) = dstwl {
            if dstwl.window == srcwl.window {
                return Err(format!("same index: {dstidx}"));
            }
            if killflag {
                // Can't use session_detach as it will destroy session
                // if this makes it empty.
                events_fire_winlink("window-unlinked", dstwl);
                winlink_remove_flags(dstwl.id, WINLINK_ALERTFLAGS);

                // Force select/redraw if current.
                if Some(dstwl.id) == (*dst).curw_id() {
                    selectflag = true;
                }
                // This also takes it off the stack and leaves the session
                // without a current window if it was that one.
                winlink_remove(dstwl.id);
            }
        }

        if dstidx == -1 {
            dstidx = -1 - options_get_number_((*dst).options, "base-index") as i32;
        }
        let dstwl = session_attach(dst, winlink_window(srcwl), dstidx)?;

        if selectflag {
            session_select(dst, winlink_of(Some(dstwl)).idx);
        }
        server_redraw_session_group(dst);

        Ok(())
    }
}

pub unsafe fn server_unlink_window(s: *mut session, wl: WinlinkId) {
    unsafe {
        if session_detach(s, wl) != 0 {
            server_destroy_session_group(s);
        } else {
            server_redraw_session_group(s);
        }
    }
}

/// Fires pane-died or pane-exited, with how the pane's process ended.
///
/// # Safety
/// `wp` must be a valid pane in a valid window.
unsafe fn server_fire_pane_exit(name: &str, wp: *mut window_pane) {
    unsafe {
        let status = (*wp).status;
        let mut payload = crate::events::payload_for_pane(wp);
        if WIFEXITED(status) {
            payload.set_int("exit_status", WEXITSTATUS(status));
        } else if WIFSIGNALED(status) {
            // C's sig2name gives the number where there is no
            // sys_signame, as with glibc. pane_dead_signal does the same.
            payload.set_string("exit_signal", WTERMSIG(status).to_string());
        }
        payload.set_int("exit_success", (status == 0) as i32);
        crate::events::events_fire(name, payload, null_mut());
    }
}

pub unsafe fn server_destroy_pane(wp: *mut window_pane, notify: i32) {
    unsafe {
        let w = window_pane_window(wp);
        let mut ctx: MaybeUninit<screen_write_ctx> = MaybeUninit::<screen_write_ctx>::uninit();
        let ctx = ctx.as_mut_ptr();

        let mut gc: MaybeUninit<GridCell> = MaybeUninit::<GridCell>::uninit();
        let gc = gc.as_mut_ptr();

        let sx = screen_size_x(&raw mut (*wp).base);
        let sy = screen_size_y(&raw mut (*wp).base);

        if (*wp).fd != -1 {
            #[cfg(feature = "utempter")]
            {
                utempter_remove_record((*wp).fd);
            }
            (*wp).event_read = None;
            (*wp).event_write = None;
            close((*wp).fd);
            (*wp).fd = -1;
        }

        let remain_on_exit = options_get_number_((*wp).options, "remain-on-exit");
        if remain_on_exit != 0 && !(*wp).flags.intersects(window_pane_flags::PANE_STATUSREADY) {
            return;
        }
        'out: {
            match remain_on_exit {
                0 => (),
                1 | 2 => {
                    if remain_on_exit == 2
                        && WIFEXITED((*wp).status)
                        && WEXITSTATUS((*wp).status) == 0
                    {
                        break 'out;
                    }
                    if (*wp).flags.intersects(window_pane_flags::PANE_STATUSDRAWN) {
                        return;
                    }
                    (*wp).flags |= window_pane_flags::PANE_STATUSDRAWN;

                    gettimeofday(&raw mut (*wp).dead_time, null_mut());
                    if notify != 0 {
                        server_fire_pane_exit("pane-died", wp);
                    }

                    let s = options_get_string_((*wp).options, "remain-on-exit-format");
                    if *s != b'\0' {
                        screen_write_start_pane(ctx, wp, &raw mut (*wp).base);
                        screen_write_scrollregion(ctx, 0, sy - 1);
                        screen_write_cursormove(ctx, 0, sy as i32 - 1, 0);
                        screen_write_linefeed(ctx, true, 8);
                        memcpy_(gc, &raw const GRID_DEFAULT_CELL, size_of::<GridCell>());

                        let expanded =
                            format_single(null_mut(), cstr_to_str(s), null_mut(), null_mut(), None, wp);
                        format_draw(ctx, gc, sx, cstr_to_str(expanded), null_mut(), 0);
                        free_(expanded);

                        screen_write_stop(ctx);
                    }
                    (*wp).base.mode &= !mode_flag::MODE_CURSOR;

                    (*wp).flags |= window_pane_flags::PANE_REDRAW;
                    return;
                }
                _ => (),
            }
        }

        if notify != 0 {
            server_fire_pane_exit("pane-exited", wp);
        }

        server_unzoom_window(w);
        layout_close_pane(wp);
        window_remove_pane(w, wp);

        if (*w).panes.is_empty() {
            server_kill_window(w, 1);
        } else {
            server_redraw_window(w);
        }
    }
}

pub unsafe fn server_destroy_session_group(sess: *mut session) {
    unsafe {
        match session_group_contains(sess) {
            None => {
                server_destroy_session(sess);
                session_destroy(sess, 1, c!("server_destroy_session_group"));
            }
            // Collect first: session_destroy removes the session from the
            // group, and frees the group with its last session.
            Some(name) => for s in session_group_sessions(&name) {
                server_destroy_session(s);
                session_destroy(s, 1, c!("server_destroy_session_group"));
            }
        }
    }
}

pub unsafe fn server_find_session(
    s: *mut session,
    f: unsafe fn(*mut session, *mut session) -> i32,
) -> *mut session {
    unsafe {
        let mut s_out: *mut session = null_mut();
        for s_loop in sessions_iter() {
            if s_loop != s && f(s_loop, s_out) != 0 {
                s_out = s_loop;
            }
        }
        s_out
    }
}

/// Whether `s_loop` is a better choice than `s_out`, the best so far or
/// null: any session beats none, otherwise the most recently active wins.
pub unsafe fn server_newer_session(s_loop: *mut session, s_out: *mut session) -> i32 {
    unsafe {
        if s_out.is_null() {
            return 1;
        }
        (timer::new(&raw const (*s_loop).activity_time)
            > timer::new(&raw const (*s_out).activity_time)) as i32
    }
}

pub unsafe fn server_newer_detached_session(s_loop: *mut session, s_out: *mut session) -> i32 {
    unsafe {
        if (*s_loop).attached != 0 {
            return 0;
        }
        server_newer_session(s_loop, s_out)
    }
}

pub unsafe fn server_destroy_session(s: *mut session) {
    unsafe {
        let detach_on_destroy = options_get_number_((*s).options, "detach-on-destroy");
        let by_name = SortCriteria::new(Some(SortOrder::Name), false);

        let mut s_new: *mut session = if detach_on_destroy == 0 {
            server_find_session(s, server_newer_session)
        } else if detach_on_destroy == 2 {
            server_find_session(s, server_newer_detached_session)
        } else if detach_on_destroy == 3 {
            session_previous_session(s, &by_name)
        } else if detach_on_destroy == 4 {
            session_next_session(s, &by_name)
        } else {
            null_mut()
        };

        if s_new == s {
            s_new = null_mut();
        }
        for c in clients_iter() {
            if client_get_session(c) != s {
                continue;
            }
            (*c).session = None;
            (*c).last_session = None;
            server_client_set_session(c, s_new);
            if s_new.is_null() {
                (*c).flags |= client_flag::EXIT;
            }
        }
        recalculate_sizes();
    }
}

pub unsafe fn server_check_unattached() {
    unsafe {
        let sessions: Vec<*mut session> =
            sessions_iter().collect();
        for s in sessions {
            if (*s).attached != 0 {
                continue;
            }
            match options_get_number_((*s).options, "destroy-unattached") {
                0 => continue, // off
                1 => (),       // on
                2 => {
                    // keep-last
                    let sg = session_group_contains(s);
                    if sg.is_none_or(|name| session_group_count(&name) <= 1) {
                        continue;
                    }
                }
                3 => {
                    // keep-group
                    let sg = session_group_contains(s);
                    if sg.is_some_and(|name| session_group_count(&name) == 1) {
                        continue;
                    }
                }
                _ => (),
            }
            session_destroy(s, 1, c!("server_check_unattached"));
        }
    }
}

pub unsafe fn server_unzoom_window(w: *mut window) {
    unsafe {
        if window_unzoom(w, 1) == 0 {
            server_redraw_window(w);
        }
    }
}
