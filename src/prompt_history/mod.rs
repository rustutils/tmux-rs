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

//! Prompt history for the rest of tmux.
//!
//! The history is kept by [`PromptHistory`] in [`store`]. The functions
//! here reach it through the bridge in `global.rs`, read the
//! prompt-history-limit and history-file options, and read and write the
//! history file.

pub mod store;

pub use store::{HistoryPositions, PromptHistory, PromptType};

use std::os::unix::ffi::OsStrExt;

use crate::GLOBAL_OPTIONS;
use crate::global::with_prompt_history;
use crate::options_::{options_get_number_, options_get_string_};
use crate::{find_home, log_debug};

/// The prompt-history-limit option.
///
/// # Safety
/// The global options must exist.
unsafe fn history_limit() -> usize {
    // SAFETY: the caller guarantees GLOBAL_OPTIONS is valid.
    let limit = unsafe { options_get_number_(GLOBAL_OPTIONS, "prompt-history-limit") };
    usize::try_from(limit).unwrap_or(0)
}

/// The history file from the history-file option: an absolute path or
/// one starting with "~/", otherwise none.
///
/// # Safety
/// The global options must exist.
unsafe fn history_file() -> Option<std::path::PathBuf> {
    // SAFETY: the caller guarantees GLOBAL_OPTIONS is valid.
    let option = unsafe { options_get_string_(GLOBAL_OPTIONS, "history-file") };
    // SAFETY: options_get_string_ returns a valid string.
    let option = unsafe { std::ffi::CStr::from_ptr(option.cast()) }.to_bytes();
    let option = std::str::from_utf8(option).ok()?;
    if option.starts_with('/') {
        return Some(option.into());
    }
    let rest = option.strip_prefix("~/")?;
    let home = std::ffi::OsStr::from_bytes(find_home()?.to_bytes());
    Some(std::path::Path::new(home).join(rest))
}

/// Add `line` to the history of `ty`.
///
/// # Safety
/// The global options must exist.
pub unsafe fn prompt_history_add(ty: PromptType, line: &[u8]) {
    // SAFETY: the caller guarantees the global options exist.
    let limit = unsafe { history_limit() };
    with_prompt_history(|h| h.add(ty, line, limit));
}

/// The entry before the current one of a prompt, see
/// [`PromptHistory::up`].
pub fn prompt_history_up(ty: PromptType, positions: &mut HistoryPositions) -> Option<Vec<u8>> {
    with_prompt_history(|h| h.up(ty, positions).map(<[u8]>::to_vec))
}

/// The entry after the current one of a prompt, see
/// [`PromptHistory::down`].
pub fn prompt_history_down(ty: PromptType, positions: &mut HistoryPositions) -> Vec<u8> {
    with_prompt_history(|h| h.down(ty, positions).to_vec())
}

/// Load the history file, if history-file names one.
///
/// # Safety
/// The global options must exist.
pub unsafe fn prompt_history_load() {
    // SAFETY: the caller guarantees the global options exist.
    let (Some(path), limit) = (unsafe { history_file() }, unsafe { history_limit() }) else {
        return;
    };
    log_debug!("loading history from {}", path.display());
    match std::fs::read(&path) {
        Ok(contents) => with_prompt_history(|h| h.load(&contents, limit)),
        Err(e) => log_debug!("{}: {e}", path.display()),
    }
}

/// Save the history file, if history-file names one.
///
/// # Safety
/// The global options must exist.
pub unsafe fn prompt_history_save() {
    // SAFETY: the caller guarantees the global options exist.
    let Some(path) = (unsafe { history_file() }) else {
        return;
    };
    log_debug!("saving history to {}", path.display());
    let contents = with_prompt_history(|h| h.to_file());
    if let Err(e) = std::fs::write(&path, contents) {
        log_debug!("{}: {e}", path.display());
    }
}
