// Copyright (c) 2026 Dane Jensen <dhcjensen@gmail.com>
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
#![forbid(unsafe_code)]
//! Sort orders shared by the list commands, the format loops, next and
//! previous session and the choose modes (sort.c in tmux 3.8).
//!
//! This module never sees an entity. Each kind of thing that can be
//! sorted has a key struct holding the plain values its comparator looks
//! at ([`BufferKey`], [`ClientKey`], [`SessionKey`], [`PaneKey`],
//! [`WinlinkKey`] and [`KeyBindingKey`]). Callers collect a key for each
//! item and sort with [`sort`] or [`sort_by`]. The `sort_get` module does that
//! for the entity registries.
//!
//! Every comparator falls back to a name when its order finds two items
//! equal, so the result rarely depends on the order the items were
//! collected in. Where it does (two panes with the same title), the sort
//! is stable and keeps the collected order.

use std::cmp::Ordering;
use std::fmt;

use thiserror::Error;

/// Which property a list is sorted by.
///
/// Not every kind of item uses every order. A comparator ignores the
/// orders it does not know and compares by name only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SortOrder {
    /// Most recent activity first.
    Activity,
    /// Creation time or creation order.
    Creation,
    /// Index, ID or key code.
    Index,
    /// Key modifiers (key bindings only).
    Modifier,
    /// Name or title.
    Name,
    /// The order the items were collected in, no sorting.
    Order,
    /// Size.
    Size,
    /// Stacking order of panes.
    Z,
}

impl SortOrder {
    /// Parses an order name, ignoring case. "key" is an alias for index
    /// and "title" for name.
    pub fn from_name(name: &str) -> Option<Self> {
        let is = |s: &str| name.eq_ignore_ascii_case(s);
        if is("activity") {
            Some(Self::Activity)
        } else if is("creation") {
            Some(Self::Creation)
        } else if is("index") || is("key") {
            Some(Self::Index)
        } else if is("modifier") {
            Some(Self::Modifier)
        } else if is("name") || is("title") {
            Some(Self::Name)
        } else if is("order") {
            Some(Self::Order)
        } else if is("size") {
            Some(Self::Size)
        } else if is("z") {
            Some(Self::Z)
        } else {
            None
        }
    }

    /// The canonical name of the order, as [`SortOrder::from_name`]
    /// accepts it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Creation => "creation",
            Self::Index => "index",
            Self::Modifier => "modifier",
            Self::Name => "name",
            Self::Order => "order",
            Self::Size => "size",
            Self::Z => "z",
        }
    }
}

impl fmt::Display for SortOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Errors from building sort criteria.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SortError {
    /// The -O argument is not an order name.
    #[error("invalid sort order")]
    InvalidOrder,
}

/// How to sort a list: the order, whether to reverse it, and for the
/// modes the orders a key cycles through.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SortCriteria {
    /// The order, or None to keep the list as it was collected (C's
    /// `SORT_END`). Unlike [`SortOrder::Order`], None ignores `reversed`.
    pub order: Option<SortOrder>,
    /// Reverse the result.
    pub reversed: bool,
    /// The orders [`SortCriteria::next_order`] cycles through. Empty when
    /// the order does not cycle.
    pub order_seq: &'static [SortOrder],
}

impl SortCriteria {
    /// Criteria for one order, without cycling.
    pub fn new(order: Option<SortOrder>, reversed: bool) -> Self {
        Self {
            order,
            reversed,
            order_seq: &[],
        }
    }

    /// Criteria from a command's -O argument (None when it was not given)
    /// and its -r flag. An unknown name is an error, as in C.
    pub fn from_args(order: Option<&str>, reversed: bool) -> Result<Self, SortError> {
        let order = match order {
            Some(name) => Some(SortOrder::from_name(name).ok_or(SortError::InvalidOrder)?),
            None => None,
        };
        Ok(Self::new(order, reversed))
    }

    /// Moves to the next order in `order_seq`, wrapping at the end. An
    /// order that is not in the sequence moves to its first entry. Does
    /// nothing when there is no sequence.
    #[cfg_attr(not(test), expect(dead_code, reason = "used by the choose modes, F1b"))]
    pub fn next_order(&mut self) {
        let Some(&first) = self.order_seq.first() else {
            return;
        };
        let next = self
            .order
            .and_then(|order| self.order_seq.iter().position(|&o| o == order))
            .and_then(|i| self.order_seq.get(i + 1).copied());
        self.order = Some(next.unwrap_or(first));
    }

