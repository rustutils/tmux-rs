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

//! Which sinks get an event.
//!
//! A sink is registered under an event name and gets every event fired
//! with that name. [`EventBus`] keeps the sinks in the order they were
//! added, which is the order they are called in, and decides who gets an
//! event. It does not call anyone: [`EventBus::begin`] returns the sinks
//! and the events adapter calls them, so a sink can add and remove sinks
//! while an event is delivered without the bus being borrowed.
//!
//! The rules are those of C's events.c:
//!
//! - A sink added while an event is delivered does not get that event.
//!   `begin` returns the sinks registered when it was called, C compares
//!   generations for the same effect.
//! - A sink removed while an event is delivered gets nothing more, even
//!   from the event in progress. The adapter checks [`EventBus::is_live`]
//!   before each call. It is only marked dead then and freed when the
//!   outermost delivery ends, since an event can fire inside a sink.
#![forbid(unsafe_code)]

/// Who gets an event, called by the events adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink {
    /// Control mode notifications, one sink per notification event.
    Control,
    /// Hooks, one sink per hook name.
    Hooks,
    /// A `wait-for -E` waiter, by its number.
    WaitFor(u32),
}

/// Identifies a registered sink, to remove it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SinkId(u64);

#[derive(Debug)]
struct Entry {
    id: SinkId,
    name: String,
    sink: Sink,
    dead: bool,
}

/// The registered sinks.
#[derive(Debug, Default)]
pub struct EventBus {
    sinks: Vec<Entry>,
    /// How many deliveries are in progress (they nest).
    dispatching: u32,
    /// The last sink id handed out.
    last_id: u64,
}

impl EventBus {
    /// A bus with no sinks.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a sink for events called `name`. It is called after the sinks
    /// already there.
    pub fn add(&mut self, name: &str, sink: Sink) -> SinkId {
        self.last_id += 1;
        let id = SinkId(self.last_id);
        self.sinks.push(Entry {
            id,
            name: name.to_string(),
            sink,
            dead: false,
        });
        id
    }

    /// Removes a sink. While a delivery is in progress it is only marked,
    /// so the sinks [`begin`](Self::begin) returned stay valid, and it is
    /// freed when the delivery ends.
    pub fn remove(&mut self, id: SinkId) {
        if self.dispatching == 0 {
            self.sinks.retain(|e| e.id != id);
        } else if let Some(e) = self.sinks.iter_mut().find(|e| e.id == id) {
            e.dead = true;
        }
    }

    /// Whether a sink is registered for `name`.
    pub fn has(&self, name: &str, sink: Sink) -> bool {
        self.sinks
            .iter()
            .any(|e| !e.dead && e.sink == sink && e.name == name)
    }

    /// Starts delivering an event called `name` and returns the sinks to
    /// call, in order. Each call to `begin` needs a call to
    /// [`end`](Self::end) after the sinks were called.
    pub fn begin(&mut self, name: &str) -> Vec<(SinkId, Sink)> {
        self.dispatching += 1;
        self.sinks
            .iter()
            .filter(|e| !e.dead && e.name == name)
            .map(|e| (e.id, e.sink))
            .collect()
    }

    /// Whether a sink returned by [`begin`](Self::begin) should still be
    /// called. False once it was removed.
    pub fn is_live(&self, id: SinkId) -> bool {
        self.sinks.iter().any(|e| e.id == id && !e.dead)
    }

    /// Ends a delivery. When the outermost one ends, sinks removed during
    /// it are freed.
    pub fn end(&mut self) {
        debug_assert!(self.dispatching > 0, "end without begin");
        self.dispatching = self.dispatching.saturating_sub(1);
        if self.dispatching == 0 {
            self.sinks.retain(|e| !e.dead);
        }
    }

    /// How many sinks are registered, dead ones included.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.sinks.len()
    }

    /// Whether no sinks are registered.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.sinks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deliver(bus: &mut EventBus, name: &str) -> Vec<Sink> {
        let sinks = bus.begin(name);
        let out = sinks
            .into_iter()
            .filter(|(id, _)| bus.is_live(*id))
            .map(|(_, s)| s)
            .collect();
        bus.end();
        out
    }

    #[test]
    fn delivers_by_name_in_order() {
        let mut bus = EventBus::new();
        bus.add("window-renamed", Sink::Control);
        bus.add("pane-bell", Sink::Hooks);
        bus.add("window-renamed", Sink::Hooks);
        bus.add("window-renamed", Sink::WaitFor(4));
        assert_eq!(
            deliver(&mut bus, "window-renamed"),
            [Sink::Control, Sink::Hooks, Sink::WaitFor(4)]
        );
        assert_eq!(deliver(&mut bus, "pane-bell"), [Sink::Hooks]);
        assert!(deliver(&mut bus, "nothing").is_empty());
    }

    #[test]
    fn remove_outside_delivery_frees_at_once() {
        let mut bus = EventBus::new();
        let id = bus.add("a", Sink::WaitFor(1));
        bus.add("a", Sink::Hooks);
        bus.remove(id);
        assert_eq!(bus.len(), 1);
        assert_eq!(deliver(&mut bus, "a"), [Sink::Hooks]);
    }

    #[test]
    fn sink_added_during_delivery_misses_it() {
        let mut bus = EventBus::new();
        bus.add("a", Sink::Hooks);
        let sinks = bus.begin("a");
        bus.add("a", Sink::WaitFor(1));
        assert_eq!(sinks.len(), 1);
        bus.end();
        assert_eq!(deliver(&mut bus, "a"), [Sink::Hooks, Sink::WaitFor(1)]);
    }

    #[test]
    fn sink_removed_during_delivery_is_skipped_and_freed_after() {
        let mut bus = EventBus::new();
        bus.add("a", Sink::Hooks);
        let waiter = bus.add("a", Sink::WaitFor(1));
        let sinks = bus.begin("a");
        // The first sink removes the second while it runs.
        assert!(bus.is_live(sinks[0].0));
        bus.remove(waiter);
        assert!(!bus.is_live(sinks[1].0));
        assert_eq!(bus.len(), 2);
        assert!(!bus.has("a", Sink::WaitFor(1)));
        bus.end();
        assert_eq!(bus.len(), 1);
    }

    #[test]
    fn nested_delivery_frees_only_at_the_outermost_end() {
        let mut bus = EventBus::new();
        let a = bus.add("a", Sink::Hooks);
        bus.add("b", Sink::Control);
        let outer = bus.begin("a");
        let inner = bus.begin("b");
        bus.remove(a);
        bus.end();
        assert_eq!(inner.len(), 1);
        assert_eq!(bus.len(), 2, "still delivering the outer event");
        assert!(!bus.is_live(outer[0].0));
        bus.end();
        assert_eq!(bus.len(), 1);
    }

    #[test]
    fn has_finds_live_sinks() {
        let mut bus = EventBus::new();
        assert!(bus.is_empty());
        let id = bus.add("@mine", Sink::Hooks);
        assert!(bus.has("@mine", Sink::Hooks));
        assert!(!bus.has("@mine", Sink::Control));
        assert!(!bus.has("@other", Sink::Hooks));
        bus.remove(id);
        assert!(!bus.has("@mine", Sink::Hooks));
    }
}
