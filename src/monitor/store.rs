// Copyright (c) 2026 Nicholas Marriott <nicholas.marriott@gmail.com>
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

//! Format monitors: which watched values changed.
//!
//! A [`MonitorSet`] holds named items, each a format watched on a target:
//! the session, one pane (`%3`), every pane (`%*`), one window (`@1`) or
//! every window (`@*`). Once a second the adapter expands each format
//! where it applies and hands the values to [`MonitorSet::check`], which
//! compares them with the last values and returns what changed. The set
//! keeps a last value per pane or window and its index in the session,
//! since a window can be linked at several indexes.
//!
//! Control mode subscriptions (`refresh-client -B`) use a set per client.
//! Hook monitors (`set-hook -B`, not ported yet) use one per hook.
//!
//! The set does not expand formats or look at entities: values come in
//! as strings and the time as a parameter, changes go out as values.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// What a monitor item watches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MonitorTarget {
    /// The session, once.
    Session,
    /// One pane, by ID, at each index its window has in the session.
    Pane(u32),
    /// Every pane in the session.
    AllPanes,
    /// One window, by ID, at each index it has in the session.
    Window(u32),
    /// Every window in the session.
    AllWindows,
}

/// When a monitor item reports.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MonitorFlags {
    /// Report the first value too, not only changes.
    pub initial: bool,
    /// Report only values that are true (not empty and not "0").
    pub only_true: bool,
}

/// Where a value was seen: the session, or a pane or window at an index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MonitorKey {
    Session,
    Pane { pane: u32, index: i32 },
    Window { window: u32, index: i32 },
}

/// A value the adapter found for an item in this check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub name: String,
    pub key: MonitorKey,
    pub value: String,
}

/// A value to report: it changed, or it is the first one and the item
/// reports first values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MonitorChange {
    pub name: String,
    pub key: MonitorKey,
    pub value: String,
    /// The value before, None for a first value.
    pub last: Option<String>,
}

#[derive(Debug)]
struct Seen {
    last: String,
    generation: u32,
}

#[derive(Debug)]
struct Item {
    format: String,
    target: MonitorTarget,
    flags: MonitorFlags,
    last: BTreeMap<MonitorKey, Seen>,
    fire_count: u32,
    fire_time: i64,
}

/// The monitor items of one owner, by name.
#[derive(Debug, Default)]
pub struct MonitorSet {
    items: BTreeMap<String, Item>,
    generation: u32,
}

/// Splits `name:what:format` into its parts, as C's `monitor_parse`.
/// `what` is empty for the session, `%N` or `%*` for panes, `@N` or `@*`
/// for windows. Anything else does not parse.
pub fn monitor_parse(value: &str) -> Option<(String, MonitorTarget, String)> {
    let (name, rest) = value.split_once(':')?;
    let (what, format) = rest.split_once(':')?;
    let target = match what {
        "" => MonitorTarget::Session,
        "%*" => MonitorTarget::AllPanes,
        "@*" => MonitorTarget::AllWindows,
        _ => {
            let (kind, number) = what.split_at(1);
            let id = parse_id(number)?;
            match kind {
                "%" => MonitorTarget::Pane(id),
                "@" => MonitorTarget::Window(id),
                _ => return None,
            }
        }
    };
    Some((name.to_string(), target, format.to_string()))
}

/// A non-negative ID as C's sscanf("%d") reads it: leading digits, an
/// optional sign, trailing text ignored.
fn parse_id(s: &str) -> Option<u32> {
    let s = s.trim_start();
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let n: i64 = digits[..end].parse().ok()?;
    let n = if negative { -n } else { n };
    u32::try_from(n).ok().filter(|&n| i32::try_from(n).is_ok())
}

/// Whether a value counts as true, as C's `format_true`.
fn value_true(value: &str) -> bool {
    !value.is_empty() && value != "0"
}

