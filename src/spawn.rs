// Copyright (c) 2019 Nicholas Marriott <nicholas.marriott@gmail.com>
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

use crate::compat::{closefrom, fdforkpty::fdforkpty};
use crate::libc::{
    _exit, SIG_BLOCK, SIG_SETMASK, STDERR_FILENO, STDIN_FILENO, TCSANOW, VERASE, close, execl,
    execvp, sigfillset, sigprocmask, tcgetattr, tcsetattr,
};
#[cfg(feature = "utempter")]
use crate::utempter::utempter_add_record;
use crate::*;
use crate::options_::*;

pub unsafe fn spawn_log(from: &str, sc: *mut spawn_context) {
    unsafe {
        let s = (*sc).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        let wl = (*sc).wl;
        let wp0 = pane_ptr_from_id((*sc).wp0);
        let name = cmdq_get_name((*sc).item);
        type tmp_type = [u8; 128];
        let mut tmp = MaybeUninit::<tmp_type>::uninit();

        log_debug!("{}: {}, flags={:#x}", from, _s(name), (*sc).flags);

        if !wl.is_null() && !wp0.is_null() {
            _ = xsnprintf_!(
                tmp.as_mut_ptr().cast(),
                size_of::<tmp_type>(),
                "wl={} wp0=%{}",
                (*wl).idx,
                (*wp0).id,
            );
        } else if !wl.is_null() {
            _ = xsnprintf_!(
                tmp.as_mut_ptr().cast(),
                size_of::<tmp_type>(),
                "wl={} wp0=none",
                (*wl).idx,
            );
        } else if !wp0.is_null() {
            _ = xsnprintf_!(
                tmp.as_mut_ptr().cast(),
                size_of::<tmp_type>(),
                "wl=none wp0=%{}",
                (*wp0).id,
            );
        } else {
            _ = xsnprintf_!(
                tmp.as_mut_ptr().cast(),
                size_of::<tmp_type>(),
                "wl=none wp0=none",
            );
        }
        log_debug!(
            "{}: s=${} {} idx={}",
            from,
            (*s).id,
            _s(tmp.as_ptr().cast::<i8>()),
            (*sc).idx
        );
        log_debug!(
            "{}: name={}",
            from,
            _s(if (*sc).name.is_null() {
                c!("none")
            } else {
                (*sc).name
            }),
        );
    }
}

