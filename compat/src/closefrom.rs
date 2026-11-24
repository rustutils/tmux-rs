// Copyright (c) 2004-2005 Todd C. Miller <Todd.Miller@courtesan.com>
//
// Permission to use, copy, modify, and distribute this software for any
// purpose with or without fee is hereby granted, provided that the above
// copyright notice and this permission notice appear in all copies.
//
// THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
// WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
// MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
// ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
// WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
// ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
// OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

/// Closes all file descriptors >= `start_fd`, as BSD closefrom does. On
/// Linux it uses the close_range system call (Linux 5.9 and later), as
/// glibc's closefrom does. musl and macOS have no closefrom, and older
/// kernels no close_range, so otherwise it closes them one by one.
///
/// # Safety
///
/// Nothing may use those file descriptors afterwards, for example because
/// the process is about to exec.
pub unsafe fn closefrom(start_fd: i32) {
    unsafe {
        #[cfg(target_os = "linux")]
        if let Ok(first) = libc::c_uint::try_from(start_fd)
            && libc::syscall(libc::SYS_close_range, first, libc::c_uint::MAX, 0) == 0
        {
            return;
        }
        let max_fd = libc::sysconf(libc::_SC_OPEN_MAX);
        let max_fd = if max_fd > 0 {
            i32::try_from(max_fd).unwrap_or(i32::MAX)
        } else {
            1024
        };
        for fd in start_fd..max_fd {
            libc::close(fd); // ignore close errors
        }
    }
}
