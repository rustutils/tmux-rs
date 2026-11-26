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

//! The adapter between environments ([`store::Environ`]) and the rest of
//! tmux: the parts of environ.c that read options, the server process or
//! the process environment of a child.
mod store;

use std::ffi::CStr;
use std::os::unix::ffi::OsStrExt;

pub use self::store::{Environ, EnvironEntry, EnvironFlags, ProcessVars};

use crate::options_::*;
use crate::*;

/// Allocates an empty environment that the caller frees with
/// [`environ_free`].
pub fn environ_create() -> NonNull<Environ> {
    NonNull::from(Box::leak(Box::new(Environ::new())))
}

/// Frees an environment from [`environ_create`].
///
/// # Safety
/// `env` must come from [`environ_create`] and not be used afterwards.
pub unsafe fn environ_free(env: *mut Environ) {
    // SAFETY: the caller passes a pointer from environ_create.
    unsafe { drop(Box::from_raw(env)) }
}

/// Applies the update-environment option of `oo` to `dst`, with the
/// variables of `src` (a client's environment).
///
/// # Safety
/// `oo` must be a valid options tree.
pub unsafe fn environ_update(oo: *mut options, src: &Environ, dst: &mut Environ) {
    unsafe {
        let o = options_get(&mut *oo, "update-environment");
        if o.is_null() {
            return;
        }
        let patterns = options_array_strings(o);
        dst.update(
            patterns.iter().map(|(_, p)| p.as_bytes()),
            src,
            fnmatch_bytes,
        );
    }
}

/// fnmatch(3) with no flags on byte strings. Neither can contain NUL: a
/// pattern comes from a C string option and a name from a C environment.
fn fnmatch_bytes(pattern: &[u8], name: &[u8]) -> bool {
    let (Ok(pattern), Ok(name)) = (CString::new(pattern), CString::new(name)) else {
        return false;
    };
    // SAFETY: both are NUL terminated strings that live for the call.
    unsafe { libc::fnmatch(pattern.as_ptr().cast(), name.as_ptr().cast(), 0) == 0 }
}

/// Replaces the environment of this process with `env`'s process
/// variables (`environ_push`). Called in a child after fork, before exec:
/// the child gets exactly the variables tmux built, nothing the server
/// process had besides them.
///
/// # Safety
/// Changes the process environment, so no other thread may read or write
/// it. A child after fork has only this thread.
pub unsafe fn environ_push(env: &Environ) {
    let current: Vec<_> = std::env::vars_os().map(|(name, _)| name).collect();
    for name in current {
        // SAFETY: the caller guarantees no other thread uses the
        // environment.
        unsafe { std::env::remove_var(name) };
    }
    for (name, value) in env.process_vars() {
        // SAFETY: as above. process_vars gives no empty name, and names
        // and values come from C strings, so neither holds NUL. A name
        // holds no '=' because put and set-environment stop or fail there.
        unsafe {
            std::env::set_var(
                std::ffi::OsStr::from_bytes(name),
                std::ffi::OsStr::from_bytes(value),
            );
        }
    }
}

macro_rules! environ_log {
   ($env:expr, $fmt:literal $(, $args:expr)* $(,)?) => {
        crate::environ::environ_log_(&*$env, format_args!($fmt $(, $args)*))
    };
}
pub(crate) use environ_log;

/// Logs the variables of `env` that have a value, each after `args`.
pub fn environ_log_(env: &Environ, args: std::fmt::Arguments) {
    let prefix = args.to_string();
    for entry in env.iter() {
        if let Some(value) = entry.value()
            && !entry.name().is_empty()
        {
            log_debug!(
                "{}{}={}",
                prefix,
                String::from_utf8_lossy(entry.name()),
                String::from_utf8_lossy(value),
            );
        }
    }
}

/// The environment of a new process in session `s` (`environ_for_session`):
/// the global environment, the session's over it, then TERM,
/// `TERM_PROGRAM`, `TERM_PROGRAM_VERSION` and COLORTERM unless `no_term` is
/// nonzero, then TMUX.
///
/// # Safety
/// `s` must be null or a valid session, and the global environment and
/// options must exist.
pub unsafe fn environ_for_session(s: *mut session, no_term: c_int) -> *mut Environ {
    unsafe {
        let term = (no_term == 0).then(|| options_get_string_(GLOBAL_OPTIONS, "default-terminal"));
        let idx = if s.is_null() { -1 } else { (*s).id as i32 };
        let mut tmux = socket_path().to_bytes().to_vec();
        tmux.extend_from_slice(format!(",{},{}", std::process::id(), idx).as_bytes());
        let vars = ProcessVars {
            term: term.map(|t| CStr::from_ptr(t.cast()).to_bytes()),
            version: getversion().as_bytes(),
            tmux: &tmux,
        };
        let session = if s.is_null() { None } else { Some(&*(*s).environ) };
        let env = Environ::for_process(&*GLOBAL_ENVIRON, session, &vars);
        Box::leak(Box::new(env))
    }
}