    /// Compares two keys: by the order, then by the key's fallback, then
    /// reversed if asked.
    pub fn compare<K: SortKey>(&self, a: &K, b: &K) -> Ordering {
        let primary = match self.order {
            Some(order) => a.compare_by(b, order),
            None => Ordering::Equal,
        };
        let result = primary.then_with(|| a.compare_fallback(b));
        if self.reversed {
            result.reverse()
        } else {
            result
        }
    }
}

/// Something that can be sorted: the values one kind of item is compared
/// on.
pub trait SortKey {
    /// Compares by one order. Orders this kind does not use compare equal.
    fn compare_by(&self, other: &Self, order: SortOrder) -> Ordering;

    /// The comparison used when the order finds two items equal.
    fn compare_fallback(&self, other: &Self) -> Ordering;
}

/// Sorts keys in place.
#[cfg_attr(not(test), expect(dead_code, reason = "used by the choose modes, F1b"))]
pub fn sort<K: SortKey>(items: &mut [K], criteria: &SortCriteria) {
    sort_by(items, criteria, |item| item);
}

/// Sorts items in place by the key `key` returns for each.
///
/// With no order the items stay as they are. [`SortOrder::Order`] keeps
/// them as well, but reverses them when the criteria say so. Any other
/// order sorts stably.
pub fn sort_by<T, K: SortKey>(
    items: &mut [T],
    criteria: &SortCriteria,
    key: impl Fn(&T) -> &K,
) {
    match criteria.order {
        None => {}
        Some(SortOrder::Order) => {
            if criteria.reversed {
                items.reverse();
            }
        }
        Some(_) => items.sort_by(|a, b| criteria.compare(key(a), key(b))),
    }
}

/// Whether sorting would put two windows of the tree mode in a different
/// order than their index does. Never true when sorting by index.
#[cfg_attr(not(test), expect(dead_code, reason = "used by the choose modes, F1b"))]
pub fn would_window_tree_swap(criteria: &SortCriteria, a: &WinlinkKey, b: &WinlinkKey) -> bool {
    if criteria.order == Some(SortOrder::Index) {
        return false;
    }
    criteria.compare(a, b) != Ordering::Equal
}

/// A time as seconds and microseconds, compared like C's timercmp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    /// Seconds.
    pub sec: i64,
    /// Microseconds.
    pub usec: i64,
}

/// Compares ASCII case insensitively, like strcasecmp.
/// strcasecmp on bytes.
fn cmp_bytes_ignore_ascii_case(a: &[u8], b: &[u8]) -> Ordering {
    let a = a.iter().map(u8::to_ascii_lowercase);
    let b = b.iter().map(u8::to_ascii_lowercase);
    a.cmp(b)
}

/// A paste buffer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BufferKey {
    /// Buffer name.
    pub name: String,
    /// Creation order, higher is newer.
    pub order: u32,
    /// Size in bytes.
    pub size: usize,
}

impl SortKey for BufferKey {
    /// Creation is newest first. Fallback is the name.
    fn compare_by(&self, other: &Self, order: SortOrder) -> Ordering {
        match order {
            SortOrder::Name => self.name.cmp(&other.name),
            SortOrder::Creation => other.order.cmp(&self.order),
            SortOrder::Size => self.size.cmp(&other.size),
            _ => Ordering::Equal,
        }
    }

    fn compare_fallback(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}

/// A client.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientKey {
    /// Client name.
    pub name: String,
    /// Terminal width.
    pub sx: u32,
    /// Terminal height.
    pub sy: u32,
    /// When the client connected.
    pub creation: Timestamp,
    /// Latest activity.
    pub activity: Timestamp,
}

impl SortKey for ClientKey {
    /// Size is width, then height. Creation is oldest first, activity
    /// most recent first. Fallback is the name.
    fn compare_by(&self, other: &Self, order: SortOrder) -> Ordering {
        match order {
            SortOrder::Name => self.name.cmp(&other.name),
            SortOrder::Size => (self.sx, self.sy).cmp(&(other.sx, other.sy)),
            SortOrder::Creation => self.creation.cmp(&other.creation),
            SortOrder::Activity => other.activity.cmp(&self.activity),
            _ => Ordering::Equal,
        }
    }

    fn compare_fallback(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}

/// A session.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionKey {
    /// Session ID, used as its index.
    pub id: u32,
    /// Session name.
    pub name: String,
    /// When the session was created.
    pub creation: Timestamp,
    /// Latest activity.
    pub activity: Timestamp,
}

