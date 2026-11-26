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

//! Key tables, the store part of key-bindings.c from tmux 3.8 in safe
//! Rust.
//!
//! A key table is a named set of bindings from a key to a command list.
//! Each table also keeps the default bindings, a copy of what the
//! built-in bindings set up, so a key can be reset to its default. A
//! table is created by its first binding and dropped when it has neither
//! bindings nor defaults left, or when it is removed as a whole.
//!
//! Tables have an ID that is never reused. A client refers to its table
//! by ID: when the table is dropped under it, the ID finds nothing, which
//! is how 3.8 behaves (the client keeps an empty table that nobody else
//! can reach).
//!
//! Names and notes are bytes. Keys come in with the flags already masked
//! off. The store is generic over the command list type, which the
//! adapter supplies, and clones bindings out so no caller holds a
//! reference into it. The time comes in as an argument.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::time::Duration;

/// A key code with its flags masked off.
pub type KeyCode = u64;

/// Identifies a key table. Never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyTableId(pub u32);

/// One binding: a key and what it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyBinding<C> {
    pub key: KeyCode,
    /// The commands it runs.
    pub cmds: C,
    /// The note shown by list-keys -N.
    pub note: Option<Vec<u8>>,
    /// Whether it repeats (bind-key -r).
    pub repeat: bool,
}

/// One key table.
#[derive(Clone, Debug)]
struct KeyTable<C> {
    id: KeyTableId,
    /// When a client last entered the table, since the epoch.
    activity: Duration,
    bindings: BTreeMap<KeyCode, KeyBinding<C>>,
    defaults: BTreeMap<KeyCode, KeyBinding<C>>,
}

/// All key tables, by name.
#[derive(Clone, Debug)]
pub struct KeyTables<C> {
    tables: BTreeMap<Vec<u8>, KeyTable<C>>,
    names: BTreeMap<KeyTableId, Vec<u8>>,
    next_id: u32,
}

impl<C> Default for KeyTables<C> {
    fn default() -> Self {
        Self {
            tables: BTreeMap::new(),
            names: BTreeMap::new(),
            next_id: 0,
        }
    }
}

impl<C: Clone> KeyTables<C> {
    /// No tables.
    pub fn new() -> Self {
        Self::default()
    }

    /// The ID of the table called `name`, if it exists.
    pub fn id(&self, name: &[u8]) -> Option<KeyTableId> {
        self.tables.get(name).map(|t| t.id)
    }

    /// The ID of the table called `name`, created empty if it does not
    /// exist.
    pub fn get_or_create(&mut self, name: &[u8]) -> KeyTableId {
        if let Some(id) = self.id(name) {
            return id;
        }
        let id = KeyTableId(self.next_id);
        self.next_id += 1;
        self.tables.insert(
            name.to_vec(),
            KeyTable {
                id,
                activity: Duration::ZERO,
                bindings: BTreeMap::new(),
                defaults: BTreeMap::new(),
            },
        );
        self.names.insert(id, name.to_vec());
        id
    }

    /// The name of table `id`, None if it was dropped.
    pub fn name(&self, id: KeyTableId) -> Option<&[u8]> {
        self.names.get(&id).map(Vec::as_slice)
    }

    /// The table names in order.
    pub fn names(&self) -> impl Iterator<Item = &[u8]> {
        self.tables.keys().map(Vec::as_slice)
    }

    fn table(&self, id: KeyTableId) -> Option<&KeyTable<C>> {
        self.tables.get(self.names.get(&id)?)
    }

    /// When a client last entered table `id`.
    pub fn activity(&self, id: KeyTableId) -> Option<Duration> {
        self.table(id).map(|t| t.activity)
    }

    /// Records that a client entered table `id` at `now`.
    pub fn set_activity(&mut self, id: KeyTableId, now: Duration) {
        if let Some(name) = self.names.get(&id)
            && let Some(table) = self.tables.get_mut(name)
        {
            table.activity = now;
        }
    }

