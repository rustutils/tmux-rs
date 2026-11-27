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

use crate::environ::Environ;
use crate::cmd_::CmdList;
use crate::events::EventBus;
use crate::format_jobs::FormatJobs;
use crate::sort::LoopLists;
use crate::{ClientId, PaneId, SessionId, WinlinkId};
use crate::job_::JobId;
use crate::cmd_::cmd_wait_for::Waiter;
use crate::key_bindings::KeyTables;
use crate::links::Links;
use crate::wait_channel::WaitChannels;
use crate::message_log::MessageLog;
use crate::paste::PasteStore;
use crate::prompt_history::PromptHistory;
use crate::server_acl::ServerAcl;
use crate::style_links::StyleLinks;

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
    /// The global environment (set-environment -g), filled from the
    /// environment of the process that started the server.
    static GLOBAL_ENVIRON: RefCell<Environ> = RefCell::new(Environ::new());
    /// The key tables with their bindings.
    static KEY_TABLES: RefCell<KeyTables<CmdList>> = RefCell::new(KeyTables::new());
    /// The wait-for channels.
    static WAIT_CHANNELS: RefCell<WaitChannels<Waiter>> = RefCell::new(WaitChannels::new());
    /// The links between sessions and windows, and the session groups.
    static LINKS: RefCell<Links> = RefCell::new(Links::new());
    /// The lists the format loops last sorted (3.8's sort_get statics).
    static LOOP_LISTS: RefCell<LoopLists<SessionId, WinlinkId, PaneId, ClientId>> = RefCell::new(LoopLists::default());
    /// The `#()` jobs of format trees without a client.
    static FORMAT_JOBS: RefCell<FormatJobs<JobId>> = RefCell::new(FormatJobs::new());
    /// The format engine and its cache of parsed formats. It has no
    /// RefCell: the engine takes `&self` and may be reached again while it
    /// expands (a callback that expands a format).
    static FORMAT_ENGINE: tmux_format::Engine = tmux_format::Engine::new();
    /// The hyperlinks of styles (link=), by number.
    static STYLE_LINKS: RefCell<StyleLinks> = RefCell::new(StyleLinks::new());
}

/// Run `f` with the hyperlinks of styles.
///
/// # Panics
/// If `f` reaches the style links again (see the module rules).
pub fn with_style_links<R>(f: impl FnOnce(&mut StyleLinks) -> R) -> R {
    STYLE_LINKS.with_borrow_mut(f)
}

/// Run `f` with the `#()` jobs of format trees without a client.
///
/// # Panics
/// If `f` reaches these jobs again (see the module rules).
pub fn with_format_jobs<R>(f: impl FnOnce(&mut FormatJobs<JobId>) -> R) -> R {
    FORMAT_JOBS.with(|jobs| f(&mut jobs.borrow_mut()))
}

/// Run `f` with the lists the format loops last sorted.
///
/// # Panics
/// If `f` reaches these lists again (see the module rules).
pub fn with_loop_lists<R>(f: impl FnOnce(&mut LoopLists<SessionId, WinlinkId, PaneId, ClientId>) -> R) -> R {
    LOOP_LISTS.with(|lists| f(&mut lists.borrow_mut()))
}

/// Run `f` with the format engine. Unlike the stores, `f` may expand
/// formats and reach the engine again, which only takes shared
/// references.
pub fn with_format_engine<R>(f: impl FnOnce(&tmux_format::Engine) -> R) -> R {
    FORMAT_ENGINE.with(f)
}

/// Run `f` with the wait channels.
///
/// # Panics
/// If `f` reaches the wait channels again (see the module rules).
pub fn with_wait_channels<R>(f: impl FnOnce(&mut WaitChannels<Waiter>) -> R) -> R {
    WAIT_CHANNELS.with_borrow_mut(f)
}

/// Run `f` with the links between sessions and windows.
///
/// # Panics
/// If `f` reaches the links again (see the module rules).
pub fn with_links<R>(f: impl FnOnce(&mut Links) -> R) -> R {
    LINKS.with_borrow_mut(f)
}

/// Run `f` with the key tables.
///
/// # Panics
/// If `f` reaches the key tables again (see the module rules).
pub fn with_key_tables<R>(f: impl FnOnce(&mut KeyTables<CmdList>) -> R) -> R {
    KEY_TABLES.with_borrow_mut(f)
}

/// Run `f` with the global environment.
///
/// # Panics
/// If `f` reaches the global environment again (see the module rules).
pub fn with_global_environ<R>(f: impl FnOnce(&mut Environ) -> R) -> R {
    GLOBAL_ENVIRON.with_borrow_mut(f)
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
