// Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
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
    AF_UNIX, O_RDWR, PF_UNSPEC, SHUT_WR, SIG_BLOCK, SIG_SETMASK, SIGCONT, SIGTERM, SIGTTIN,
    SIGTTOU, SOCK_STREAM, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO, TIOCSWINSZ, WIFSTOPPED,
    WSTOPSIG, close, dup2, execl, execvp, fork, ioctl, kill, killpg, memset, open, shutdown,
    sigfillset, sigprocmask, sigset_t, socketpair, winsize,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::os::unix::ffi::OsStrExt;

use crate::*;
use crate::options_::{options, options_get_string_};

mod info;
pub use self::info::{JobId, JobInfo, JobStep};

pub type job_update_cb = Option<unsafe fn(JobId)>;
pub type job_complete_cb = Option<unsafe fn(JobId)>;
pub type job_free_cb = Option<unsafe fn(*mut c_void)>;

/// A running job: its bookkeeping, and the descriptor, buffers, event
/// registrations and callbacks the adapter runs it with.
pub struct job {
    pub info: JobInfo,

    pub flags: job_flag,
    pub tty: [u8; TTY_NAME_MAX],

    /// Buffered data read from the subprocess fd.
    pub event_input: tmux_event::Evbuffer,
    /// Buffered data to write to the subprocess fd.
    pub event_output: tmux_event::Evbuffer,
    /// Calloop read registration for the subprocess fd.
    pub event_read: Option<IoHandle>,
    /// Calloop write registration for the subprocess fd.
    pub event_write: Option<IoHandle>,

    pub updatecb: job_update_cb,
    pub completecb: job_complete_cb,
    pub freecb: job_free_cb,
    pub data: *mut c_void,
}

thread_local! {
    /// Every job by ID, and the last ID used. Borrowed only to look up,
    /// add or take out a job, never across a call out. The jobs are boxed,
    /// so a pointer to one stays valid while it is in the map.
    static JOBS: RefCell<(BTreeMap<JobId, Box<job>>, u32)> = const { RefCell::new((BTreeMap::new(), 0)) };
}

/// The job with ID `id`, valid until it is freed.
fn job_ptr(id: JobId) -> Option<*mut job> {
    JOBS.with_borrow_mut(|(jobs, _)| jobs.get_mut(&id).map(|j| &raw mut **j))
}

/// Every job, newest first as in C.
fn job_ids() -> Vec<JobId> {
    JOBS.with_borrow(|(jobs, _)| jobs.keys().rev().copied().collect())
}

/// Takes job `id` out of the list, for freeing.
fn job_take(id: JobId) -> Option<Box<job>> {
    JOBS.with_borrow_mut(|(jobs, _)| jobs.remove(&id))
}

