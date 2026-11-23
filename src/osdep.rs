// Copyright (c) 2009 Nicholas Marriott <nicholas.marriott@gmail.com>
// Copyright (c) 2009 Joshua Elsasser <josh@elsasser.org>
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
use crate::libc;
use crate::*;

#[cfg(target_os = "linux")]
pub unsafe fn osdep_get_name(fd: i32, _tty: *const u8) -> *mut u8 {
    unsafe {
        let pgrp = libc::tcgetpgrp(fd);
        if pgrp == -1 {
            return null_mut();
        }

        let path = format_nul!("/proc/{pgrp}/cmdline");
        let f = fopen(path, c!("r"));
        if f.is_null() {
            free_(path);
            return null_mut();
        }
        free_(path);

        let mut len = 0;
        let mut buf: *mut u8 = null_mut();

        loop {
            let ch = libc::fgetc(f);
            if ch == libc::EOF {
                break;
            }
            if ch == b'\0' as i32 {
                break;
            }
            buf = xrealloc_(buf, len + 2).as_ptr();
            *buf.add(len) = ch as u8;
            len += 1;
        }
        if !buf.is_null() {
            *buf.add(len) = b'\0';
        }

        fclose(f);
        buf
    }
}

/// The working directory of the foreground process group of the terminal
/// `fd`, falling back to the session leader.
#[cfg(target_os = "linux")]
pub fn osdep_get_cwd(fd: i32) -> Option<std::path::PathBuf> {
    // SAFETY: tcgetpgrp only queries the descriptor.
    let pgrp = unsafe { libc::tcgetpgrp(fd) };
    if pgrp == -1 {
        return None;
    }
    if let Ok(cwd) = std::fs::read_link(format!("/proc/{pgrp}/cwd")) {
        return Some(cwd);
    }

    let mut sid: pid_t = 0;
    // SAFETY: TIOCGSID writes one pid_t to the pointer, which is valid.
    if unsafe { libc::ioctl(fd, libc::TIOCGSID, &raw mut sid) } != -1 {
        return std::fs::read_link(format!("/proc/{sid}/cwd")).ok();
    }
    None
}

#[cfg(target_os = "linux")]
pub unsafe fn osdep_event_init() -> *mut event_base {
    unsafe {
        // On Linux, epoll doesn't work on /dev/null (yes, really).
        std::env::set_var("EVENT_NOEPOLL", "1");

        let base = event_init();

        std::env::remove_var("EVENT_NOEPOLL");

        base
    }
}

// osdep darwin

#[cfg(target_os = "macos")]
pub unsafe fn osdep_get_name(fd: i32, _tty: *const u8) -> *mut u8 {
    // note only bothering to port the version for > Mac OS X 10.7 SDK or later
    unsafe {
        use libc::proc_pidinfo;

        let mut bsdinfo: proc_bsdshortinfo = zeroed();
        let pgrp: pid_t = libc::tcgetpgrp(fd);
        if pgrp == -1 {
            return null_mut();
        }

        const PROC_PIDT_SHORTBSDINFO: i32 = 13;
        // abi compatible version of struct defined in sys/proc_info.h
        #[repr(C)]
        struct proc_bsdshortinfo {
            padding1: [u32; 4],
            pbsi_comm: [u8; 16],
            padding2: [u32; 8],
        }

        let ret = proc_pidinfo(
            pgrp,
            PROC_PIDT_SHORTBSDINFO as _,
            0,
            (&raw mut bsdinfo).cast(),
            size_of::<proc_bsdshortinfo>() as _,
        );
        if ret == size_of::<proc_bsdshortinfo>() as _ && bsdinfo.pbsi_comm[0] != b'\0' {
            return libc::strdup((&raw const bsdinfo.pbsi_comm).cast());
        }
        null_mut()
    }
}

/// The working directory of the foreground process group of the terminal
/// `fd`.
#[cfg(target_os = "macos")]
pub fn osdep_get_cwd(fd: i32) -> Option<std::path::PathBuf> {
    use std::os::unix::ffi::OsStrExt;

    // SAFETY: tcgetpgrp only queries the descriptor.
    let pgrp: pid_t = unsafe { libc::tcgetpgrp(fd) };
    if pgrp == -1 {
        return None;
    }

    // SAFETY: an all zero proc_vnodepathinfo is valid, and proc_pidinfo
    // writes at most the size it is given.
    let (ret, pathinfo) = unsafe {
        let mut pathinfo: libc::proc_vnodepathinfo = zeroed();
        let ret = libc::proc_pidinfo(
            pgrp,
            libc::PROC_PIDVNODEPATHINFO as _,
            0,
            (&raw mut pathinfo).cast(),
            size_of::<libc::proc_vnodepathinfo>() as _,
        );
        (ret, pathinfo)
    };
    if ret != size_of::<libc::proc_vnodepathinfo>() as i32 {
        return None;
    }
    let path = &pathinfo.pvi_cdir.vip_path;
    // SAFETY: vip_path is a fixed size array of c_char, viewed as bytes.
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(path.as_ptr().cast(), size_of_val(path)) };
    let len = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    Some(std::ffi::OsStr::from_bytes(&bytes[..len]).into())
}

#[cfg(target_os = "macos")]
pub unsafe fn osdep_event_init() -> *mut event_base {
    unsafe {
        // On OS X, kqueue and poll are both completely broken and don't
        // work on anything except socket file descriptors (yes, really).
        std::env::set_var("EVENT_NOKQUEUE", "1");
        std::env::set_var("EVENT_NOPOLL", "1");

        let base: *mut event_base = event_init();

        std::env::remove_var("EVENT_NOKQUEUE");
        std::env::remove_var("EVENT_NOPOLL");

        base
    }
}
