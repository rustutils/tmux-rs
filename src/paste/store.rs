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

//! The paste buffer store, paste.c from tmux 3.8 in safe Rust.
//!
//! Buffers come in two kinds:
//!
//! - Automatic buffers are made by copy commands and named from a prefix
//!   and a counter (`buffer0`, `buffer1`). The `buffer-limit` option caps
//!   how many are kept, the oldest are deleted first.
//! - Named buffers are made with `set-buffer -b` or by renaming a buffer.
//!   They are never deleted to make room.
//!
//! Every buffer has an order number that grows with each new buffer. It is
//! the buffer's identity: renaming keeps it, replacing a buffer under the
//! same name gives a new one. Iteration is newest first, as in C.
//!
//! The store does not call out. Methods that change it return the
//! [`PasteEvent`]s the caller has to raise (the paste-buffer-changed and
//! paste-buffer-deleted hooks). The time and the buffer limit come in as
//! arguments. Buffer data is shared as `Rc<[u8]>`, so callers take a copy
//! of a [`PasteBuffer`] cheaply and never hold a reference into the store.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::rc::Rc;

use crate::clean_name::clean_name;
use tmux_compat::vis::vis_flags;
use crate::utf8::utf8_strvis_;

/// Width of [`PasteBuffer::sample`] before it is cut off with "...".
const SAMPLE_WIDTH: usize = 200;

/// One paste buffer. Cloning is cheap, the data is shared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasteBuffer {
    name: String,
    data: Rc<[u8]>,
    created: i64,
    automatic: bool,
    order: u32,
}

impl PasteBuffer {
    /// The buffer name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The buffer contents.
    pub fn data(&self) -> &Rc<[u8]> {
        &self.data
    }

    /// When the buffer was created, in seconds since the epoch.
    pub fn created(&self) -> i64 {
        self.created
    }

    /// Order number, unique within a store and higher for newer buffers.
    /// Renaming keeps it, so it tells whether a buffer is still the same
    /// one.
    pub fn order(&self) -> u32 {
        self.order
    }

    /// The start of the buffer as printable text, for list-buffers and
    /// choose-buffer. Non-printable bytes are escaped. Longer than 200
    /// bytes (before or after escaping), it is cut at 200 bytes and ends
    /// with "...".
    pub fn sample(&self) -> String {
        let len = self.data.len().min(SAMPLE_WIDTH);
        let mut buf = Vec::with_capacity(len * 4 + 3);
        utf8_strvis_(
            &mut buf,
            &self.data[..len],
            vis_flags::VIS_OCTAL | vis_flags::VIS_CSTYLE | vis_flags::VIS_TAB | vis_flags::VIS_NL,
        );
        if self.data.len() > SAMPLE_WIDTH || buf.len() > SAMPLE_WIDTH {
            buf.truncate(SAMPLE_WIDTH);
            buf.extend_from_slice(b"...");
        }
        // Cutting at 200 bytes can split a UTF-8 character, which C
        // leaves as it is.
        String::from_utf8_lossy(&buf).into_owned()
    }
}

/// A change the caller reports once the store call has returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasteEvent {
    /// A buffer was created or its contents changed (paste-buffer-changed).
    Changed(String),
    /// A buffer was deleted or renamed away (paste-buffer-deleted).
    Deleted(String),
}

/// Why a paste store change failed. The text is the C error message.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PasteError {
    /// Renaming without naming the buffer to rename.
    #[error("no buffer")]
    NoBuffer,
    /// Renaming a buffer that does not exist.
    #[error("no buffer {0}")]
    UnknownBuffer(String),
    /// Renaming to an empty name.
    #[error("new name is empty")]
    NewNameEmpty,
    /// Setting a buffer with an empty name.
    #[error("empty buffer name")]
    EmptyName,
    /// A name with control characters or invalid UTF-8.
    #[error("invalid buffer name: {0}")]
    InvalidName(String),
}

/// All paste buffers of a server.
#[derive(Debug, Default)]
pub struct PasteStore {
    /// The buffers by name.
    by_name: BTreeMap<String, PasteBuffer>,
    /// Buffer names by order number, oldest first.
    by_order: BTreeMap<u32, String>,
    /// Counter for automatic buffer names.
    next_index: u32,
    /// Order number of the next buffer.
    next_order: u32,
    /// How many buffers are automatic.
    num_automatic: u32,
}