pub unsafe fn job_run(
    cmd: *const u8,
    argc: c_int,
    argv: *mut *mut u8,
    e: Option<&Environ>,
    s: *mut session,
    cwd: *const u8,
    updatecb: job_update_cb,
    completecb: job_complete_cb,
    freecb: job_free_cb,
    data: *mut c_void,
    flags: job_flag,
    sx: c_int,
    sy: c_int,
) -> Option<JobId> {
    let __func__ = "job_run";
    unsafe {
        let mut env: Environ;
        let pid: pid_t;
        let nullfd: i32;
        let mut out: [i32; 2] = [0; 2];
        let mut master: i32 = 0;
        let mut shell: *const u8;
        let mut set = MaybeUninit::<sigset_t>::uninit();
        let mut oldset = MaybeUninit::<sigset_t>::uninit();
        let mut ws = MaybeUninit::<winsize>::uninit();
        let argvp: *mut *mut u8;
        // let mut tty = MaybeUninit::<[c_char; TTY_NAME_MAX]>::uninit();
        let mut tty = [0i8; 64];
        let argv0: *mut u8;
        let oo: *mut options;

        'fail: {
            env = environ_for_session(s, !CFG_FINISHED.load(atomic::Ordering::Acquire));
            if let Some(e) = e {
                env.copy_from(e);
            }

            if !flags.intersects(job_flag::JOB_DEFAULTSHELL) {
                shell = _PATH_BSHELL;
            } else {
                if !s.is_null() {
                    oo = (*s).options;
                } else {
                    oo = GLOBAL_S_OPTIONS;
                }
                shell = options_get_string_(oo, "default-shell");
                if !checkshell_(shell) {
                    shell = _PATH_BSHELL;
                }
            }
            argv0 = shell_argv0(shell, 0);

            sigfillset(set.as_mut_ptr());
            sigprocmask(SIG_BLOCK, set.as_mut_ptr(), oldset.as_mut_ptr());

            if flags.intersects(job_flag::JOB_PTY) {
                memset(ws.as_mut_ptr().cast(), 0, size_of::<winsize>());
                (*ws.as_mut_ptr()).ws_col = sx as u16;
                (*ws.as_mut_ptr()).ws_row = sy as u16;
                pid = fdforkpty(
                    ptm_fd(),
                    &raw mut master,
                    (&raw mut tty) as *mut u8,
                    null_mut(),
                    ws.as_mut_ptr(),
                );
            } else {
                if socketpair(AF_UNIX, SOCK_STREAM, PF_UNSPEC, &raw mut out as *mut c_int) != 0 {
                    break 'fail;
                }
                pid = fork();
            }

            if cmd.is_null() {
                cmd_log_argv!(argc, argv, "{__func__}");
                log_debug!(
                    "{} cwd={} shell={}",
                    __func__,
                    _s(if cwd.is_null() { c!("") } else { cwd }),
                    _s(shell),
                );
            } else {
                log_debug!(
                    "{} cmd={} cwd={} shell={}",
                    __func__,
                    _s(cmd),
                    _s(if cwd.is_null() { c!("") } else { cwd }),
                    _s(shell),
                );
            }

            match pid {
                -1 => {
                    if !flags.intersects(job_flag::JOB_PTY) {
                        close(out[0]);
                        close(out[1]);
                    }
                    break 'fail;
                }
                0 => {
                    proc_clear_signals(SERVER_PROC, 1);
                    sigprocmask(SIG_SETMASK, oldset.as_mut_ptr(), null_mut());
                    proc_unblock_signals();

                    // Without a cwd the job stays in the server's directory.
                    // Otherwise try cwd, then home, then the root, and tell
                    // the job where it ended up in PWD.
                    if !cwd.is_null() {
                        let cwd = std::ffi::OsStr::from_bytes(CStr::from_ptr(cwd.cast()).to_bytes());
                        let home = find_home().map(|h| std::path::Path::new(std::ffi::OsStr::from_bytes(h.to_bytes())));
                        let dir = [Some(std::path::Path::new(cwd)), home, Some(std::path::Path::new("/"))]
                            .into_iter()
                            .flatten()
                            .find(|dir| std::env::set_current_dir(dir).is_ok());
                        match dir {
                            Some(dir) => env.set(b"PWD", dir.as_os_str().as_bytes(), EnvironFlags::empty()),
                            None => libc::_exit(1),
                        }
                    }

                    environ_push(&env);

                    if !flags.intersects(job_flag::JOB_PTY) {
                        if dup2(out[1], STDIN_FILENO) == -1 {
                            libc::_exit(1);
                        }
                        if dup2(out[1], STDOUT_FILENO) == -1 {
                            libc::_exit(1);
                        }
                        if out[1] != STDIN_FILENO && out[1] != STDOUT_FILENO {
                            close(out[1]);
                        }
                        close(out[0]);

                        nullfd = open(_PATH_DEVNULL, O_RDWR, 0);
                        if nullfd == -1 {
                            libc::_exit(1);
                        }
                        if dup2(nullfd, STDERR_FILENO) == -1 {
                            libc::_exit(1);
                        }
                        if nullfd != STDERR_FILENO {
                            close(nullfd);
                        }
                    }
                    closefrom(STDERR_FILENO + 1);

                    if !cmd.is_null() {
                        // Only popups use default-shell, and only they say so
                        // in SHELL.
                        if flags.intersects(job_flag::JOB_DEFAULTSHELL) {
                            std::env::set_var(
                                "SHELL",
                                std::ffi::OsStr::from_bytes(CStr::from_ptr(shell.cast()).to_bytes()),
                            );
                        }
                        execl(
                            shell.cast(),
                            argv0.cast(),
                            c!("-c"),
                            cmd,
                            null_mut::<c_void>(),
                        );
                        libc::_exit(1);
                    } else {
                        argvp = cmd_copy_argv(argc, argv);
                        execvp((*argvp).cast(), argvp.cast());
                        libc::_exit(1);
                    }
                }
                _ => (),
            }

            sigprocmask(SIG_SETMASK, oldset.as_ptr(), null_mut());
            free_(argv0);

            let cmd_bytes = if cmd.is_null() {
                cmd_stringify_argv(argc, argv).into_bytes()
            } else {
                CStr::from_ptr(cmd.cast()).to_bytes().to_vec()
            };
            let fd = if flags.intersects(job_flag::JOB_PTY) {
                master
            } else {
                close(out[1]);
                out[0]
            };
            setblocking(fd, 0);

            let id = JOBS.with_borrow_mut(|(_, last)| {
                *last += 1;
                JobId(*last)
            });
            let mut info = JobInfo::new(id, cmd_bytes, pid, flags.intersects(job_flag::JOB_NOWAIT));
            info.fd = fd;
            let mut new = Box::new(job {
                info,
                flags,
                tty: [0; TTY_NAME_MAX],
                event_input: tmux_event::Evbuffer::default(),
                event_output: tmux_event::Evbuffer::default(),
                event_read: None,
                event_write: None,
                updatecb,
                completecb,
                freecb,
                data,
            });
            strlcpy(new.tty.as_mut_ptr(), tty.as_ptr().cast(), TTY_NAME_MAX);
            new.event_read = io_register(fd, EV_READ, Box::new(move |_fd, _events| job_read_fire(id)));
            new.event_write = io_register(fd, EV_WRITE, Box::new(move |_fd, _events| job_write_fire(id)));
            log_debug!(
                "run job {}: {} pid {}",
                id.0,
                String::from_utf8_lossy(&new.info.cmd),
                pid
            );
            JOBS.with_borrow_mut(|(jobs, _)| jobs.insert(id, new));
            return Some(id);
        }

        sigprocmask(SIG_SETMASK, oldset.as_ptr(), null_mut());
        free_(argv0);
        None
    }
}