impl MonitorSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds an item, replacing any item with the same name and its last
    /// values.
    pub fn add(&mut self, name: &str, target: MonitorTarget, format: &str, flags: MonitorFlags) {
        self.items.insert(
            name.to_string(),
            Item {
                format: format.to_string(),
                target,
                flags,
                last: BTreeMap::new(),
                fire_count: 0,
                fire_time: 0,
            },
        );
    }

    /// Removes an item. Returns false if there was none.
    pub fn remove(&mut self, name: &str) -> bool {
        self.items.remove(name).is_some()
    }

    /// Whether the set has no items. The adapter stops its timer then.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The items as (name, target, format), by name.
    pub fn items(&self) -> impl Iterator<Item = (&str, MonitorTarget, &str)> {
        self.items
            .iter()
            .map(|(name, item)| (name.as_str(), item.target, item.format.as_str()))
    }

    /// How often an item has reported.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by show-hooks -B, after F2"))]
    pub fn fire_count(&self, name: &str) -> u32 {
        self.items.get(name).map_or(0, |item| item.fire_count)
    }

    /// When an item last reported, 0 if never.
    #[cfg_attr(not(test), expect(dead_code, reason = "read by show-hooks -B, after F2"))]
    pub fn fire_time(&self, name: &str) -> i64 {
        self.items.get(name).map_or(0, |item| item.fire_time)
    }

    /// Compares the values of one check with the last ones and returns
    /// the changes to report, in the order of `observed`.
    ///
    /// `observed` holds every value found in this check. For the
    /// all-panes and all-windows items, last values of panes and windows
    /// that were not observed are dropped, so one that comes back is
    /// reported as new. Observations for unknown items are ignored.
    pub fn check(&mut self, observed: Vec<Observation>, now: i64) -> Vec<MonitorChange> {
        self.generation = self.generation.wrapping_add(1).max(1);
        let generation = self.generation;

        let mut changes = Vec::new();
        for o in observed {
            let Some(item) = self.items.get_mut(&o.name) else {
                continue;
            };
            let report = |value: &str| !item.flags.only_true || value_true(value);
            let last = match item.last.get_mut(&o.key) {
                None => {
                    let reported = item.flags.initial && report(&o.value);
                    item.last.insert(
                        o.key,
                        Seen {
                            last: o.value.clone(),
                            generation,
                        },
                    );
                    reported.then_some(None)
                }
                Some(seen) => {
                    seen.generation = generation;
                    if seen.last == o.value {
                        None
                    } else {
                        let reported = report(&o.value);
                        let last = std::mem::replace(&mut seen.last, o.value.clone());
                        reported.then_some(Some(last))
                    }
                }
            };
            if let Some(last) = last {
                item.fire_count += 1;
                item.fire_time = now;
                changes.push(MonitorChange {
                    name: o.name,
                    key: o.key,
                    value: o.value,
                    last,
                });
            }
        }

        for item in self.items.values_mut() {
            if matches!(
                item.target,
                MonitorTarget::AllPanes | MonitorTarget::AllWindows
            ) {
                item.last.retain(|_, seen| seen.generation == generation);
            }
        }
        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INITIAL: MonitorFlags = MonitorFlags {
        initial: true,
        only_true: false,
    };

    fn obs(name: &str, key: MonitorKey, value: &str) -> Observation {
        Observation {
            name: name.into(),
            key,
            value: value.into(),
        }
    }

    fn values(changes: &[MonitorChange]) -> Vec<(&str, &str, Option<&str>)> {
        changes
            .iter()
            .map(|c| (c.name.as_str(), c.value.as_str(), c.last.as_deref()))
            .collect()
    }

    #[test]
    fn parse_targets() {
        let p = |v| monitor_parse(v);
        assert_eq!(
            p("name::#{session_name}"),
            Some(("name".into(), MonitorTarget::Session, "#{session_name}".into()))
        );
        assert_eq!(p("a:%3:x").unwrap().1, MonitorTarget::Pane(3));
        assert_eq!(p("a:%*:x").unwrap().1, MonitorTarget::AllPanes);
        assert_eq!(p("a:@0:x").unwrap().1, MonitorTarget::Window(0));
        assert_eq!(p("a:@*:x").unwrap().1, MonitorTarget::AllWindows);
        // The format may contain colons.
        assert_eq!(p("a::#{?x,a:b,c}").unwrap().2, "#{?x,a:b,c}");
        // sscanf ignores what follows the number.
        assert_eq!(p("a:%7x:f").unwrap().1, MonitorTarget::Pane(7));
        assert_eq!(p("::f"), Some((String::new(), MonitorTarget::Session, "f".into())));
    }

    #[test]
    fn parse_failures() {
        for v in ["name", "name:", "name:%1", "a:bogus:x", "a:%:x", "a:%-1:x", "a:@x:x", "a:$1:x"] {
            assert_eq!(monitor_parse(v), None, "{v}");
        }
    }

    #[test]
    fn first_value_reported_with_initial_flag_only() {
        let mut set = MonitorSet::new();
        set.add("a", MonitorTarget::Session, "f", INITIAL);
        set.add("b", MonitorTarget::Session, "f", MonitorFlags::default());
        let changes = set.check(
            vec![
                obs("a", MonitorKey::Session, "1"),
                obs("b", MonitorKey::Session, "1"),
            ],
            10,
        );
        assert_eq!(values(&changes), [("a", "1", None)]);
        let changes = set.check(
            vec![
                obs("a", MonitorKey::Session, "2"),
                obs("b", MonitorKey::Session, "2"),
            ],
            11,
        );
        assert_eq!(
            values(&changes),
            [("a", "2", Some("1")), ("b", "2", Some("1"))]
        );
        assert_eq!(set.fire_count("a"), 2);
        assert_eq!(set.fire_time("a"), 11);
        assert_eq!(set.fire_count("b"), 1);
        assert_eq!(set.fire_count("none"), 0);
    }

    #[test]
    fn unchanged_values_are_not_reported() {
        let mut set = MonitorSet::new();
        set.add("a", MonitorTarget::Session, "f", INITIAL);
        set.check(vec![obs("a", MonitorKey::Session, "x")], 1);
        assert!(set.check(vec![obs("a", MonitorKey::Session, "x")], 2).is_empty());
    }

    #[test]
    fn only_true_values_but_last_still_updates() {
        let mut set = MonitorSet::new();
        let flags = MonitorFlags {
            initial: true,
            only_true: true,
        };
        set.add("a", MonitorTarget::Session, "f", flags);
        assert!(set.check(vec![obs("a", MonitorKey::Session, "0")], 1).is_empty());
        let changes = set.check(vec![obs("a", MonitorKey::Session, "1")], 2);
        assert_eq!(values(&changes), [("a", "1", Some("0"))]);
        assert!(set.check(vec![obs("a", MonitorKey::Session, "")], 3).is_empty());
        let changes = set.check(vec![obs("a", MonitorKey::Session, "yes")], 4);
        assert_eq!(values(&changes), [("a", "yes", Some(""))]);
    }

    #[test]
    fn values_are_kept_per_key() {
        let mut set = MonitorSet::new();
        set.add("w", MonitorTarget::Window(1), "f", INITIAL);
        let at = |index| MonitorKey::Window { window: 1, index };
        let changes = set.check(vec![obs("w", at(0), "x"), obs("w", at(4), "x")], 1);
        assert_eq!(changes.len(), 2);
        let changes = set.check(vec![obs("w", at(0), "x"), obs("w", at(4), "y")], 2);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].key, at(4));
    }

    #[test]
    fn all_items_forget_what_is_not_seen() {
        let mut set = MonitorSet::new();
        set.add("aw", MonitorTarget::AllWindows, "f", INITIAL);
        set.add("w", MonitorTarget::Window(1), "f", INITIAL);
        let at = |index| MonitorKey::Window { window: 1, index };
        set.check(vec![obs("aw", at(0), "x"), obs("w", at(0), "x")], 1);
        // The window moves to index 5.
        let changes = set.check(vec![obs("aw", at(5), "x"), obs("w", at(5), "x")], 2);
        assert_eq!(changes.len(), 2);
        // And back: the all-windows item forgot index 0, the single
        // window item did not.
        let changes = set.check(vec![obs("aw", at(0), "x"), obs("w", at(0), "x")], 3);
        assert_eq!(values(&changes), [("aw", "x", None)]);
    }

    #[test]
    fn changes_keep_the_observation_order() {
        let mut set = MonitorSet::new();
        for name in ["a", "b", "c"] {
            set.add(name, MonitorTarget::Session, "f", INITIAL);
        }
        let changes = set.check(
            vec![
                obs("c", MonitorKey::Session, "1"),
                obs("a", MonitorKey::Session, "1"),
                obs("b", MonitorKey::Session, "1"),
            ],
            1,
        );
        let names: Vec<&str> = changes.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["c", "a", "b"]);
    }

    #[test]
    fn replace_and_remove() {
        let mut set = MonitorSet::new();
        assert!(set.is_empty());
        set.add("a", MonitorTarget::Session, "f", INITIAL);
        set.check(vec![obs("a", MonitorKey::Session, "x")], 1);
        // Replacing forgets the last value, so the value is new again.
        set.add("a", MonitorTarget::AllPanes, "g", INITIAL);
        assert_eq!(set.fire_count("a"), 0);
        let items: Vec<_> = set.items().collect();
        assert_eq!(items, [("a", MonitorTarget::AllPanes, "g")]);
        let key = MonitorKey::Pane { pane: 2, index: 0 };
        assert_eq!(set.check(vec![obs("a", key, "x")], 2).len(), 1);
        assert!(set.check(vec![obs("gone", key, "x")], 3).is_empty());
        assert!(set.remove("a"));
        assert!(!set.remove("a"));
        assert!(set.is_empty());
    }
}