pub unsafe fn spawn_window(sc: *mut spawn_context) -> Result<NonNull<winlink>, String> {
    unsafe {
        let item = (*sc).item;
        let c = cmdq_get_client(item);
        let s = (*sc).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        let mut idx = (*sc).idx;
        let mut w: *mut window;
        spawn_log("spawn_window", sc);

        // If the window already exists, we are respawning, so destroy all the
        // panes except one.
        if (*sc).flags.intersects(spawn_flags::SPAWN_RESPAWN) {
            w = (*(*sc).wl).window.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
            if !(*sc).flags.intersects(SPAWN_KILL) {
                // Check if any pane is still alive (fd != -1).
                // In the C code, TAILQ_FOREACH left wp as NULL if no
                // match was found. Replicate that with find().
                let alive = (*w)
                    .panes
                    .iter()
                    .copied()
                    .any(|p| (*p).fd != -1);
                if alive {
                    return Err(format!("window {}:{} still active", (*s).name, (*(*sc).wl).idx));
                }
            }

            let wp0_new = (*w).panes.first().copied().unwrap_or(null_mut());
            (*sc).wp0 = pane_id_from_ptr(wp0_new);
            (*w).panes.retain(|&p| p != wp0_new);

            layout_free(w);
            window_destroy_panes(w);

            (*w).panes.insert(0, wp0_new);
            window_pane_resize(wp0_new, (*w).sx, (*w).sy);

            layout_init(w, wp0_new);
            (*w).active = None;
            window_set_active_pane(w, wp0_new, 0);
        }

        // Otherwise we have no window so we will need to create one. First
        // check if the given index already exists and destroy it if so.
        if !(*sc).flags.intersects(SPAWN_RESPAWN) && idx != -1 {
            let wl = winlink_find_by_index(&raw mut (*s).windows, idx);
            if !wl.is_null() && !(*sc).flags.intersects(SPAWN_KILL) {
                return Err(format!("index {idx} in use"));
            }
            if !wl.is_null() {
                // Can't use session_detach as it will destroy session
                // if this makes it empty.
                (*wl).flags &= !WINLINK_ALERTFLAGS;
                events_fire_winlink("window-unlinked", wl);
                winlink_stack_remove(&raw mut (*s).lastw, wl);
                winlink_remove(&raw mut (*s).windows, wl);

                if (*s).curw == wl {
                    (*s).curw = null_mut();
                    (*sc).flags &= !SPAWN_DETACHED;
                }
            }
        }

        // Then create a window if needed.
        if !(*sc).flags.intersects(SPAWN_RESPAWN) {
            if idx == -1 {
                idx = -1 - options_get_number_((*s).options, "base-index") as i32;
            }
            (*sc).wl = winlink_add(&raw mut (*s).windows, idx);
            if (*sc).wl.is_null() {
                return Err(format!("couldn't add window {idx}"));
            }
            let mut sx = 0u32;
            let mut sy = 0u32;
            let mut xpixel = 0u32;
            let mut ypixel = 0u32;
            default_window_size(
                (*sc).tc,
                s,
                null_mut(),
                &raw mut sx,
                &raw mut sy,
                &raw mut xpixel,
                &raw mut ypixel,
                None,
            );
            w = window_create(sx, sy, xpixel, ypixel);
            if w.is_null() {
                winlink_remove(&raw mut (*s).windows, (*sc).wl);
                return Err(format!("couldn't create window {idx}"));
            }
            if (*s).curw.is_null() {
                (*s).curw = (*sc).wl;
            }
            (*(*sc).wl).session = Some(SessionId((*s).id));
            (*w).latest = (*sc).tc.cast();
            winlink_set_window((*sc).wl, w);
        } else {
            w = null_mut();
        }
        (*sc).flags |= SPAWN_NONOTIFY;

        // Spawn the pane.
        if let Err(e) = spawn_pane(sc) {
            if !(*sc).flags.intersects(SPAWN_RESPAWN) {
                winlink_remove(&raw mut (*s).windows, (*sc).wl);
            }
            return Err(e);
        }

        // Set the name of the new window.
        if !(*sc).flags.intersects(SPAWN_RESPAWN) {
            if !(*sc).name.is_null() {
                let p = format_single(item, cstr_to_str((*sc).name), c, s, null_mut(), null_mut());
                (*w).name = Some(
                    std::ffi::CStr::from_ptr(p.cast())
                        .to_string_lossy()
                        .into_owned(),
                );
                free_(p);
                options_set_number((*w).options, "automatic-rename", 0);
            } else {
                (*w).name = Some(default_window_name(w));
            }
        }

        // Switch to the new window if required.
        if !(*sc).flags.intersects(SPAWN_DETACHED) {
            session_select(s, (*(*sc).wl).idx);
        }

        // Fire notification if new window.
        if !(*sc).flags.intersects(SPAWN_RESPAWN) {
            events_fire_window("window-created", w);
            events_fire_winlink("window-linked", (*sc).wl);
        }

        session_group_synchronize_from(s);
        Ok(NonNull::new((*sc).wl).unwrap())
    }
}

