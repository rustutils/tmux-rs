/// Creates the server socket through systemd socket activation.
///
/// This is a declaration only. Nothing in the tree implements the C symbol
/// yet, so the "systemd" feature does not link.
///
/// # Safety
///
/// `cause` must be valid for writing a pointer. On error it receives a
/// message allocated with malloc that the caller frees.
pub unsafe fn systemd_create_socket(flags: i32, cause: *mut *mut u8) -> i32 {
    unsafe extern "C" {
        fn systemd_create_socket(flags: i32, cause: *mut *mut u8) -> i32;
    }
    unsafe { systemd_create_socket(flags, cause) }
}
