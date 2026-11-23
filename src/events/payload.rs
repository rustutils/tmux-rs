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

//! What an event carries: named values and a target.
//!
//! Every event has a [`Payload`]. Its items are named values (strings,
//! numbers, or a client, session, window or pane) kept sorted by
//! name, as C's tree of items is. Hooks see them as format variables
//! (`hook_session`, `hook_old_name`), `wait-for -E -v` prints them as
//! `name=value` lines, and `wait-for -F` filters on them.
//!
//! Entities are held as an ID and, for clients, sessions and windows, the
//! name they had when the event fired. C keeps a reference on the entity
//! so it can still read the name after the entity is gone, the snapshot
//! does the same without one. Code that needs the entity itself looks the
//! ID up and finds nothing once it is destroyed.
//!
//! Names starting with `_` are internal: they are never printed and never
//! become format variables.
//!
//! The target is where the event happened, as IDs and a window index. The
//! events adapter turns it back into a command target, falling back the
//! way C's `event_payload_get_target` does when parts of it are gone.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;

use crate::{ClientId, PaneId, SessionId, WindowId};

/// One value in a payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PayloadValue {
    String(String),
    Int(i32),
    Uint(u32),
    Client { id: ClientId, name: String },
    Session { id: SessionId, name: String },
    Window { id: WindowId, name: String },
    Pane(PaneId),
}

impl fmt::Display for PayloadValue {
    /// Prints the value as C's `event_payload_item_print` does: a client by
    /// its name, entities by their ID with the prefix `$`, `@` or `%`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(s) => f.write_str(s),
            Self::Int(n) => write!(f, "{n}"),
            Self::Uint(n) => write!(f, "{n}"),
            Self::Client { name, .. } => f.write_str(name),
            Self::Session { id, .. } => write!(f, "${}", id.0),
            Self::Window { id, .. } => write!(f, "@{}", id.0),
            Self::Pane(id) => write!(f, "%{}", id.0),
        }
    }
}

/// Where an event happened.
///
/// `index` is the window's index in `session`. C keeps it apart from the
/// winlink so the target still means something after the winlink is gone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    pub session: Option<SessionId>,
    pub window: Option<WindowId>,
    pub pane: Option<PaneId>,
    pub index: Option<i32>,
}

/// The values and target of one event.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Payload {
    items: BTreeMap<String, PayloadValue>,
    target: Target,
}

impl Payload {
    /// An empty payload with no target.
    pub fn new() -> Self {
        Self::default()
    }

    /// The target.
    pub fn target(&self) -> Target {
        self.target
    }

    /// Replaces the target.
    pub fn set_target(&mut self, target: Target) {
        self.target = target;
    }

    /// Sets an item, replacing any item with the same name.
    pub fn set(&mut self, name: &str, value: PayloadValue) {
        self.items.insert(name.to_string(), value);
    }

    /// Sets a string item.
    pub fn set_string(&mut self, name: &str, value: impl Into<String>) {
        self.set(name, PayloadValue::String(value.into()));
    }

    /// Sets a signed number item.
    pub fn set_int(&mut self, name: &str, value: i32) {
        self.set(name, PayloadValue::Int(value));
    }

    /// Sets an unsigned number item.
    pub fn set_uint(&mut self, name: &str, value: u32) {
        self.set(name, PayloadValue::Uint(value));
    }

    /// The item called `name`.
    pub fn get(&self, name: &str) -> Option<&PayloadValue> {
        self.items.get(name)
    }