/// Fires pane-created, with the pane's command and directory and whether it
/// was created empty or respawned.
///
/// # Safety
/// `sc` must be a valid spawn context with a valid winlink, `wp` a valid
/// pane in a valid window.
unsafe fn spawn_fire_pane_created(sc: *mut spawn_context, wp: *mut window_pane) {
    unsafe {
        let wl = (*sc).wl;
        let mut fs: cmd_find_state = zeroed();
        cmd_find_from_winlink_pane(&raw mut fs, wl, wp, cmd_find_flags::empty());
        let mut payload = crate::events::Payload::new();
        payload.set_target(crate::events::payload_target(&raw const fs));
        if let Some(s) = (*sc).s.and_then(|id| session_from_id(id)) {
            payload.set("session", crate::events::payload_session(s));
        }
        if let Some(w) = (*wp).window.and_then(|id| window_from_id(id)) {
            payload.set("window", crate::events::payload_window(w));
        }
        payload.set_int("window_index", (*wl).idx);
        payload.set("pane", crate::events::payload_pane(wp));

        let cmd = if (*wp).argc != 0 {
            cmd_stringify_argv((*wp).argc, (*wp).argv)
        } else {
            String::new()
        };
        if !cmd.is_empty() {
            payload.set_string("pane_command", cmd);
        } else if let Some(shell) = &(*wp).shell {
            payload.set_string("pane_command", shell.to_string_lossy());
        }
        if let Some(cwd) = &(*wp).cwd {
            payload.set_string("pane_current_path", cwd.to_string_lossy());
        }
        payload.set_int("created_empty", (*sc).flags.intersects(SPAWN_EMPTY) as i32);
        payload.set_int("created_respawn", (*sc).flags.intersects(SPAWN_RESPAWN) as i32);
        crate::events::events_fire("pane-created", payload, null_mut());
    }
}

