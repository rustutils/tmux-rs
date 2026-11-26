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

//! The server message log for the rest of tmux.
//!
//! [`MessageLog`] in [`store`] keeps the messages and is safe Rust. The
//! functions here are the adapter: they reach the log through the bridge
//! in `global.rs` and pass in the time and the message-limit option.
//! Adding a message is unsafe until the options are converted (F2).

pub mod store;

use std::time::{SystemTime, UNIX_EPOCH};

pub use store::{Message, MessageLog};

use crate::global::with_message_log;
use crate::libc::timeval;
use crate::options_::options_get_number_;
use crate::GLOBAL_OPTIONS;

/// Adds a formatted message to the server message log, see
/// [`server_add_message_`].
macro_rules! server_add_message {
   ($fmt:literal $(, $args:expr)* $(,)?) => {
        crate::message_log::server_add_message_(format_args!($fmt $(, $args)*))
    };
}
pub(crate) use server_add_message;

/// Adds a message to the server message log, keeping as many messages as
/// the message-limit option says.
///
/// # Safety
/// The global options must be initialized.
pub unsafe fn server_add_message_(args: std::fmt::Arguments) {
    let text = args.to_string();
    log_debug!("message: {}", text);
    // SAFETY: the caller guarantees the global options exist.
    let limit = unsafe { options_get_number_(GLOBAL_OPTIONS, "message-limit") };
    let limit = u32::try_from(limit).unwrap_or(u32::MAX);
    with_message_log(|log| log.add(text, SystemTime::now(), limit));
}

/// Empties the log and restarts the numbering, when a server starts.
pub fn message_log_reset() {
    with_message_log(|log| *log = MessageLog::new());
}

/// A message time as a timeval for formats.
#[cfg_attr(
    not(target_os = "macos"),
    expect(
        clippy::unnecessary_fallible_conversions,
        reason = "tv_usec is i32 on macOS, where the conversion can fail"
    )
)]
pub fn message_timeval(message: &Message) -> timeval {
    let since = message.time().duration_since(UNIX_EPOCH).unwrap_or_default();
    timeval {
        tv_sec: since.as_secs().try_into().unwrap_or_default(),
        tv_usec: since.subsec_micros().try_into().unwrap_or_default(),
    }
}
