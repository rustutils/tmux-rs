// Copyright (c) 2024 Patrick Elsen
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

//! The bridge from unconverted code to converted stores.
//!
//! Converted stores (see "Track 2 conventions" in the project notes) take
//! the state they work on as `&mut self` and never touch globals. Until a
//! Server struct owns them, each store lives in a thread local here and
//! unconverted code reaches it with `with_x(|store| ...)`.
//!
//! Two rules keep this sound without any unsafe code:
//!
//! - A closure passed to `with_x` only works on the store and returns
//!   values (names, `Rc` handles, events). It contains no unsafe code and
//!   calls nothing that could reach the same store again, because a
//!   second borrow of the `RefCell` panics. Notifications and other calls
//!   out happen after the closure has returned.
//! - Converted modules never use the bridge. They get the store they need
//!   as an argument, so they keep working unchanged when the Server
//!   struct replaces this file.
//!
//! The server is single threaded, so thread locals hold the same state a
//! static would. Unit tests get a fresh store per test thread.
#![forbid(unsafe_code)]

use std::cell::RefCell;

use crate::events::EventBus;
use crate::message_log::MessageLog;
use crate::paste::PasteStore;
use crate::prompt_history::PromptHistory;
use crate::server_acl::ServerAcl;

thread_local! {
    /// All paste buffers of this server.
    static PASTE: RefCell<PasteStore> = RefCell::new(PasteStore::new());
    /// The server message log.
    static MESSAGE_LOG: RefCell<MessageLog> = RefCell::new(MessageLog::new());
    /// Who may connect to this server, set up by `server_acl_init`.
    static ACL: RefCell<ServerAcl> = RefCell::new(ServerAcl::default());
    /// The registered event sinks.
    static EVENTS: RefCell<EventBus> = RefCell::new(EventBus::new());
    /// The prompt history of every prompt type.
    static PROMPT_HISTORY: RefCell<PromptHistory> = RefCell::new(PromptHistory::new());
}

/// Run `f` with the paste buffer store.
///
/// # Panics
/// If `f` reaches the paste store again (see the module rules).
pub fn with_paste<R>(f: impl FnOnce(&mut PasteStore) -> R) -> R {
    PASTE.with_borrow_mut(f)
}

/// Run `f` with the server message log.
///
/// # Panics
/// If `f` reaches the message log again (see the module rules).
pub fn with_message_log<R>(f: impl FnOnce(&mut MessageLog) -> R) -> R {
    MESSAGE_LOG.with_borrow_mut(f)
}

/// Run `f` with the server access control list.
///
/// # Panics
/// If `f` reaches the access control list again (see the module rules).
pub fn with_acl<R>(f: impl FnOnce(&mut ServerAcl) -> R) -> R {
    ACL.with_borrow_mut(f)
}

/// Run `f` with the event bus.
///
/// # Panics
/// If `f` reaches the event bus again (see the module rules).
pub fn with_events<R>(f: impl FnOnce(&mut EventBus) -> R) -> R {
    EVENTS.with_borrow_mut(f)
}

/// Run `f` with the prompt history.
///
/// # Panics
/// If `f` reaches the prompt history again (see the module rules).
pub fn with_prompt_history<R>(f: impl FnOnce(&mut PromptHistory) -> R) -> R {
    PROMPT_HISTORY.with_borrow_mut(f)
}
