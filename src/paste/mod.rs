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

//! Paste buffers for the rest of tmux.
//!
//! The buffers themselves are kept by [`PasteStore`] in [`store`], which is
//! safe Rust and knows nothing outside itself. The functions here are the
//! adapter for unconverted code: they reach the store through the bridge
//! in `global.rs`, pass in the time and the buffer-limit option, and raise
//! the paste-buffer-changed and paste-buffer-deleted notifications for the
//! events the store returns.
//!
//! Lookups return a copy of the buffer ([`PasteBuffer`] shares its data,
//! so this is cheap). Callers keep the name or the copy, never a pointer.
//! The functions that raise notifications are unsafe until notify.rs is
//! converted.

pub mod store;

use std::rc::Rc;

pub use store::{PasteBuffer, PasteError, PasteEvent, PasteStore};

use crate::global::with_paste;
use crate::options_::options_get_number_;
use crate::GLOBAL_OPTIONS;
use crate::events::{Payload, events_fire};

/// Whether there are no paste buffers.
pub fn paste_is_empty() -> bool {
    with_paste(|store| store.is_empty())
}

/// The buffer called `name`, if there is one.
pub fn paste_get_name(name: &str) -> Option<PasteBuffer> {
    with_paste(|store| store.get(name).cloned())
}

/// The buffer called `name`, for a name given as bytes by a client. Names
/// that are not UTF-8 find nothing, since no buffer can have one.
pub fn paste_get_name_bytes(name: &[u8]) -> Option<PasteBuffer> {
    paste_get_name(std::str::from_utf8(name).ok()?)
}

/// The newest automatic buffer, which commands use when no buffer is
/// named.
pub fn paste_get_top() -> Option<PasteBuffer> {
    with_paste(|store| store.top().cloned())
}

/// All buffers, newest first.
pub fn paste_list() -> Vec<PasteBuffer> {
    with_paste(|store| store.iter().cloned().collect())
}

/// Delete the buffer called `name`. Returns false if there is none.
///
/// # Safety
/// Raises a notification, see [`events_fire`].
pub unsafe fn paste_free(name: &str) -> bool {
    let event = with_paste(|store| store.delete(name));
    let found = event.is_some();
    // SAFETY: the caller upholds events_fire's requirements.
    unsafe { paste_notify(event) };
    found
}

/// Add an automatic buffer named from `prefix` ("buffer" if `None`) and a
/// counter, deleting the oldest automatic buffers beyond buffer-limit.
///
/// # Safety
/// Reads the global options and raises notifications.
pub unsafe fn paste_add(prefix: Option<&str>, data: impl Into<Rc<[u8]>>) {
    let prefix = prefix.unwrap_or("buffer");
    // SAFETY: the caller guarantees the global options exist.
    let limit = unsafe { buffer_limit() };
    let events = with_paste(|store| store.add(prefix, data, limit, now()));
    // SAFETY: the caller upholds events_fire's requirements.
    unsafe { paste_notify(events) };
}

/// Set the buffer called `name` to `data`, or add an automatic buffer if
/// `name` is `None`. The name is checked and cleaned as in tmux 3.8.
///
/// # Safety
/// Reads the global options and raises notifications.
pub unsafe fn paste_set(data: impl Into<Rc<[u8]>>, name: Option<&[u8]>) -> Result<(), PasteError> {
    // SAFETY: the caller guarantees the global options exist.
    let limit = unsafe { buffer_limit() };
    let events = with_paste(|store| store.set(name, data, limit, now()))?;
    // SAFETY: the caller upholds events_fire's requirements.
    unsafe { paste_notify(events) };
    Ok(())
}

/// Replace the contents of buffer `name`. Returns false if there is none.
///
/// # Safety
/// Raises a notification.
pub unsafe fn paste_replace(name: &str, data: impl Into<Rc<[u8]>>) -> bool {
    let event = with_paste(|store| store.replace(name, data));
    let found = event.is_some();
    // SAFETY: the caller upholds events_fire's requirements.
    unsafe { paste_notify(event) };
    found
}

/// Raise the notification for each event.
///
/// # Safety
/// See [`events_fire`].
pub(crate) unsafe fn paste_notify(events: impl IntoIterator<Item = PasteEvent>) {
    for event in events {
        let (event_name, name) = match event {
            PasteEvent::Changed(name) => ("paste-buffer-changed", name),
            PasteEvent::Deleted(name) => ("paste-buffer-deleted", name),
        };
        let mut payload = Payload::new();
        payload.set_string("paste_buffer", name);
        // SAFETY: the caller upholds events_fire's requirements.
        unsafe { events_fire(event_name, payload, std::ptr::null_mut()) };
    }
}

/// The buffer-limit option.
///
/// # Safety
/// The global options must exist.
pub(crate) unsafe fn buffer_limit() -> u32 {
    // SAFETY: the caller guarantees GLOBAL_OPTIONS is valid.
    let limit = unsafe { options_get_number_(GLOBAL_OPTIONS, "buffer-limit") };
    u32::try_from(limit.max(0)).unwrap_or(u32::MAX)
}

/// The current time in seconds since the epoch.
pub(crate) fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}