impl PasteStore {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether there are no buffers.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }

    /// The buffer called `name`. An empty name finds nothing.
    pub fn get(&self, name: &str) -> Option<&PasteBuffer> {
        self.by_name.get(name)
    }

    /// The newest automatic buffer, which commands use when no buffer is
    /// named.
    pub fn top(&self) -> Option<&PasteBuffer> {
        self.iter().find(|pb| pb.automatic)
    }

    /// All buffers, newest first.
    pub fn iter(&self) -> impl Iterator<Item = &PasteBuffer> {
        self.by_order.values().rev().filter_map(|name| self.by_name.get(name))
    }

    /// Delete the buffer called `name`. Returns the deleted event, or
    /// `None` if there is no such buffer.
    pub fn delete(&mut self, name: &str) -> Option<PasteEvent> {
        let pb = self.by_name.remove(name)?;
        self.by_order.remove(&pb.order);
        if pb.automatic {
            self.num_automatic -= 1;
        }
        self.debug_check();
        Some(PasteEvent::Deleted(pb.name))
    }

    /// Add an automatic buffer named `prefix` followed by a number. Before
    /// that, the oldest automatic buffers are deleted until fewer than
    /// `limit` are left. Empty data adds nothing.
    pub fn add(&mut self, prefix: &str, data: impl Into<Rc<[u8]>>, limit: u32, now: i64) -> Vec<PasteEvent> {
        let data = data.into();
        if data.is_empty() {
            return Vec::new();
        }

        let mut events = Vec::new();
        let oldest_automatic: Vec<String> = self
            .by_order
            .values()
            .filter(|name| self.by_name.get(*name).is_some_and(|pb| pb.automatic))
            .cloned()
            .collect();
        for name in oldest_automatic {
            if self.num_automatic < limit {
                break;
            }
            events.extend(self.delete(&name));
        }

        let name = loop {
            let name = format!("{prefix}{}", self.next_index);
            self.next_index = self.next_index.wrapping_add(1);
            if !self.by_name.contains_key(&name) {
                break name;
            }
        };
        self.num_automatic += 1;
        self.insert(name.clone(), data, true, now);
        events.push(PasteEvent::Changed(name));
        events
    }

    /// Set the buffer called `name` to `data`, replacing any buffer with
    /// that name. The name is cleaned first (see
    /// [`clean_name`](crate::clean_name::clean_name)). Without a name this
    /// is [`add`](Self::add) with the prefix "buffer". Empty data does
    /// nothing, even with an invalid name, as in C.
    pub fn set(
        &mut self,
        name: Option<&[u8]>,
        data: impl Into<Rc<[u8]>>,
        limit: u32,
        now: i64,
    ) -> Result<Vec<PasteEvent>, PasteError> {
        let data = data.into();
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let Some(name) = name else {
            return Ok(self.add("buffer", data, limit, now));
        };
        if name.is_empty() {
            return Err(PasteError::EmptyName);
        }
        let name = clean_name(name, false).ok_or_else(|| invalid_name(name))?;

        let mut events: Vec<PasteEvent> = self.delete(&name).into_iter().collect();
        self.insert(name.clone(), data, false, now);
        events.push(PasteEvent::Changed(name));
        Ok(events)
    }

    /// Rename buffer `old` to `new`, deleting any buffer already called
    /// `new`. The renamed buffer becomes a named buffer and keeps its
    /// place in the order. Renaming a buffer to its own name does nothing.
    pub fn rename(&mut self, old: &str, new: &[u8]) -> Result<Vec<PasteEvent>, PasteError> {
        if old.is_empty() {
            return Err(PasteError::NoBuffer);
        }
        if new.is_empty() {
            return Err(PasteError::NewNameEmpty);
        }
        let new = clean_name(new, false).ok_or_else(|| invalid_name(new))?;
        if !self.by_name.contains_key(old) {
            return Err(PasteError::UnknownBuffer(old.to_string()));
        }
        if new == old {
            return Ok(Vec::new());
        }

        let mut events: Vec<PasteEvent> = self.delete(&new).into_iter().collect();
        let mut pb = self.by_name.remove(old).expect("checked above");
        if pb.automatic {
            self.num_automatic -= 1;
        }
        pb.automatic = false;
        pb.name.clone_from(&new);
        self.by_order.insert(pb.order, new.clone());
        self.by_name.insert(new.clone(), pb);
        self.debug_check();

        events.push(PasteEvent::Deleted(old.to_string()));
        events.push(PasteEvent::Changed(new));
        Ok(events)
    }

    /// Replace the contents of buffer `name`, keeping its name, kind and
    /// order. Returns the changed event, or `None` if there is no such
    /// buffer.
    pub fn replace(&mut self, name: &str, data: impl Into<Rc<[u8]>>) -> Option<PasteEvent> {
        let pb = self.by_name.get_mut(name)?;
        pb.data = data.into();
        Some(PasteEvent::Changed(name.to_string()))
    }

    /// Store a new buffer with the next order number. The name must be
    /// free.
    fn insert(&mut self, name: String, data: Rc<[u8]>, automatic: bool, now: i64) {
        let order = self.next_order;
        self.next_order = self.next_order.wrapping_add(1);
        debug_assert!(!self.by_name.contains_key(&name));
        self.by_order.insert(order, name.clone());
        self.by_name.insert(
            name.clone(),
            PasteBuffer {
                name,
                data,
                created: now,
                automatic,
                order,
            },
        );
        self.debug_check();
    }

    /// Check that the two maps and the automatic count agree.
    fn debug_check(&self) {
        debug_assert_eq!(self.by_name.len(), self.by_order.len());
        debug_assert!(
            self.by_order
                .iter()
                .all(|(order, name)| self.by_name.get(name).is_some_and(|pb| pb.order == *order))
        );
        debug_assert_eq!(
            self.by_name.values().filter(|pb| pb.automatic).count(),
            self.num_automatic as usize
        );
    }
}