/// Lets go of job `id` without killing it: its pid and tty go to the
/// caller, which also gets its descriptor (`job_transfer`).
pub unsafe fn job_transfer(id: JobId, pid: *mut pid_t, tty: *mut u8, ttylen: usize) -> c_int {
    unsafe {
        let Some(mut job) = job_take(id) else {
            return -1;
        };
        log_debug!("transfer job {}: {}", id.0, String::from_utf8_lossy(&job.info.cmd));
        if !pid.is_null() {
            *pid = job.info.pid;
        }
        if !tty.is_null() {
            strlcpy(tty, job.tty.as_mut_ptr(), ttylen);
        }
        if let Some(freecb) = job.freecb
            && !job.data.is_null()
        {
            freecb(job.data);
        }
        job.info.fd
    }
}

/// Frees job `id`: its owner's data, the process (SIGTERM) and the
/// descriptor. A job already gone is ignored.
pub unsafe fn job_free(id: JobId) {
    unsafe {
        let Some(job) = job_take(id) else {
            return;
        };
        log_debug!("free job {}: {}", id.0, String::from_utf8_lossy(&job.info.cmd));
        if let Some(freecb) = job.freecb
            && !job.data.is_null()
        {
            freecb(job.data);
        }
        if job.info.pid != -1 {
            kill(job.info.pid, SIGTERM);
        }
        let fd = job.info.fd;
        // The event registrations go before the descriptor closes.
        drop(job);
        if fd != -1 {
            close(fd);
        }
    }
}

/// Tells a job on a pty its new size.
pub unsafe fn job_resize(id: JobId, sx: c_uint, sy: c_uint) {
    unsafe {
        let Some(job) = job_ptr(id) else {
            return;
        };
        if (*job).info.fd == -1 || !(*job).flags.intersects(job_flag::JOB_PTY) {
            return;
        }
        log_debug!("resize job {}: {}x{}", id.0, sx, sy);
        let mut ws: winsize = zeroed();
        ws.ws_col = sx as u16;
        ws.ws_row = sy as u16;
        if ioctl((*job).info.fd, TIOCSWINSZ, &raw mut ws) == -1 {
            fatal("ioctl failed");
        }
    }
}

/// Read callback: reads from the job's descriptor into its input buffer
/// and runs the update callback.
unsafe fn job_read_fire(id: JobId) {
    unsafe {
        let Some(job) = job_ptr(id) else {
            return;
        };
        let n = (*job).event_input.read_from_fd((*job).info.fd, 4096);
        if n > 0 {
            if let Some(updatecb) = (*job).updatecb {
                updatecb(id);
            }
        } else if n == 0
            || (n < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock)
        {
            job_error_fire(id);
        }
    }
}

/// Write callback: writes the job's output buffer to its descriptor.
unsafe fn job_write_fire(id: JobId) {
    unsafe {
        let Some(job) = job_ptr(id) else {
            return;
        };
        if !(*job).event_output.is_empty() {
            let n = (*job).event_output.write_to_fd((*job).info.fd);
            if n < 0 {
                if std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock {
                    return;
                }
                job_error_fire(id);
                return;
            }
        }
        let len = (*job).event_output.len();
        log_debug!(
            "job write {}: {}, pid {}, output left {}",
            id.0,
            String::from_utf8_lossy(&(*job).info.cmd),
            (*job).info.pid,
            len,
        );
        if len == 0 && !(*job).flags.intersects(job_flag::JOB_KEEPWRITE) {
            shutdown((*job).info.fd, SHUT_WR);
            (*job).event_write = None;
        }
    }
}

