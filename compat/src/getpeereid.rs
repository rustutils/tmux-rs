/// Get the effective user and group id of the process at the other end of
/// the unix socket `s`. The server uses this to apply its access control
/// list to connecting clients. Returns 0 on success and -1 on failure with
/// errno set.
///
/// On Linux this asks the kernel with `SO_PEERCRED`. On the BSDs and macOS
/// libc has `getpeereid` itself.
///
/// # Safety
///
/// `uid` and `gid` must be valid for writes.
#[cfg(any(target_os = "linux", target_os = "android"))]
pub unsafe fn getpeereid(s: i32, uid: *mut libc::uid_t, gid: *mut libc::gid_t) -> i32 {
    unsafe {
        let mut uc: libc::ucred = std::mem::zeroed();
        let mut len = size_of::<libc::ucred>() as libc::socklen_t;
        if libc::getsockopt(
            s,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut uc).cast(),
            &raw mut len,
        ) == -1
        {
            return -1;
        }
        *uid = uc.uid;
        *gid = uc.gid;
        0
    }
}

/// See the Linux version above.
///
/// # Safety
///
/// `uid` and `gid` must be valid for writes.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
pub unsafe fn getpeereid(s: i32, uid: *mut libc::uid_t, gid: *mut libc::gid_t) -> i32 {
    unsafe { libc::getpeereid(s, uid, gid) }
}

#[cfg(test)]
mod tests {
    use super::getpeereid;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn reports_peer_of_socket_pair() {
        let (a, _b) = UnixStream::pair().expect("socketpair");
        let mut uid: libc::uid_t = u32::MAX;
        let mut gid: libc::gid_t = u32::MAX;
        let rc = unsafe { getpeereid(a.as_raw_fd(), &raw mut uid, &raw mut gid) };
        assert_eq!(rc, 0);
        // Both ends belong to this process.
        assert_eq!(uid, unsafe { libc::geteuid() });
        assert_eq!(gid, unsafe { libc::getegid() });
    }

    #[test]
    fn fails_on_non_socket() {
        let file = std::fs::File::open("/dev/null").expect("open /dev/null");
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        let rc = unsafe { getpeereid(file.as_raw_fd(), &raw mut uid, &raw mut gid) };
        assert_eq!(rc, -1);
    }
}