impl SortKey for SessionKey {
    /// Index is the ID. Creation is oldest first, activity most recent
    /// first. Fallback is the name.
    fn compare_by(&self, other: &Self, order: SortOrder) -> Ordering {
        match order {
            SortOrder::Index => self.id.cmp(&other.id),
            SortOrder::Creation => self.creation.cmp(&other.creation),
            SortOrder::Activity => other.activity.cmp(&self.activity),
            SortOrder::Name => self.name.cmp(&other.name),
            _ => Ordering::Equal,
        }
    }

    fn compare_fallback(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}

/// A pane.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PaneKey {
    /// Pane ID, used for creation order.
    pub id: u32,
    /// When the pane was last made active, as a counter.
    pub active_point: u32,
    /// Width times height.
    pub size: u64,
    /// Index in its window.
    pub index: u32,
    /// Position in the window's stacking order.
    pub z: u32,
    /// Pane title.
    pub title: String,
}

impl SortKey for PaneKey {
    /// Activity is the active point, least recent first, as in C.
    /// Creation is the ID. Fallback is the title.
    fn compare_by(&self, other: &Self, order: SortOrder) -> Ordering {
        match order {
            SortOrder::Activity => self.active_point.cmp(&other.active_point),
            SortOrder::Creation => self.id.cmp(&other.id),
            SortOrder::Size => self.size.cmp(&other.size),
            SortOrder::Index => self.index.cmp(&other.index),
            SortOrder::Name => self.title.cmp(&other.title),
            SortOrder::Z => self.z.cmp(&other.z),
            _ => Ordering::Equal,
        }
    }

    fn compare_fallback(&self, other: &Self) -> Ordering {
        self.title.cmp(&other.title)
    }
}

/// A winlink and the window it links.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WinlinkKey {
    /// Winlink index.
    pub idx: i32,
    /// Window name.
    pub name: String,
    /// When the window was created.
    pub creation: Timestamp,
    /// Latest window activity.
    pub activity: Timestamp,
    /// Window width times height.
    pub size: u64,
}

impl SortKey for WinlinkKey {
    /// Creation is oldest first, activity most recent first. Fallback is
    /// the window name.
    fn compare_by(&self, other: &Self, order: SortOrder) -> Ordering {
        match order {
            SortOrder::Index => self.idx.cmp(&other.idx),
            SortOrder::Creation => self.creation.cmp(&other.creation),
            SortOrder::Activity => other.activity.cmp(&self.activity),
            SortOrder::Name => self.name.cmp(&other.name),
            SortOrder::Size => self.size.cmp(&other.size),
            _ => Ordering::Equal,
        }
    }

    fn compare_fallback(&self, other: &Self) -> Ordering {
        self.name.cmp(&other.name)
    }
}

/// A key binding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyBindingKey {
    /// Name of the key table.
    pub table: Vec<u8>,
    /// Key code with its flags.
    pub key: u64,
    /// The modifier bits of the key code.
    pub modifiers: u64,
}

impl SortKey for KeyBindingKey {
    /// Index is the key code. Name is the table name, ignoring case.
    /// Fallback is the table name, then the key code.
    fn compare_by(&self, other: &Self, order: SortOrder) -> Ordering {
        match order {
            SortOrder::Index => self.key.cmp(&other.key),
            SortOrder::Modifier => self.modifiers.cmp(&other.modifiers),
            SortOrder::Name => cmp_bytes_ignore_ascii_case(&self.table, &other.table),
            _ => Ordering::Equal,
        }
    }