/// The job's descriptor closed or failed. Completes the job if its process
/// has exited too.
unsafe fn job_error_fire(id: JobId) {
    unsafe {
        let Some(job) = job_ptr(id) else {
            return;
        };
        log_debug!(
            "job error {}: {}, pid {}",
            id.0,
            String::from_utf8_lossy(&(*job).info.cmd),
            (*job).info.pid
        );
        match (*job).info.closed() {
            JobStep::Complete => {
                if let Some(completecb) = (*job).completecb {
                    completecb(id);
                }
                job_free(id);
            }
            JobStep::Wait => (*job).event_read = None,
        }
    }
}

/// A child process changed state (SIGCHLD). Continues a job stopped for
/// another reason than terminal access, otherwise records its exit and
/// completes it if its descriptor is closed too.
pub unsafe fn job_check_died(pid: pid_t, status: i32) {
    unsafe {
        let Some(id) = JOBS.with_borrow(|(jobs, _)| {
            jobs.values().find(|j| j.info.pid == pid).map(|j| j.info.id)
        }) else {
            return;
        };
        let Some(job) = job_ptr(id) else {
            return;
        };
        if WIFSTOPPED(status) {
            if WSTOPSIG(status) == SIGTTIN || WSTOPSIG(status) == SIGTTOU {
                return;
            }
            killpg((*job).info.pid, SIGCONT);
            return;
        }
        log_debug!(
            "job died {}: {} pid {}",
            id.0,
            String::from_utf8_lossy(&(*job).info.cmd),
            pid as c_long
        );
        if (*job).info.exited(status) == JobStep::Complete {
            if let Some(completecb) = (*job).completecb {
                completecb(id);
            }
            job_free(id);
        }
    }
}

/// The wait status of job `id`'s process, 0 for a job that has gone.
pub fn job_get_status(id: JobId) -> i32 {
    // SAFETY: a pointer from job_ptr is valid while the job is listed,
    // and nothing runs between the lookup and the read.
    job_ptr(id).map_or(0, |j| unsafe { (*j).info.status })
}

/// The owner's data of job `id`.
pub fn job_get_data(id: JobId) -> *mut c_void {
    // SAFETY: as for job_get_status.
    job_ptr(id).map_or(null_mut(), |j| unsafe { (*j).data })
}

/// What job `id` has written and nobody read yet, valid while the job
/// lives.
pub fn job_get_input(id: JobId) -> *mut tmux_event::Evbuffer {
    // SAFETY: as for job_get_status. The buffer is inside the boxed job.
    job_ptr(id).map_or(null_mut(), |j| unsafe { &raw mut (*j).event_input })
}

/// What is waiting to be written to job `id`, valid while the job lives.
pub fn job_get_output(id: JobId) -> *mut tmux_event::Evbuffer {
    // SAFETY: as for job_get_input.
    job_ptr(id).map_or(null_mut(), |j| unsafe { &raw mut (*j).event_output })
}

/// Writes `data` to job `id`'s process.
pub unsafe fn job_write(id: JobId, data: *const c_void, size: usize) {
    unsafe {
        let Some(job) = job_ptr(id) else {
            return;
        };
        let slice = std::slice::from_raw_parts(data.cast::<u8>(), size);
        (*job).event_output.add(slice);
        if (*job).event_write.is_none() && (*job).info.fd >= 0 {
            (*job).event_write = io_register(
                (*job).info.fd,
                EV_WRITE,
                Box::new(move |_fd, _events| job_write_fire(id)),
            );
        }
    }
}

/// Sends SIGTERM to every job's process (server exit).
pub unsafe fn job_kill_all() {
    let pids: Vec<pid_t> = JOBS.with_borrow(|(jobs, _)| jobs.values().map(|j| j.info.pid).collect());
    for pid in pids {
        if pid != -1 {
            // SAFETY: kill only sends a signal.
            unsafe { kill(pid, SIGTERM) };
        }
    }
}

/// Whether a job the server waits for is still running.
pub fn job_still_running() -> bool {
    JOBS.with_borrow(|(jobs, _)| jobs.values().any(|j| j.info.keeps_server()))
}

/// The lines of show-messages -J, newest job first.
pub unsafe fn job_print_summary(item: *mut cmdq_item, mut blank: i32) {
    unsafe {
        let lines: Vec<Vec<u8>> = job_ids()
            .into_iter()
            .enumerate()
            .filter_map(|(n, id)| job_ptr(id).map(|j| (*j).info.summary(n)))
            .collect();
        for line in lines {
            if blank != 0 {
                cmdq_print!(item, "");
                blank = 0;
            }
            cmdq_print!(item, "{}", String::from_utf8_lossy(&line));
        }
    }
}