    /// The binding of `key` in table `id`.
    pub fn get(&self, id: KeyTableId, key: KeyCode) -> Option<KeyBinding<C>> {
        self.table(id)?.bindings.get(&key).cloned()
    }

    /// The binding of `key` in the table called `name`.
    pub fn get_by_name(&self, name: &[u8], key: KeyCode) -> Option<KeyBinding<C>> {
        self.tables.get(name)?.bindings.get(&key).cloned()
    }

    /// The default binding of `key` in the table called `name`.
    pub fn get_default(&self, name: &[u8], key: KeyCode) -> Option<KeyBinding<C>> {
        self.tables.get(name)?.defaults.get(&key).cloned()
    }

    /// The bindings of the table called `name`, in key order.
    pub fn bindings(&self, name: &[u8]) -> Vec<KeyBinding<C>> {
        self.tables
            .get(name)
            .map(|t| t.bindings.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Whether the table called `name` has bindings or defaults.
    fn is_empty(&self, name: &[u8]) -> bool {
        self.tables
            .get(name)
            .is_none_or(|t| t.bindings.is_empty() && t.defaults.is_empty())
    }

    /// Binds `key` in the table called `name`, creating the table
    /// (`key_bindings_add`). With `cmds` the binding is replaced. Without,
    /// an existing binding gets the note if one is given and the repeat
    /// flag if `repeat`, and nothing else changes.
    pub fn add(&mut self, name: &[u8], key: KeyCode, note: Option<&[u8]>, repeat: bool, cmds: Option<C>) {
        self.get_or_create(name);
        let table = self.tables.get_mut(name).expect("just created");
        match cmds {
            None => {
                if let Some(bd) = table.bindings.get_mut(&key) {
                    if let Some(note) = note {
                        bd.note = Some(note.to_vec());
                    }
                    if repeat {
                        bd.repeat = true;
                    }
                }
            }
            Some(cmds) => {
                table.bindings.insert(
                    key,
                    KeyBinding {
                        key,
                        cmds,
                        note: note.map(<[u8]>::to_vec),
                        repeat,
                    },
                );
            }
        }
    }

    /// Unbinds `key` in the table called `name`, dropping the table when
    /// it has neither bindings nor defaults left. Returns whether a
    /// binding was removed.
    pub fn remove(&mut self, name: &[u8], key: KeyCode) -> bool {
        let Some(table) = self.tables.get_mut(name) else {
            return false;
        };
        if table.bindings.remove(&key).is_none() {
            return false;
        }
        if self.is_empty(name) {
            self.drop_table(name);
        }
        true
    }

    /// Puts `key` in the table called `name` back to its default, or
    /// unbinds it if it has none.
    pub fn reset(&mut self, name: &[u8], key: KeyCode) {
        let Some(table) = self.tables.get_mut(name) else {
            return;
        };
        if !table.bindings.contains_key(&key) {
            return;
        }
        match table.defaults.get(&key).cloned() {
            Some(default) => {
                table.bindings.insert(key, default);
            }
            None => {
                self.remove(name, key);
            }
        }
    }

    /// Removes the table called `name` with its defaults. Returns its ID,
    /// so the caller can move the clients that are in it.
    pub fn remove_table(&mut self, name: &[u8]) -> Option<KeyTableId> {
        let id = self.id(name)?;
        self.drop_table(name);
        Some(id)
    }

    fn drop_table(&mut self, name: &[u8]) {
        if let Some(table) = self.tables.remove(name) {
            self.names.remove(&table.id);
        }
    }

    /// Makes the current bindings of every table its defaults, once the
    /// built-in bindings are set up (`key_bindings_init_done`).
    pub fn snapshot_defaults(&mut self) {
        for table in self.tables.values_mut() {
            for (key, bd) in &table.bindings {
                table.defaults.insert(*key, bd.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Tables = KeyTables<&'static str>;

    fn listing(t: &Tables, name: &[u8]) -> Vec<(KeyCode, &'static str, Option<String>, bool)> {
        t.bindings(name)
            .into_iter()
            .map(|b| {
                (
                    b.key,
                    b.cmds,
                    b.note.map(|n| String::from_utf8(n).unwrap()),
                    b.repeat,
                )
            })
            .collect()
    }

    #[test]
    fn add_creates_and_replaces() {
        let mut t = Tables::new();
        t.add(b"prefix", 2, Some(b"two"), false, Some("b"));
        t.add(b"prefix", 1, None, true, Some("a"));
        t.add(b"prefix", 2, None, false, Some("c"));
        assert_eq!(
            listing(&t, b"prefix"),
            [(1, "a", None, true), (2, "c", None, false)]
        );
        assert_eq!(t.get_by_name(b"prefix", 1).unwrap().cmds, "a");
        assert!(t.get_by_name(b"prefix", 3).is_none());
        assert!(t.get_by_name(b"root", 1).is_none());
    }

    #[test]
    fn add_without_commands_changes_only_what_is_given() {
        let mut t = Tables::new();
        t.add(b"prefix", 1, Some(b"note"), false, Some("a"));
        t.add(b"prefix", 1, None, true, None);
        assert_eq!(listing(&t, b"prefix"), [(1, "a", Some("note".into()), true)]);
        t.add(b"prefix", 1, Some(b"other"), false, None);
        assert_eq!(listing(&t, b"prefix"), [(1, "a", Some("other".into()), true)]);
        // A missing binding is not created, but the table is (as C).
        t.add(b"new", 1, Some(b"x"), true, None);
        assert!(t.id(b"new").is_some());
        assert!(listing(&t, b"new").is_empty());
    }

    #[test]
    fn ids_are_never_reused() {
        let mut t = Tables::new();
        t.add(b"mine", 1, None, false, Some("a"));
        let first = t.id(b"mine").unwrap();
        assert_eq!(t.name(first), Some(b"mine".as_slice()));
        assert!(t.remove(b"mine", 1));
        // The last binding went, so the table did.
        assert_eq!(t.id(b"mine"), None);
        assert_eq!(t.name(first), None);
        assert!(t.get(first, 1).is_none());
        t.add(b"mine", 2, None, false, Some("b"));
        let second = t.id(b"mine").unwrap();
        assert_ne!(first, second);
        assert!(t.get(first, 2).is_none());
        assert_eq!(t.get(second, 2).unwrap().cmds, "b");
    }

    #[test]
    fn defaults_keep_a_table_and_reset_keys() {
        let mut t = Tables::new();
        t.add(b"prefix", 1, Some(b"n"), false, Some("default"));
        t.snapshot_defaults();
        t.add(b"prefix", 1, None, false, Some("changed"));
        t.add(b"prefix", 2, None, false, Some("extra"));
        t.reset(b"prefix", 1);
        t.reset(b"prefix", 2);
        assert_eq!(listing(&t, b"prefix"), [(1, "default", Some("n".into()), false)]);
        assert_eq!(t.get_default(b"prefix", 1).unwrap().cmds, "default");
        // Without bindings the defaults keep the table.
        assert!(t.remove(b"prefix", 1));
        assert!(t.id(b"prefix").is_some());
        assert!(!t.remove(b"prefix", 1));
    }

    #[test]
    fn remove_table_returns_its_id() {
        let mut t = Tables::new();
        t.add(b"prefix", 1, None, false, Some("a"));
        t.snapshot_defaults();
        let id = t.id(b"prefix").unwrap();
        assert_eq!(t.remove_table(b"prefix"), Some(id));
        assert_eq!(t.remove_table(b"prefix"), None);
        assert!(t.get_default(b"prefix", 1).is_none());
    }

    #[test]
    fn activity_and_byte_names() {
        let mut t = Tables::new();
        let id = t.get_or_create(b"x\xff");
        assert_eq!(t.get_or_create(b"x\xff"), id);
        t.set_activity(id, Duration::from_secs(5));
        assert_eq!(t.activity(id), Some(Duration::from_secs(5)));
        t.get_or_create(b"a");
        let names: Vec<&[u8]> = t.names().collect();
        assert_eq!(names, [b"a".as_slice(), b"x\xff"]);
    }
}