    /// The string item called `name`. None if it is missing or not a
    /// string.
    pub fn get_string(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            PayloadValue::String(s) => Some(s),
            _ => None,
        }
    }

    /// The client item called `name`.
    pub fn get_client(&self, name: &str) -> Option<ClientId> {
        match self.get(name)? {
            PayloadValue::Client { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// The session item called `name`.
    pub fn get_session(&self, name: &str) -> Option<SessionId> {
        match self.get(name)? {
            PayloadValue::Session { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// The window item called `name`.
    pub fn get_window(&self, name: &str) -> Option<WindowId> {
        match self.get(name)? {
            PayloadValue::Window { id, .. } => Some(*id),
            _ => None,
        }
    }

    /// The item called `name` printed, as C's `event_payload_print`.
    pub fn print(&self, name: &str) -> Option<String> {
        self.get(name).map(ToString::to_string)
    }

    /// The items that are not internal, sorted by name.
    pub fn visible(&self) -> impl Iterator<Item = (&str, &PayloadValue)> {
        self.items
            .iter()
            .filter(|(name, _)| !name.starts_with('_'))
            .map(|(name, value)| (name.as_str(), value))
    }

    /// The `name=value` lines `wait-for -E -v` prints.
    pub fn lines(&self) -> Vec<String> {
        self.visible()
            .map(|(name, value)| format!("{name}={value}"))
            .collect()
    }

    /// The format variables for the items, each name with `prefix` in
    /// front, as C's `event_payload_add_formats`. Sessions and windows also
    /// give a `_name` variable with their name.
    pub fn formats(&self, prefix: &str) -> Vec<(String, String)> {
        let mut out = Vec::new();
        for (name, value) in self.visible() {
            out.push((format!("{prefix}{name}"), value.to_string()));
            match value {
                PayloadValue::Session { name: n, .. } | PayloadValue::Window { name: n, .. } => {
                    out.push((format!("{prefix}{name}_name"), n.clone()));
                }
                _ => {}
            }
        }
        out
    }
}

impl fmt::Display for Payload {
    /// All items, internal ones included, for the log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for (name, value) in &self.items {
            if !first {
                f.write_str(", ")?;
            }
            first = false;
            write!(f, "{name}={value}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Payload {
        let mut p = Payload::new();
        p.set(
            "session",
            PayloadValue::Session {
                id: SessionId(3),
                name: "work".into(),
            },
        );
        p.set(
            "window",
            PayloadValue::Window {
                id: WindowId(7),
                name: "vim".into(),
            },
        );
        p.set("pane", PayloadValue::Pane(PaneId(12)));
        p.set(
            "client",
            PayloadValue::Client {
                id: ClientId(1),
                name: "/dev/pts/4".into(),
            },
        );
        p.set_int("window_index", -1);
        p.set_uint("width", 80);
        p.set_string("_internal", "hidden");
        p
    }

    #[test]
    fn values_print_as_in_c() {
        let p = sample();
        assert_eq!(p.print("session").as_deref(), Some("$3"));
        assert_eq!(p.print("window").as_deref(), Some("@7"));
        assert_eq!(p.print("pane").as_deref(), Some("%12"));
        assert_eq!(p.print("client").as_deref(), Some("/dev/pts/4"));
        assert_eq!(p.print("window_index").as_deref(), Some("-1"));
        assert_eq!(p.print("width").as_deref(), Some("80"));
        assert_eq!(p.print("missing"), None);
    }

    #[test]
    fn lines_are_sorted_and_skip_internal_items() {
        let p = sample();
        assert_eq!(
            p.lines(),
            [
                "client=/dev/pts/4",
                "pane=%12",
                "session=$3",
                "width=80",
                "window=@7",
                "window_index=-1",
            ]
        );
    }

    #[test]
    fn formats_add_prefix_and_names() {
        let p = sample();
        let formats = p.formats("hook_");
        let get = |k: &str| {
            formats
                .iter()
                .find(|(name, _)| name == k)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("hook_session"), Some("$3"));
        assert_eq!(get("hook_session_name"), Some("work"));
        assert_eq!(get("hook_window"), Some("@7"));
        assert_eq!(get("hook_window_name"), Some("vim"));
        assert_eq!(get("hook_pane"), Some("%12"));
        assert_eq!(get("hook_pane_name"), None);
        assert_eq!(get("hook_client_name"), None);
        assert_eq!(get("hook__internal"), None);
        assert_eq!(formats.len(), 8);
    }

    #[test]
    fn set_replaces_and_getters_check_the_type() {
        let mut p = Payload::new();
        p.set_string("value", "a");
        p.set_string("value", "b");
        assert_eq!(p.get_string("value"), Some("b"));
        p.set_int("value", 4);
        assert_eq!(p.get_string("value"), None);
        assert_eq!(p.print("value").as_deref(), Some("4"));
        assert_eq!(p.get_session("value"), None);

        let p = sample();
        assert_eq!(p.get_session("session"), Some(SessionId(3)));
        assert_eq!(p.get_window("window"), Some(WindowId(7)));
        assert_eq!(p.get_client("client"), Some(ClientId(1)));
        assert_eq!(p.get_window("session"), None);
    }

    #[test]
    fn target_starts_empty() {
        let mut p = Payload::new();
        assert_eq!(p.target(), Target::default());
        let t = Target {
            session: Some(SessionId(1)),
            window: Some(WindowId(2)),
            pane: None,
            index: Some(0),
        };
        p.set_target(t);
        assert_eq!(p.target(), t);
    }

    #[test]
    fn display_shows_every_item() {
        let mut p = Payload::new();
        p.set_string("event", "pane-bell");
        p.set_string("_cmd", "x");
        assert_eq!(p.to_string(), "_cmd=x, event=pane-bell");
    }
}