    fn compare_fallback(&self, other: &Self) -> Ordering {
        cmp_bytes_ignore_ascii_case(&self.table, &other.table).then(self.key.cmp(&other.key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn by(order: SortOrder) -> SortCriteria {
        SortCriteria::new(Some(order), false)
    }

    fn rev(order: SortOrder) -> SortCriteria {
        SortCriteria::new(Some(order), true)
    }

    fn ts(sec: i64, usec: i64) -> Timestamp {
        Timestamp { sec, usec }
    }

    fn buffer(name: &str, order: u32, size: usize) -> BufferKey {
        BufferKey {
            name: name.into(),
            order,
            size,
        }
    }

    fn names<K>(items: &[K], name: impl Fn(&K) -> &str) -> Vec<String> {
        items.iter().map(|k| name(k).to_string()).collect()
    }

    const ALL: [SortOrder; 8] = [
        SortOrder::Activity,
        SortOrder::Creation,
        SortOrder::Index,
        SortOrder::Modifier,
        SortOrder::Name,
        SortOrder::Order,
        SortOrder::Size,
        SortOrder::Z,
    ];

    #[test]
    fn order_names_round_trip() {
        for order in ALL {
            assert_eq!(SortOrder::from_name(order.name()), Some(order));
            assert_eq!(order.to_string(), order.name());
        }
    }

    #[test]
    fn order_names_ignore_case_and_have_aliases() {
        assert_eq!(SortOrder::from_name("NAME"), Some(SortOrder::Name));
        assert_eq!(SortOrder::from_name("Title"), Some(SortOrder::Name));
        assert_eq!(SortOrder::from_name("key"), Some(SortOrder::Index));
        assert_eq!(SortOrder::from_name("Z"), Some(SortOrder::Z));
        assert_eq!(SortOrder::from_name(""), None);
        assert_eq!(SortOrder::from_name("names"), None);
    }

    #[test]
    fn criteria_from_args() {
        assert_eq!(
            SortCriteria::from_args(None, true),
            Ok(SortCriteria::new(None, true))
        );
        assert_eq!(
            SortCriteria::from_args(Some("size"), false),
            Ok(by(SortOrder::Size))
        );
        let err = SortCriteria::from_args(Some("bogus"), false).unwrap_err();
        assert_eq!(err.to_string(), "invalid sort order");
        assert!(SortCriteria::from_args(Some(""), false).is_err());
    }

    #[test]
    fn next_order_cycles_and_wraps() {
        const SEQ: &[SortOrder] = &[SortOrder::Index, SortOrder::Name, SortOrder::Activity];
        let mut crit = SortCriteria {
            order: Some(SortOrder::Index),
            reversed: false,
            order_seq: SEQ,
        };
        crit.next_order();
        assert_eq!(crit.order, Some(SortOrder::Name));
        crit.next_order();
        assert_eq!(crit.order, Some(SortOrder::Activity));
        crit.next_order();
        assert_eq!(crit.order, Some(SortOrder::Index));

        crit.order = Some(SortOrder::Size);
        crit.next_order();
        assert_eq!(crit.order, Some(SortOrder::Index));
        crit.order = None;
        crit.next_order();
        assert_eq!(crit.order, Some(SortOrder::Index));
    }

    #[test]
    fn next_order_without_sequence_does_nothing() {
        let mut crit = by(SortOrder::Name);
        crit.next_order();
        assert_eq!(crit.order, Some(SortOrder::Name));
    }

    #[test]
    fn no_order_keeps_the_list_even_reversed() {
        let mut l = vec![buffer("b", 1, 0), buffer("a", 2, 0), buffer("c", 3, 0)];
        sort(&mut l, &SortCriteria::new(None, true));
        assert_eq!(names(&l, |k| &k.name), ["b", "a", "c"]);
    }

    #[test]
    fn order_keeps_the_list_and_reverses_it() {
        let mut l = vec![buffer("b", 1, 0), buffer("a", 2, 0), buffer("c", 3, 0)];
        sort(&mut l, &by(SortOrder::Order));
        assert_eq!(names(&l, |k| &k.name), ["b", "a", "c"]);
        sort(&mut l, &rev(SortOrder::Order));
        assert_eq!(names(&l, |k| &k.name), ["c", "a", "b"]);
    }

    #[test]
    fn sort_by_sorts_items_by_their_key() {
        let mut l = vec![(buffer("b", 1, 0), 'x'), (buffer("a", 2, 0), 'y')];
        sort_by(&mut l, &by(SortOrder::Name), |(k, _)| k);
        assert_eq!(l[0].1, 'y');
        assert_eq!(l[1].1, 'x');
    }

    #[test]
    fn buffers() {
        let l = vec![
            buffer("b", 1, 30),
            buffer("a", 3, 10),
            buffer("c", 2, 10),
        ];
        let sorted = |crit: SortCriteria| {
            let mut l = l.clone();
            sort(&mut l, &crit);
            names(&l, |k| &k.name)
        };
        assert_eq!(sorted(by(SortOrder::Name)), ["a", "b", "c"]);
        // Creation is newest first.
        assert_eq!(sorted(by(SortOrder::Creation)), ["a", "c", "b"]);
        // Size ties fall back to the name.
        assert_eq!(sorted(by(SortOrder::Size)), ["a", "c", "b"]);
        assert_eq!(sorted(rev(SortOrder::Size)), ["b", "c", "a"]);
        // Orders buffers do not use compare by name.
        assert_eq!(sorted(by(SortOrder::Activity)), ["a", "b", "c"]);
        assert_eq!(sorted(by(SortOrder::Z)), ["a", "b", "c"]);
    }

    #[test]
    fn clients() {
        let client = |name: &str, sx, sy, creation, activity| ClientKey {
            name: name.into(),
            sx,
            sy,
            creation,
            activity,
        };
        let l = vec![
            client("/dev/pts/2", 80, 24, ts(10, 5), ts(20, 0)),
            client("/dev/pts/1", 80, 30, ts(10, 2), ts(30, 0)),
            client("/dev/pts/3", 100, 10, ts(9, 0), ts(20, 0)),
        ];
        let sorted = |crit: SortCriteria| {
            let mut l = l.clone();
            sort(&mut l, &crit);
            names(&l, |k| &k.name)
        };
        assert_eq!(
            sorted(by(SortOrder::Name)),
            ["/dev/pts/1", "/dev/pts/2", "/dev/pts/3"]
        );
        // Width first, then height.
        assert_eq!(
            sorted(by(SortOrder::Size)),
            ["/dev/pts/2", "/dev/pts/1", "/dev/pts/3"]
        );
        // Oldest first, microseconds count.
        assert_eq!(
            sorted(by(SortOrder::Creation)),
            ["/dev/pts/3", "/dev/pts/1", "/dev/pts/2"]
        );
        // Most recent first, ties by name.
        assert_eq!(
            sorted(by(SortOrder::Activity)),
            ["/dev/pts/1", "/dev/pts/2", "/dev/pts/3"]
        );
        assert_eq!(
            sorted(rev(SortOrder::Activity)),
            ["/dev/pts/3", "/dev/pts/2", "/dev/pts/1"]
        );
        assert_eq!(
            sorted(by(SortOrder::Index)),
            ["/dev/pts/1", "/dev/pts/2", "/dev/pts/3"]
        );
    }

    #[test]
    fn sessions() {
        let session = |id, name: &str, creation, activity| SessionKey {
            id,
            name: name.into(),
            creation,
            activity,
        };
        let l = vec![
            session(2, "beta", ts(5, 0), ts(7, 0)),
            session(0, "gamma", ts(3, 0), ts(7, 0)),
            session(1, "alpha", ts(4, 0), ts(9, 0)),
        ];
        let sorted = |crit: SortCriteria| {
            let mut l = l.clone();
            sort(&mut l, &crit);
            names(&l, |k| &k.name)
        };
        assert_eq!(sorted(by(SortOrder::Index)), ["gamma", "alpha", "beta"]);
        assert_eq!(sorted(by(SortOrder::Name)), ["alpha", "beta", "gamma"]);
        assert_eq!(sorted(by(SortOrder::Creation)), ["gamma", "alpha", "beta"]);
        assert_eq!(sorted(by(SortOrder::Activity)), ["alpha", "beta", "gamma"]);
        assert_eq!(sorted(rev(SortOrder::Name)), ["gamma", "beta", "alpha"]);
        assert_eq!(sorted(by(SortOrder::Size)), ["alpha", "beta", "gamma"]);
    }

    #[test]
    fn panes() {
        let pane = |id, active_point, size, index, z, title: &str| PaneKey {
            id,
            active_point,
            size,
            index,
            z,
            title: title.into(),
        };
        let l = vec![
            pane(5, 2, 100, 0, 1, "b"),
            pane(3, 7, 50, 1, 1, "a"),
            pane(4, 1, 100, 2, 0, "c"),
        ];
        let sorted = |crit: SortCriteria| {
            let mut l = l.clone();
            sort(&mut l, &crit);
            names(&l, |k| &k.title)
        };
        // Activity is the active point, lowest first.
        assert_eq!(sorted(by(SortOrder::Activity)), ["c", "b", "a"]);
        assert_eq!(sorted(by(SortOrder::Creation)), ["a", "c", "b"]);
        assert_eq!(sorted(by(SortOrder::Size)), ["a", "b", "c"]);
        assert_eq!(sorted(by(SortOrder::Index)), ["b", "a", "c"]);
        assert_eq!(sorted(by(SortOrder::Name)), ["a", "b", "c"]);
        assert_eq!(sorted(by(SortOrder::Z)), ["c", "a", "b"]);
        assert_eq!(sorted(rev(SortOrder::Index)), ["c", "a", "b"]);
    }

    #[test]
    fn panes_with_the_same_title_keep_their_order() {
        let pane = |id| PaneKey {
            id,
            title: "host".into(),
            ..PaneKey::default()
        };
        let mut l = vec![pane(2), pane(0), pane(1)];
        sort(&mut l, &by(SortOrder::Name));
        assert_eq!(l.iter().map(|k| k.id).collect::<Vec<_>>(), [2, 0, 1]);
        sort(&mut l, &rev(SortOrder::Name));
        assert_eq!(l.iter().map(|k| k.id).collect::<Vec<_>>(), [2, 0, 1]);
    }

    #[test]
    fn winlinks() {
        let winlink = |idx, name: &str, creation, activity, size| WinlinkKey {
            idx,
            name: name.into(),
            creation,
            activity,
            size,
        };
        let l = vec![
            winlink(3, "vim", ts(1, 0), ts(5, 0), 200),
            winlink(1, "top", ts(2, 0), ts(5, 0), 100),
            winlink(-1, "sh", ts(3, 0), ts(8, 0), 200),
        ];
        let sorted = |crit: SortCriteria| {
            let mut l = l.clone();
            sort(&mut l, &crit);
            names(&l, |k| &k.name)
        };
        assert_eq!(sorted(by(SortOrder::Index)), ["sh", "top", "vim"]);
        assert_eq!(sorted(by(SortOrder::Creation)), ["vim", "top", "sh"]);
        assert_eq!(sorted(by(SortOrder::Activity)), ["sh", "top", "vim"]);
        assert_eq!(sorted(by(SortOrder::Name)), ["sh", "top", "vim"]);
        assert_eq!(sorted(by(SortOrder::Size)), ["top", "sh", "vim"]);
        assert_eq!(sorted(rev(SortOrder::Size)), ["vim", "sh", "top"]);
    }

    #[test]
    fn window_tree_swap() {
        let a = WinlinkKey {
            idx: 2,
            name: "a".into(),
            ..WinlinkKey::default()
        };
        let b = WinlinkKey {
            idx: 1,
            name: "b".into(),
            ..WinlinkKey::default()
        };
        assert!(!would_window_tree_swap(&by(SortOrder::Index), &a, &b));
        assert!(would_window_tree_swap(&by(SortOrder::Name), &a, &b));
        assert!(would_window_tree_swap(&by(SortOrder::Size), &a, &b));
        assert!(!would_window_tree_swap(&by(SortOrder::Name), &a, &a));
    }

    #[test]
    fn key_bindings() {
        const META: u64 = 0x0010_0000_0000_0000;
        let binding = |table: &str, key, modifiers| KeyBindingKey {
            table: table.as_bytes().to_vec(),
            key,
            modifiers,
        };
        let l = vec![
            binding("root", 0x61 | META, META),
            binding("Prefix", 0x62, 0),
            binding("prefix", 0x61, 0),
            binding("copy-mode", 0x7a, 0),
        ];
        let sorted = |crit: SortCriteria| {
            let mut l = l.clone();
            sort(&mut l, &crit);
            l.iter()
                .map(|k| format!("{}:{:x}", String::from_utf8_lossy(&k.table), k.key & 0xff))
                .collect::<Vec<_>>()
        };
        // Table names ignore case, ties by key code.
        assert_eq!(
            sorted(by(SortOrder::Name)),
            ["copy-mode:7a", "prefix:61", "Prefix:62", "root:61"]
        );
        assert_eq!(
            sorted(by(SortOrder::Index)),
            ["prefix:61", "Prefix:62", "copy-mode:7a", "root:61"]
        );
        assert_eq!(
            sorted(by(SortOrder::Modifier)),
            ["copy-mode:7a", "prefix:61", "Prefix:62", "root:61"]
        );
        assert_eq!(
            sorted(rev(SortOrder::Modifier)),
            ["root:61", "Prefix:62", "prefix:61", "copy-mode:7a"]
        );
    }

    #[test]
    fn strcasecmp_order() {
        assert_eq!(cmp_bytes_ignore_ascii_case(b"ABC", b"abd"), Ordering::Less);
        assert_eq!(cmp_bytes_ignore_ascii_case(b"abc", b"ABC"), Ordering::Equal);
        assert_eq!(cmp_bytes_ignore_ascii_case(b"ab", b"abc"), Ordering::Less);
        // strcasecmp lowercases, so '_' (0x5f) sorts before 'A' (0x61
        // after lowering), unlike strcmp.
        assert_eq!(cmp_bytes_ignore_ascii_case(b"_", b"A"), Ordering::Less);
    }
}