/// The error for a name that [`clean_name`] rejects.
fn invalid_name(name: &[u8]) -> PasteError {
    PasteError::InvalidName(String::from_utf8_lossy(name).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMIT: u32 = 50;
    const NOW: i64 = 1_700_000_000;

    fn names(store: &PasteStore) -> Vec<&str> {
        store.iter().map(PasteBuffer::name).collect()
    }

    fn changed(name: &str) -> PasteEvent {
        PasteEvent::Changed(name.to_string())
    }

    fn deleted(name: &str) -> PasteEvent {
        PasteEvent::Deleted(name.to_string())
    }

    fn set(store: &mut PasteStore, name: &str, data: &[u8]) -> Vec<PasteEvent> {
        store.set(Some(name.as_bytes()), data, LIMIT, NOW).unwrap()
    }

    #[test]
    fn new_store_is_empty() {
        let store = PasteStore::new();
        assert!(store.is_empty());
        assert_eq!(store.by_name.len(), 0);
        assert!(store.top().is_none());
        assert!(store.iter().next().is_none());
    }

    #[test]
    fn set_named_buffer() {
        let mut store = PasteStore::new();
        assert_eq!(set(&mut store, "mybuf", b"hello"), vec![changed("mybuf")]);
        let pb = store.get("mybuf").unwrap();
        assert_eq!(&**pb.data(), b"hello");
        assert!(!pb.automatic);
        assert_eq!(pb.created(), NOW);
        assert!(store.top().is_none(), "named buffers are not the top buffer");
    }

    #[test]
    fn set_replaces_a_buffer_with_the_same_name() {
        let mut store = PasteStore::new();
        set(&mut store, "buf", b"first");
        let first = store.get("buf").unwrap().order();
        assert_eq!(set(&mut store, "buf", b"second"), vec![deleted("buf"), changed("buf")]);
        assert_eq!(store.by_name.len(), 1);
        let pb = store.get("buf").unwrap();
        assert_eq!(&**pb.data(), b"second");
        assert_ne!(pb.order(), first, "a replaced buffer is a new buffer");
    }

    #[test]
    fn set_errors() {
        let mut store = PasteStore::new();
        assert_eq!(store.set(Some(b""), b"x".as_slice(), LIMIT, NOW), Err(PasteError::EmptyName));
        let err = store.set(Some(b"a\tb"), b"x".as_slice(), LIMIT, NOW).unwrap_err();
        assert_eq!(err.to_string(), "invalid buffer name: a\tb");
        assert_eq!(PasteError::EmptyName.to_string(), "empty buffer name");
        assert!(store.is_empty());
    }

    #[test]
    fn set_with_empty_data_does_nothing() {
        let mut store = PasteStore::new();
        // C checks the size first, so even a bad name is not an error.
        assert_eq!(store.set(Some(b"a\tb"), Vec::new(), LIMIT, NOW), Ok(Vec::new()));
        assert_eq!(store.set(None, Vec::new(), LIMIT, NOW), Ok(Vec::new()));
        assert!(store.is_empty());
    }

    #[test]
    fn set_cleans_the_name() {
        let mut store = PasteStore::new();
        set(&mut store, "a\\b", b"x");
        assert_eq!(names(&store), vec!["a\\\\b"]);
    }

    #[test]
    fn set_without_name_adds_an_automatic_buffer() {
        let mut store = PasteStore::new();
        let events = store.set(None, b"auto".as_slice(), LIMIT, NOW).unwrap();
        assert_eq!(events, vec![changed("buffer0")]);
        assert!(store.get("buffer0").unwrap().automatic);
    }

    #[test]
    fn add_names_buffers_from_prefix_and_counter() {
        let mut store = PasteStore::new();
        store.add("buffer", b"a".as_slice(), LIMIT, NOW);
        store.add("buffer", b"b".as_slice(), LIMIT, NOW);
        store.add("custom", b"c".as_slice(), LIMIT, NOW);
        assert_eq!(names(&store), vec!["custom2", "buffer1", "buffer0"]);
    }

    #[test]
    fn add_skips_names_that_are_taken() {
        let mut store = PasteStore::new();
        set(&mut store, "buffer0", b"named");
        store.add("buffer", b"auto".as_slice(), LIMIT, NOW);
        assert_eq!(names(&store), vec!["buffer1", "buffer0"]);
        assert!(!store.get("buffer0").unwrap().automatic);
    }

    #[test]
    fn add_with_empty_data_does_nothing() {
        let mut store = PasteStore::new();
        assert!(store.add("buffer", Vec::new(), LIMIT, NOW).is_empty());
        assert!(store.is_empty());
    }

    #[test]
    fn add_deletes_oldest_automatic_buffers_at_the_limit() {
        let mut store = PasteStore::new();
        set(&mut store, "named", b"n");
        for data in [b"a", b"b", b"c"] {
            store.add("buffer", data.as_slice(), 3, NOW);
        }
        let events = store.add("buffer", b"d".as_slice(), 3, NOW);
        assert_eq!(events, vec![deleted("buffer0"), changed("buffer3")]);
        assert_eq!(names(&store), vec!["buffer3", "buffer2", "buffer1", "named"]);

        // A lower limit deletes several, oldest first, and never named ones.
        let events = store.add("buffer", b"e".as_slice(), 1, NOW);
        assert_eq!(
            events,
            vec![deleted("buffer1"), deleted("buffer2"), deleted("buffer3"), changed("buffer4")]
        );
        assert_eq!(names(&store), vec!["buffer4", "named"]);
    }

    #[test]
    fn iteration_is_newest_first() {
        let mut store = PasteStore::new();
        set(&mut store, "ccc", b"1");
        set(&mut store, "aaa", b"2");
        store.add("buffer", b"3".as_slice(), LIMIT, NOW);
        set(&mut store, "bbb", b"4");
        assert_eq!(names(&store), vec!["bbb", "buffer0", "aaa", "ccc"]);
    }

    #[test]
    fn top_is_the_newest_automatic_buffer() {
        let mut store = PasteStore::new();
        store.add("buffer", b"a1".as_slice(), LIMIT, NOW);
        store.add("buffer", b"a2".as_slice(), LIMIT, NOW);
        set(&mut store, "manual", b"m");
        assert_eq!(store.top().unwrap().name(), "buffer1");
    }

    #[test]
    fn get_with_empty_or_unknown_name() {
        let mut store = PasteStore::new();
        set(&mut store, "x", b"x");
        assert!(store.get("").is_none());
        assert!(store.get("nope").is_none());
    }

    #[test]
    fn delete_removes_the_buffer() {
        let mut store = PasteStore::new();
        set(&mut store, "named1", b"n1");
        store.add("buffer", b"a".as_slice(), LIMIT, NOW);
        set(&mut store, "named2", b"n2");
        assert_eq!(store.delete("buffer0"), Some(deleted("buffer0")));
        assert_eq!(store.delete("buffer0"), None);
        assert_eq!(names(&store), vec!["named2", "named1"]);
        assert!(store.top().is_none());
    }

    #[test]
    fn delete_frees_room_under_the_limit() {
        let mut store = PasteStore::new();
        store.add("buffer", b"a".as_slice(), 2, NOW);
        store.add("buffer", b"b".as_slice(), 2, NOW);
        store.delete("buffer0");
        // One automatic buffer left, so a second one fits again.
        let events = store.add("buffer", b"c".as_slice(), 2, NOW);
        assert_eq!(events, vec![changed("buffer2")]);
    }

    #[test]
    fn rename_keeps_order_and_makes_buffer_named() {
        let mut store = PasteStore::new();
        store.add("buffer", b"content".as_slice(), LIMIT, NOW);
        set(&mut store, "later", b"l");
        let order = store.get("buffer0").unwrap().order();

        let events = store.rename("buffer0", b"new").unwrap();
        assert_eq!(events, vec![deleted("buffer0"), changed("new")]);
        assert!(store.get("buffer0").is_none());
        let pb = store.get("new").unwrap();
        assert_eq!(&**pb.data(), b"content");
        assert!(!pb.automatic);
        assert_eq!(pb.order(), order);
        assert_eq!(names(&store), vec!["later", "new"]);
        assert!(store.top().is_none());
    }

    #[test]
    fn rename_over_an_existing_buffer() {
        let mut store = PasteStore::new();
        set(&mut store, "from", b"src");
        set(&mut store, "to", b"dst-old");
        let events = store.rename("from", b"to").unwrap();
        assert_eq!(events, vec![deleted("to"), deleted("from"), changed("to")]);
        assert_eq!(store.by_name.len(), 1);
        assert_eq!(&**store.get("to").unwrap().data(), b"src");
    }

    #[test]
    fn rename_to_itself_does_nothing() {
        let mut store = PasteStore::new();
        set(&mut store, "keep", b"q");
        assert_eq!(store.rename("keep", b"keep"), Ok(Vec::new()));
        assert_eq!(&**store.get("keep").unwrap().data(), b"q");

        // The comparison is with the cleaned name.
        set(&mut store, "a\\b", b"x");
        assert_eq!(store.rename("a\\\\b", b"a\\b"), Ok(Vec::new()));
        assert_eq!(store.by_name.len(), 2);
    }

    #[test]
    fn rename_errors() {
        let mut store = PasteStore::new();
        set(&mut store, "buf", b"x");
        assert_eq!(store.rename("", b"new"), Err(PasteError::NoBuffer));
        assert_eq!(store.rename("buf", b""), Err(PasteError::NewNameEmpty));
        assert_eq!(store.rename("buf", b"n\x01"), Err(PasteError::InvalidName("n\x01".into())));
        assert_eq!(store.rename("nope", b"new"), Err(PasteError::UnknownBuffer("nope".into())));
        // The new name is checked before the old buffer is looked up.
        assert_eq!(store.rename("nope", b"bad\xff"), Err(PasteError::InvalidName("bad\u{fffd}".into())));
        assert_eq!(PasteError::NoBuffer.to_string(), "no buffer");
        assert_eq!(PasteError::NewNameEmpty.to_string(), "new name is empty");
        assert_eq!(PasteError::UnknownBuffer("nope".into()).to_string(), "no buffer nope");
        assert_eq!(names(&store), vec!["buf"]);
    }

    #[test]
    fn replace_keeps_name_and_order() {
        let mut store = PasteStore::new();
        set(&mut store, "buf", b"old");
        let order = store.get("buf").unwrap().order();
        assert_eq!(store.replace("buf", b"new-data".as_slice()), Some(changed("buf")));
        let pb = store.get("buf").unwrap();
        assert_eq!(&**pb.data(), b"new-data");
        assert_eq!(pb.order(), order);
        assert_eq!(store.replace("nope", b"x".as_slice()), None);
    }

    #[test]
    fn handed_out_data_survives_changes() {
        let mut store = PasteStore::new();
        set(&mut store, "buf", b"old");
        let copy = store.get("buf").unwrap().clone();
        store.replace("buf", b"new".as_slice());
        store.delete("buf");
        assert_eq!(&**copy.data(), b"old");
    }

    fn sample(data: &[u8]) -> String {
        let mut store = PasteStore::new();
        set(&mut store, "s", data);
        store.get("s").unwrap().sample()
    }

    #[test]
    fn sample_of_short_text() {
        assert_eq!(sample(b"hello world"), "hello world");
        assert_eq!(sample(b"a\tb\nc\\"), "a\\tb\\nc\\\\");
    }

    #[test]
    fn sample_of_long_text_is_cut() {
        assert_eq!(sample(&[b'x'; 200]), "x".repeat(200));
        assert_eq!(sample(&[b'x'; 300]), format!("{}...", "x".repeat(200)));
    }

    #[test]
    fn sample_is_cut_after_escaping() {
        // Checked against tmux 3.8: 50 bytes of \001 escape to exactly 200
        // bytes and are not cut, 101 bytes are cut to 50 escapes.
        assert_eq!(sample(&[1; 50]), "\\001".repeat(50));
        assert_eq!(sample(&[1; 101]), format!("{}...", "\\001".repeat(50)));
    }
}