pub unsafe fn spawn_pane(sc: *mut spawn_context) -> Result<NonNull<window_pane>, String> {
    unsafe {
        let item = (*sc).item;
        let target = cmdq_get_target(item);
        let c = cmdq_get_client(item);
        let s = (*sc).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
        let w = (*(*sc).wl).window.and_then(|id| window_from_id(id)).unwrap_or(null_mut());
        let new_wp: *mut window_pane;
        let child: *mut Environ;
        let argv: *mut *mut u8;
        let argvp: *mut *mut u8;
        let argv0: *mut u8;
        let mut cwd: *mut u8;
        let new_cwd: *mut u8;
        let mut cmd: *const u8;
        let argc;
        let mut idx: u32 = 0;
        let mut now: libc::termios = zeroed();
        let hlimit: u32;
        let mut ws: libc::winsize = zeroed();
        let mut set: libc::sigset_t = zeroed();
        let mut oldset: libc::sigset_t = zeroed();
        let key: key_code;

        let wp0 = pane_ptr_from_id((*sc).wp0);
        'complete: {
            spawn_log("spawn_pane", sc);

            let target_s = (*target).s.and_then(|id| session_from_id(id)).unwrap_or(null_mut());
            if !(*sc).cwd.is_null() {
                cwd = format_single(item, cstr_to_str((*sc).cwd), c, target_s, null_mut(), null_mut());
                if *cwd != b'/' {
                    let base = server_client_get_cwd(c, target_s);
                    new_cwd = format_nul!("{}/{}", base.display(), _s(cwd));
                    free_(cwd);
                    cwd = new_cwd;
                }
            } else if !(*sc).flags.intersects(SPAWN_RESPAWN) {
                let base = server_client_get_cwd(c, target_s);
                let base_c = std::ffi::CString::new(base.to_string_lossy().as_bytes()).unwrap_or_default();
                cwd = xstrdup(base_c.as_ptr().cast()).as_ptr();
            } else {
                cwd = null_mut();
            }

            // If we are respawning then get rid of the old process. Otherwise
            // either create a new cell or assign to the one we are given.
            hlimit = options_get_number_((*s).options, "history-limit") as u32;
            if (*sc).flags.intersects(SPAWN_RESPAWN) {
                if (*wp0).fd != -1 && !(*sc).flags.intersects(SPAWN_KILL) {
                    window_pane_index(wp0, &raw mut idx);
                    let msg = format!(
                        "pane {}:{}.{} still active",
                        (*s).name,
                        (*(*sc).wl).idx,
                        idx
                    );
                    free_(cwd);
                    return Err(msg);
                }
                if (*wp0).fd != -1 {
                    (*wp0).event_read = None;
                    (*wp0).event_write = None;
                    close((*wp0).fd);
                }
                window_pane_reset_mode_all(wp0);
                screen_reinit(&raw mut (*wp0).base);
                input_free((*wp0).ictx);
                (*wp0).ictx = null_mut();
                new_wp = wp0;
                (*new_wp).flags &=
                    !(window_pane_flags::PANE_STATUSREADY | window_pane_flags::PANE_STATUSDRAWN);
            } else if (*sc).lc.is_null() {
                new_wp = window_add_pane(w, null_mut(), hlimit, (*sc).flags);
                layout_init(w, new_wp);
            } else {
                new_wp = window_add_pane(w, wp0, hlimit, (*sc).flags);
                if (*sc).flags.intersects(SPAWN_ZOOM) {
                    layout_assign_pane((*sc).lc, new_wp, 1);
                } else {
                    layout_assign_pane((*sc).lc, new_wp, 0);
                }
            }

            // Now we have a pane with nothing running in it ready for the new
            // process. Work out the command and arguments and store the working
            // directory.
            if (*sc).argc == 0 && !(*sc).flags.intersects(SPAWN_RESPAWN) {
                cmd = options_get_string_((*s).options, "default-command");
                if !cmd.is_null() && *cmd != b'\0' {
                    argc = 1;
                    argv = &raw mut cmd as *mut *mut u8;
                } else {
                    argc = 0;
                    argv = null_mut();
                }
            } else {
                argc = (*sc).argc;
                argv = (*sc).argv;
            }
            if !cwd.is_null() {
                (*new_wp).cwd = Some(PathBuf::from(
                    std::ffi::CStr::from_ptr(cwd.cast())
                        .to_string_lossy()
                        .into_owned(),
                ));
                free_(cwd);
            }

            // Replace the stored arguments if there are new ones. If not, the
            // existing ones will be used (they will only exist for respawn).
            if argc > 0 {
                cmd_free_argv((*new_wp).argc, (*new_wp).argv);
                (*new_wp).argc = argc;
                (*new_wp).argv = cmd_copy_argv(argc, argv);
            }

            // Create an environment for this pane.
            child = environ_for_session(s, 0);
            if !(*sc).environ.is_null() {
                environ_copy(&*(*sc).environ, &mut *child);
            }
            environ_set!(
                child,
                c!("TMUX_PANE"),
                environ_flags::empty(),
                "%{}",
                (*new_wp).id,
            );

            // Then the PATH environment variable. The session one is replaced from
            // the client if there is one because otherwise running "tmux new
            // myprogram" wouldn't work if myprogram isn't in the session's path.
            if !c.is_null() && client_get_session(c).is_null() {
                // only unattached clients
                if let Some(ee) = environ_find_raw(&(*c).environ, c!("PATH"))
                    && let Some(ref value) = ee.value {
                        environ_set_(&mut *child, "PATH", environ_flags::empty(), value.clone());
                    }
            }
            if environ_find_raw(&*child, c!("PATH")).is_none() {
                environ_set!(
                    child,
                    c!("PATH"),
                    environ_flags::empty(),
                    "{}",
                    _s(_PATH_DEFPATH)
                );
            }

            // Then the shell. If respawning, use the old one.
            if !(*sc).flags.intersects(SPAWN_RESPAWN) {
                let mut tmp = options_get_string_((*s).options, "default-shell");
                if !checkshell_(tmp) {
                    tmp = _PATH_BSHELL;
                }
                (*new_wp).shell = Some(PathBuf::from(
                    std::ffi::CStr::from_ptr(tmp.cast())
                        .to_string_lossy()
                        .into_owned(),
                ));
            }
            let shell_display = (*new_wp).shell.as_deref().map(|p| p.display().to_string()).unwrap_or_default();
            environ_set!(
                child,
                c!("SHELL"),
                environ_flags::empty(),
                "{}",
                shell_display
            );

            // Log the arguments we are going to use.
            log_debug!("spawn_pane: shell={}", shell_display);
            if (*new_wp).argc != 0 {
                let cp = cmd_stringify_argv((*new_wp).argc, (*new_wp).argv);
                log_debug!("spawn_pane: cmd={}", cp);
            }
            log_debug!("spawn_pane: cwd={}", (*new_wp).cwd.as_deref().map(|p| p.display().to_string()).unwrap_or_default());
            cmd_log_argv!((*new_wp).argc, (*new_wp).argv, "spawn_pan");
            environ_log!(child, "spawn_pan: environment ");

            // Initialize the window size.
            memset0(&raw mut ws);
            ws.ws_col = screen_size_x(&raw mut (*new_wp).base) as u16;
            ws.ws_row = screen_size_y(&raw mut (*new_wp).base) as u16;
            ws.ws_xpixel = ((*w).xpixel * ws.ws_col as u32) as u16;
            ws.ws_ypixel = ((*w).ypixel * ws.ws_row as u32) as u16;

            // Block signals until fork has completed.
            sigfillset(&raw mut set);
            sigprocmask(SIG_BLOCK, &raw mut set, &raw mut oldset);

            // If the command is empty, don't fork a child process.
            if (*sc).flags.intersects(SPAWN_EMPTY) {
                (*new_wp).flags |= window_pane_flags::PANE_EMPTY;
                (*new_wp).base.mode &= !mode_flag::MODE_CURSOR;
                (*new_wp).base.mode |= mode_flag::MODE_CRLF;
                break 'complete;
            }

            // Change to the new working directory before forking and back
            // in the parent afterwards. Changing it in the child would race
            // with pane_current_path reading the child's directory. Like
            // C, do not move if the current directory cannot be found,
            // since there would be nothing to go back to.
            let home = find_home().and_then(|h| h.to_str().ok());
            let old_cwd = std::env::current_dir().ok();
            let actual_cwd: Option<PathBuf> = old_cwd.as_ref().and_then(|_| {
                [(*new_wp).cwd.clone(), home.map(PathBuf::from), Some(PathBuf::from("/"))]
                    .into_iter()
                    .flatten()
                    .find(|dir| std::env::set_current_dir(dir).is_ok())
            });
            let restore_cwd = || {
                if let (Some(_), Some(old)) = (&actual_cwd, &old_cwd)
                    && std::env::set_current_dir(old).is_err()
                    && home.is_none_or(|h| std::env::set_current_dir(h).is_err())
                {
                    let _ = std::env::set_current_dir("/");
                }
            };

            // Fork the new process.
            (*new_wp).pid = fdforkpty(
                ptm_fd(),
                &raw mut (*new_wp).fd,
                (*new_wp).tty.as_mut_ptr(),
                null_mut(),
                &raw mut ws,
            );
            if (*new_wp).pid == -1 {
                let msg = format!("fork failed: {}", strerror(errno!()));
                restore_cwd();
                (*new_wp).fd = -1;
                if !(*sc).flags.intersects(SPAWN_RESPAWN) {
                    layout_close_pane(new_wp);
                    window_remove_pane(w, new_wp);
                }
                sigprocmask(SIG_SETMASK, &raw mut oldset, null_mut());
                environ_free(child);
                return Err(msg);
            }

            // In the parent process, everything is done now. Change the
            // working directory back.
            if (*new_wp).pid != 0 {
                restore_cwd();
                #[cfg(all(feature = "systemd", feature = "cgroups"))]
                {
                    // Move the child process into a new cgroup for systemd-oomd
                    // isolation.
                    let mut cg_cause: *mut u8 = null_mut();
                    if (systemd_move_pid_to_new_cgroup((*new_wp).pid, &raw mut cg_cause) < 0) {
                        log_debug!(
                            "{}: moving pane to new cgroup failed: {}",
                            _s(__func__),
                            _s(cg_cause)
                        );
                        free_(cg_cause);
                    }
                }
                break 'complete;
            }

            // Child process. Set PWD to the working directory.
            if let Some(dir) = &actual_cwd {
                environ_set!(child, c!("PWD"), environ_flags::empty(), "{}", dir.display());
            }

            // Update terminal escape characters from the session if available and
            // force VERASE to tmux's backspace.
            if tcgetattr(STDIN_FILENO, &raw mut now) != 0 {
                _exit(1);
            }
            if let Some(ref tio) = (*s).tio {
                memcpy__(now.c_cc.as_mut_ptr(), tio.c_cc.as_ptr());
            }
            key = options_get_number_(GLOBAL_OPTIONS, "backspace") as u64;
            if key >= 0x7f {
                now.c_cc[VERASE] = b'\x7f';
            } else {
                now.c_cc[VERASE] = key as u8;
            }
            #[cfg(feature = "iutf8")]
            {
                now.c_iflag |= IUTF8;
            }
            if tcsetattr(STDIN_FILENO, TCSANOW, &now) != 0 {
                _exit(1);
            }

            // Clean up file descriptors and signals and update the environment.
            proc_clear_signals(SERVER_PROC, 1);
            closefrom(STDERR_FILENO + 1);
            sigprocmask(SIG_SETMASK, &raw mut oldset, null_mut());
            proc_unblock_signals();
            log_close();
            environ_push(&*child);

            // If given multiple arguments, use execvp(). Copy the arguments to
            // ensure they end in a NULL.
            if (*new_wp).argc != 0 && (*new_wp).argc != 1 {
                argvp = cmd_copy_argv((*new_wp).argc, (*new_wp).argv);
                execvp((*argvp).cast(), argvp.cast());
                _exit(1);
            }

            // If one argument, pass it to $SHELL -c. Otherwise create a login
            // shell.
            let shell_path = (*new_wp).shell.as_deref().unwrap();
            let shell_str = shell_path.to_string_lossy();
            let shell_c = std::ffi::CString::new(shell_str.as_bytes()).unwrap();
            let basename = shell_path
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| shell_str.into_owned());
            if (*new_wp).argc == 1 {
                let tmp = *(*new_wp).argv;
                argv0 = format_nul!("{}", basename);
                execl(
                    shell_c.as_ptr().cast(),
                    argv0.cast(),
                    c!("-c"),
                    tmp,
                    null_mut::<u8>(),
                );
                _exit(1);
            }
            argv0 = format_nul!("-{}", basename);
            execl(shell_c.as_ptr().cast(), argv0.cast(), null_mut::<u8>());
            _exit(1);
        }

        // complete:
        #[cfg(feature = "utempter")]
        {
            if !(*new_wp).flags.intersects(window_pane_flags::PANE_EMPTY) {
                let cp = format_nul!("tmux({}).%{}", std::process::id() as c_long, (*new_wp).id);
                utempter_add_record((*new_wp).fd, cp);
                kill(std::process::id() as i32, SIGCHLD);
                free_(cp);
            }
        }

        (*new_wp).flags &= !window_pane_flags::PANE_EXITED;

        sigprocmask(SIG_SETMASK, &raw mut oldset, null_mut());
        window_pane_set_event(new_wp);

        environ_free(child);
        spawn_fire_pane_created(sc, new_wp);

        if (*sc).flags.intersects(SPAWN_RESPAWN) {
            return Ok(NonNull::new(new_wp).unwrap());
        }
        if !(*sc).flags.intersects(SPAWN_DETACHED) || (*w).active.is_none() {
            if (*sc).flags.intersects(SPAWN_NONOTIFY) {
                window_set_active_pane(w, new_wp, 0);
            } else {
                window_set_active_pane(w, new_wp, 1);
            }
        }
        if !(*sc).flags.intersects(SPAWN_NONOTIFY) {
            events_fire_window("window-layout-changed", w);
        }

        Ok(NonNull::new(new_wp).unwrap())
    }
}
