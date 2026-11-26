//! Links between sessions and windows, the topology part of session.c and
//! window.c from tmux 3.8 in safe Rust.
//!
//! A winlink puts a window into a session at an index. A window can be
//! linked into several sessions, and into one session more than once.
//! Each session has its winlinks by index, a current winlink and a stack
//! of the windows used before it (the last window is the top). Sessions
//! in a group share their windows: the group's sessions are kept with the
//! same windows at the same indexes by [`Links::sync`].
//!
//! The store knows sessions and windows only by ID. It does not fire
//! events or touch the windows: operations that change links return what
//! happened ([`LinkEvent`]) in C's order, and the caller takes and drops
//! window references and fires the events. Winlink IDs are never reused,
//! so an ID kept after its winlink went finds nothing.
#![forbid(unsafe_code)]
#![allow(dead_code, reason = "the session adapter starts using it in the next change")]

use std::collections::BTreeMap;

use crate::{SessionId, WINLINK_ALERTFLAGS, WindowId, winlink_flags};

/// Identifies a winlink. Never reused.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WinlinkId(pub u32);

/// A copy of a winlink.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Winlink {
    pub id: WinlinkId,
    /// Index in its session.
    pub idx: i32,
    pub session: SessionId,
    pub window: WindowId,
    /// Alert flags, and `WINLINK_VISITED` while it is on the session's last
    /// window stack.
    pub flags: winlink_flags,
}

/// The links of one session.
#[derive(Clone, Debug, Default)]
struct SessionLinks {
    /// Winlinks by index.
    windows: BTreeMap<i32, WinlinkId>,
    /// The current winlink. None only while the session has no windows
    /// (or is being built or destroyed).
    curw: Option<WinlinkId>,
    /// Windows used before, most recent first.
    lastw: Vec<WinlinkId>,
}

/// What a change did, for the caller to act on in this order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkEvent {
    /// A winlink was added: take a window reference and fire
    /// window-linked.
    Linked(Winlink),
    /// A winlink went and its window is no longer in the session: fire
    /// window-unlinked. A Removed for the same winlink follows.
    Unlinked(Winlink),
    /// A winlink went: drop its window reference.
    Removed(Winlink),
    /// A session's current window changed.
    CurrentChanged(CurrentChange),
}

/// A change of a session's current window (`session_set_current`). The
/// caller updates focus and activity and fires session-window-changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentChange {
    pub session: SessionId,
    pub old: Option<Winlink>,
    pub new: Winlink,
    /// Sessions that had alert flags cleared on the new window, whose
    /// status lines need redrawing.
    pub cleared: Vec<SessionId>,
}

/// The answer of [`Links::last_target`].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LastTarget {
    /// The stack is empty.
    None,
    /// The top of the stack is the current window.
    Current,
    /// The window to select.
    Window(WinlinkId),
}

/// All links between sessions and windows, and the session groups.
#[derive(Clone, Debug, Default)]
pub struct Links {
    next_id: u32,
    winlinks: BTreeMap<WinlinkId, Winlink>,
    sessions: BTreeMap<SessionId, SessionLinks>,
    /// The winlinks of each window, in the order they were linked.
    windows: BTreeMap<WindowId, Vec<WinlinkId>>,
    /// Groups by name, their sessions in the order they joined.
    groups: BTreeMap<String, Vec<SessionId>>,
}

impl Links {
    pub fn new() -> Self {
        Self::default()
    }

    // Queries.

    /// The winlink with this ID, if it still exists.
    pub fn get(&self, id: WinlinkId) -> Option<Winlink> {
        self.winlinks.get(&id).copied()
    }

    /// A session's winlinks in index order.
    pub fn windows(&self, s: SessionId) -> Vec<Winlink> {
        self.sessions
            .get(&s)
            .map(|sl| sl.windows.values().map(|id| self.winlinks[id]).collect())
            .unwrap_or_default()
    }

    /// The number of winlinks in a session (`winlink_count`).
    pub fn count(&self, s: SessionId) -> u32 {
        self.sessions.get(&s).map_or(0, |sl| sl.windows.len() as u32)
    }

    /// A session's current winlink.
    pub fn current(&self, s: SessionId) -> Option<Winlink> {
        let id = self.sessions.get(&s)?.curw?;
        self.get(id)
    }

    /// A session's last window stack, most recent first.
    pub fn last_stack(&self, s: SessionId) -> Vec<Winlink> {
        self.sessions
            .get(&s)
            .map(|sl| sl.lastw.iter().map(|id| self.winlinks[id]).collect())
            .unwrap_or_default()
    }

    /// The winlink at an index (`winlink_find_by_index`).
    pub fn find_by_index(&self, s: SessionId, idx: i32) -> Option<Winlink> {
        debug_assert!(idx >= 0, "bad index {idx}");
        let id = self.sessions.get(&s)?.windows.get(&idx)?;
        self.get(*id)
    }

    /// The first winlink of a window in a session, by index
    /// (`winlink_find_by_window`).
    pub fn find_by_window(&self, s: SessionId, w: WindowId) -> Option<Winlink> {
        self.sessions
            .get(&s)?
            .windows
            .values()
            .map(|id| self.winlinks[id])
            .find(|wl| wl.window == w)
    }

    /// Whether a session has a window (`session_has`).
    pub fn has(&self, s: SessionId, w: WindowId) -> bool {
        self.window_links(w).iter().any(|wl| wl.session == s)
    }

    /// The winlink with the lowest index.
    pub fn first(&self, s: SessionId) -> Option<Winlink> {
        let id = self.sessions.get(&s)?.windows.values().next()?;
        self.get(*id)
    }

    /// The winlink with the highest index.
    pub fn last(&self, s: SessionId) -> Option<Winlink> {
        let id = self.sessions.get(&s)?.windows.values().next_back()?;
        self.get(*id)
    }

    /// The winlink after this one in its session (`winlink_next`).
    pub fn next(&self, id: WinlinkId) -> Option<Winlink> {
        let wl = self.get(id)?;
        let (_, next) = self.sessions[&wl.session]
            .windows
            .range(wl.idx.checked_add(1)?..)
            .next()?;
        self.get(*next)
    }

    /// The winlink before this one in its session (`winlink_previous`).
    pub fn previous(&self, id: WinlinkId) -> Option<Winlink> {
        let wl = self.get(id)?;
        let (_, prev) = self.sessions[&wl.session]
            .windows
            .range(..wl.idx)
            .next_back()?;
        self.get(*prev)
    }

    /// n winlinks on, wrapping to the first (`winlink_next_by_number`).
    pub fn next_by_number(&self, id: WinlinkId, n: u32) -> Option<Winlink> {
        let mut wl = self.get(id)?;
        for _ in 0..n {
            wl = match self.next(wl.id) {
                Some(next) => next,
                None => self.first(wl.session)?,
            };
        }
        Some(wl)
    }

    /// n winlinks back, wrapping to the last
    /// (`winlink_previous_by_number`).
    pub fn previous_by_number(&self, id: WinlinkId, n: u32) -> Option<Winlink> {
        let mut wl = self.get(id)?;
        for _ in 0..n {
            wl = match self.previous(wl.id) {
                Some(prev) => prev,
                None => self.last(wl.session)?,
            };
        }
        Some(wl)
    }

    /// A window's winlinks in the order they were linked.
    pub fn window_links(&self, w: WindowId) -> Vec<Winlink> {
        self.windows
            .get(&w)
            .map(|ids| ids.iter().map(|id| self.winlinks[id]).collect())
            .unwrap_or_default()
    }

    /// Whether a window is linked more than once anywhere
    /// (#{`window_linked`}).
    pub fn is_linked_anywhere(&self, w: WindowId) -> bool {
        self.windows.get(&w).is_some_and(|ids| ids.len() > 1)
    }

    /// The number of sessions a window is in, counting a group once
    /// (#{`window_linked_sessions`}).
    pub fn linked_sessions(&self, w: WindowId) -> u32 {
        let mut n = 0;
        for sessions in self.groups.values() {
            if let Some(&first) = sessions.first()
                && self.find_by_window(first, w).is_some()
            {
                n += 1;
            }
        }
        for &s in self.sessions.keys() {
            if self.group_of(s).is_none() && self.find_by_window(s, w).is_some() {
                n += 1;
            }
        }
        n
    }

    /// The index `winlink_add` would use for `idx`: a negative index asks
    /// for the first free one from -idx - 1 on, wrapping at `INT_MAX`. None
    /// if that index is taken or nothing is free.
    pub fn free_index(&self, s: SessionId, idx: i32) -> Option<i32> {
        let used = |i: i32| {
            self.sessions
                .get(&s)
                .is_some_and(|sl| sl.windows.contains_key(&i))
        };
        if idx >= 0 {
            return (!used(idx)).then_some(idx);
        }
        let start = -(idx + 1);
        let mut i = start;
        loop {
            if !used(i) {
                return Some(i);
            }
            i = if i == i32::MAX { 0 } else { i + 1 };
            if i == start {
                return None;
            }
        }
    }

    // Changes to winlinks.

    /// Links a window into a session at an index (`winlink_add` and
    /// `winlink_set_window`). A negative index asks for a free one (see
    /// [`Links::free_index`]). None if the index is taken.
    pub fn add(&mut self, s: SessionId, idx: i32, w: WindowId) -> Option<Winlink> {
        let idx = self.free_index(s, idx)?;
        let id = WinlinkId(self.next_id);
        self.next_id = self.next_id.checked_add(1).expect("winlink IDs ran out");
        let wl = Winlink {
            id,
            idx,
            session: s,
            window: w,
            flags: winlink_flags::empty(),
        };
        self.winlinks.insert(id, wl);
        self.sessions.entry(s).or_default().windows.insert(idx, id);
        self.windows.entry(w).or_default().push(id);
        Some(wl)
    }

    /// Removes a winlink (`winlink_remove`). It also leaves the last window
    /// stack, and a session whose current window it was has none until
    /// the caller picks one (callers move away from it first).
    pub fn remove(&mut self, id: WinlinkId) -> Option<Winlink> {
        let wl = self.winlinks.remove(&id)?;
        let sl = self.sessions.get_mut(&wl.session).expect("session of a winlink");
        sl.windows.remove(&wl.idx);
        sl.lastw.retain(|&x| x != id);
        if sl.curw == Some(id) {
            sl.curw = None;
        }
        let ids = self.windows.get_mut(&wl.window).expect("window of a winlink");
        ids.retain(|&x| x != id);
        if ids.is_empty() {
            self.windows.remove(&wl.window);
        }
        Some(wl)
    }

    /// Sets flags on a winlink.
    pub fn add_flags(&mut self, id: WinlinkId, flags: winlink_flags) {
        if let Some(wl) = self.winlinks.get_mut(&id) {
            wl.flags |= flags;
        }
    }

    /// Clears flags on a winlink.
    pub fn clear_flags(&mut self, id: WinlinkId, flags: winlink_flags) {
        if let Some(wl) = self.winlinks.get_mut(&id) {
            wl.flags &= !flags;
        }
    }

    /// Clears the alert flags of every winlink of a window, the winlink
    /// part of `winlink_clear_flags`. Returns the sessions of the winlinks
    /// that had any, in link order, for a status redraw.
    pub fn clear_alerts(&mut self, w: WindowId) -> Vec<SessionId> {
        let mut sessions = Vec::new();
        for id in self.windows.get(&w).cloned().unwrap_or_default() {
            let wl = self.winlinks.get_mut(&id).expect("winlink of a window");
            if wl.flags.intersects(WINLINK_ALERTFLAGS) {
                wl.flags &= !WINLINK_ALERTFLAGS;
                sessions.push(wl.session);
            }
        }
        sessions
    }

    /// Swaps the windows of two winlinks, as swap-window does.
    pub fn swap_windows(&mut self, a: WinlinkId, b: WinlinkId) {
        let (Some(wa), Some(wb)) = (self.get(a), self.get(b)) else {
            return;
        };
        if wa.window == wb.window {
            return;
        }
        // C's order: both leave their windows, then a joins b's window
        // and b joins a's, each at the end.
        for (id, w) in [(a, wa.window), (b, wb.window)] {
            self.windows.get_mut(&w).expect("window").retain(|&x| x != id);
        }
        self.windows.entry(wb.window).or_default().push(a);
        self.windows.entry(wa.window).or_default().push(b);
        self.winlinks.get_mut(&a).expect("winlink").window = wb.window;
        self.winlinks.get_mut(&b).expect("winlink").window = wa.window;
    }

    /// Makes room at a winlink's index, or the one after it, by moving
    /// the winlinks from there up to the next free index up by one
    /// (`winlink_shuffle_up`). Returns the index that is now free.
    pub fn shuffle_up(&mut self, s: SessionId, id: WinlinkId, before: bool) -> Option<i32> {
        let wl = self.get(id)?;
        if wl.idx == i32::MAX {
            return None;
        }
        let idx = if before { wl.idx } else { wl.idx + 1 };
        let sl = self.sessions.get_mut(&s)?;
        let mut last = idx;
        while last < i32::MAX && sl.windows.contains_key(&last) {
            last += 1;
        }
        if last == i32::MAX {
            return None;
        }
        while last > idx {
            let moved = sl.windows.remove(&(last - 1)).expect("winlink to move");
            sl.windows.insert(last, moved);
            self.winlinks.get_mut(&moved).expect("winlink").idx = last;
            last -= 1;
        }
        Some(idx)
    }

    /// Renumbers a session's windows from `base` on, keeping their order
    /// (`session_renumber_windows`). Nothing changes if they do not all
    /// fit. Winlinks keep their IDs, flags and places on the stack.
    pub fn renumber(&mut self, s: SessionId, base: i32) {
        let Some(sl) = self.sessions.get_mut(&s) else {
            return;
        };
        let n = sl.windows.len() as u64;
        if base < 0 || n > i32::MAX as u64 - base as u64 + 1 {
            return;
        }
        let old = std::mem::take(&mut sl.windows);
        for (i, id) in old.into_values().enumerate() {
            let idx = base + i as i32;
            sl.windows.insert(idx, id);
            self.winlinks.get_mut(&id).expect("winlink").idx = idx;
        }
    }

    // The current window and the stack.

    /// Puts a winlink on top of its session's stack (`winlink_stack_push`).
    fn stack_push(&mut self, s: SessionId, id: WinlinkId) {
        self.stack_remove(s, id);
        let sl = self.sessions.get_mut(&s).expect("session");
        sl.lastw.insert(0, id);
        self.winlinks.get_mut(&id).expect("winlink").flags |= winlink_flags::WINLINK_VISITED;
    }

    /// Takes a winlink off its session's stack (`winlink_stack_remove`).
    fn stack_remove(&mut self, s: SessionId, id: WinlinkId) {
        let Some(wl) = self.winlinks.get_mut(&id) else {
            return;
        };
        if wl.flags.intersects(winlink_flags::WINLINK_VISITED) {
            wl.flags &= !winlink_flags::WINLINK_VISITED;
            if let Some(sl) = self.sessions.get_mut(&s) {
                sl.lastw.retain(|&x| x != id);
            }
        }
    }

    /// Makes a winlink its session's current one (`session_set_current`).
    /// The old one goes on the stack and the new one's window has its
    /// alerts cleared. None if it is missing or already current.
    pub fn set_current(&mut self, s: SessionId, id: WinlinkId) -> Option<CurrentChange> {
        let new = self.get(id)?;
        debug_assert_eq!(new.session, s);
        let sl = self.sessions.get(&s)?;
        if sl.curw == Some(id) {
            return None;
        }
        let old = sl.curw.and_then(|old| self.get(old));
        self.stack_remove(s, id);
        if let Some(old) = old {
            self.stack_push(s, old.id);
        }
        self.sessions.get_mut(&s).expect("session").curw = Some(id);
        let cleared = self.clear_alerts(new.window);
        Some(CurrentChange {
            session: s,
            old,
            new: self.get(id).expect("winlink"),
            cleared,
        })
    }

    /// Sets the current winlink without the stack or alerts, for a new
    /// session or one whose current window went (spawn).
    pub fn set_current_only(&mut self, s: SessionId, id: Option<WinlinkId>) {
        if let Some(sl) = self.sessions.get_mut(&s) {
            sl.curw = id;
        }
    }

    /// The winlink `session_next` would move to: the next one, or with
    /// `alert` the next with an alert, wrapping round.
    pub fn next_target(&self, s: SessionId, alert: bool) -> Option<WinlinkId> {
        let cur = self.sessions.get(&s)?.curw?;
        let find = |mut wl: Option<Winlink>| {
            while let Some(w) = wl {
                if !alert || w.flags.intersects(WINLINK_ALERTFLAGS) {
                    return Some(w);
                }
                wl = self.next(w.id);
            }
            None
        };
        find(self.next(cur))
            .or_else(|| find(self.first(s)))
            .map(|wl| wl.id)
    }

    /// The winlink `session_previous` would move to.
    pub fn previous_target(&self, s: SessionId, alert: bool) -> Option<WinlinkId> {
        let cur = self.sessions.get(&s)?.curw?;
        let find = |mut wl: Option<Winlink>| {
            while let Some(w) = wl {
                if !alert || w.flags.intersects(WINLINK_ALERTFLAGS) {
                    return Some(w);
                }
                wl = self.previous(w.id);
            }
            None
        };
        find(self.previous(cur))
            .or_else(|| find(self.last(s)))
            .map(|wl| wl.id)
    }

    /// What `session_last` would do.
    pub fn last_target(&self, s: SessionId) -> LastTarget {
        let Some(sl) = self.sessions.get(&s) else {
            return LastTarget::None;
        };
        match sl.lastw.first() {
            None => LastTarget::None,
            Some(&id) if Some(id) == sl.curw => LastTarget::Current,
            Some(&id) => LastTarget::Window(id),
        }
    }

    /// Moves away from a session's current window as `session_detach` and
    /// `session_group_synchronize1` do: to the last window, else the
    /// previous, else the next.
    fn move_away(&mut self, s: SessionId, events: &mut Vec<LinkEvent>) {
        let mut select = |links: &mut Self, id: Option<WinlinkId>| -> bool {
            match id.and_then(|id| links.set_current(s, id)) {
                Some(change) => {
                    events.push(LinkEvent::CurrentChanged(change));
                    true
                }
                // session_set_current returns 1 for the current winlink,
                // which counts as failing here as it does in C.
                None => false,
            }
        };
        let last = match self.last_target(s) {
            LastTarget::Window(id) => Some(id),
            LastTarget::None | LastTarget::Current => None,
        };
        if select(self, last) {
            return;
        }
        let prev = self.previous_target(s, false);
        if select(self, prev) {
            return;
        }
        let next = self.next_target(s, false);
        select(self, next);
    }

    /// The first part of `session_detach`: moves the session away from the
    /// winlink if it is current, then clears its alerts. The caller fires
    /// window-unlinked while it is still linked and then calls
    /// [`Links::remove`]. Returns false (and does nothing) when it is the
    /// session's only winlink, which the caller must destroy with the
    /// session instead.
    pub fn detach_prepare(&mut self, id: WinlinkId, events: &mut Vec<LinkEvent>) -> bool {
        let Some(wl) = self.get(id) else {
            return false;
        };
        if self.count(wl.session) == 1 {
            return false;
        }
        if self.sessions[&wl.session].curw == Some(id) {
            self.move_away(wl.session, events);
        }
        self.clear_flags(id, WINLINK_ALERTFLAGS);
        true
    }

    /// Forgets a session that has no winlinks left.
    pub fn remove_session(&mut self, s: SessionId) {
        debug_assert!(self.count(s) == 0, "session still has winlinks");
        debug_assert!(self.group_of(s).is_none(), "session still in a group");
        self.sessions.remove(&s);
    }

    // Groups.

    /// The name of the group a session is in.
    pub fn group_of(&self, s: SessionId) -> Option<&str> {
        self.groups
            .iter()
            .find(|(_, sessions)| sessions.contains(&s))
            .map(|(name, _)| name.as_str())
    }

    /// A group's sessions in the order they joined. Empty if there is no
    /// such group.
    pub fn group_sessions(&self, name: &str) -> Vec<SessionId> {
        self.groups.get(name).cloned().unwrap_or_default()
    }

    /// Whether a group exists (`session_group_find`).
    pub fn group_exists(&self, name: &str) -> bool {
        self.groups.contains_key(name)
    }

    /// The names of all groups, in name order.
    pub fn group_names(&self) -> Vec<String> {
        self.groups.keys().cloned().collect()
    }

    /// Adds a session to a group, creating the group if needed
    /// (`session_group_new` and `session_group_add`). Returns false if the
    /// session was already in a group, which leaves it there.
    pub fn group_add(&mut self, name: &str, s: SessionId) -> bool {
        if self.group_of(s).is_some() {
            return false;
        }
        self.groups.entry(name.to_string()).or_default().push(s);
        true
    }

    /// Takes a session out of its group and drops the group if it is
    /// empty (`session_group_remove`). Returns the group's name.
    pub fn group_remove(&mut self, s: SessionId) -> Option<String> {
        let name = self.group_of(s)?.to_string();
        let sessions = self.groups.get_mut(&name).expect("group");
        sessions.retain(|&x| x != s);
        if sessions.is_empty() {
            self.groups.remove(&name);
        }
        Some(name)
    }

    /// Makes `s` a copy of `target` (`session_group_synchronize1`): the
    /// same windows at the same indexes with the target's alerts, its
    /// current window and stack kept by index where they still exist.
    /// All its winlinks are replaced by new ones.
    pub fn sync(&mut self, target: SessionId, s: SessionId, events: &mut Vec<LinkEvent>) {
        // Nothing to do if the target is empty (it is being destroyed).
        if self.count(target) == 0 || target == s {
            return;
        }

        // If the current window has vanished, move away from it first.
        if let Some(cur) = self.current(s)
            && self.find_by_index(target, cur.idx).is_none()
        {
            self.move_away(s, events);
        }

        let old_cur = self.current(s);
        let old_last = self.last_stack(s);
        let old = self.windows(s);
        for wl in &old {
            self.winlinks.remove(&wl.id);
            let ids = self.windows.get_mut(&wl.window).expect("window");
            ids.retain(|&x| x != wl.id);
        }
        let sl = self.sessions.entry(s).or_default();
        sl.windows.clear();
        sl.lastw.clear();
        sl.curw = None;

        // Link all the windows from the target.
        for twl in self.windows(target) {
            let wl = self.add(s, twl.idx, twl.window).expect("free index");
            self.add_flags(wl.id, twl.flags & WINLINK_ALERTFLAGS);
            events.push(LinkEvent::Linked(self.get(wl.id).expect("winlink")));
        }

        // Fix up the current window.
        let cur = match old_cur {
            Some(c) => self.find_by_index(s, c.idx),
            None => self
                .current(target)
                .and_then(|c| self.find_by_index(s, c.idx)),
        }
        .or_else(|| self.first(s));
        self.sessions.get_mut(&s).expect("session").curw = cur.map(|wl| wl.id);

        // Fix up the last window stack.
        for wl in old_last {
            if let Some(new) = self.find_by_index(s, wl.idx) {
                self.sessions.get_mut(&s).expect("session").lastw.push(new.id);
                self.add_flags(new.id, winlink_flags::WINLINK_VISITED);
            }
        }

        // Then the old winlinks go.
        for wl in old {
            if self.find_by_window(s, wl.window).is_none() {
                events.push(LinkEvent::Unlinked(wl));
            }
            events.push(LinkEvent::Removed(wl));
        }
        self.windows.retain(|_, ids| !ids.is_empty());
    }

    /// Synchronizes a session to its group, from the first other session
    /// (`session_group_synchronize_to`).
    pub fn sync_to(&mut self, s: SessionId, events: &mut Vec<LinkEvent>) {
        let Some(name) = self.group_of(s) else {
            return;
        };
        let target = self.groups[name].iter().copied().find(|&t| t != s);
        if let Some(target) = target {
            self.sync(target, s, events);
        }
    }

    /// Synchronizes the rest of a session's group to it
    /// (`session_group_synchronize_from`).
    pub fn sync_from(&mut self, target: SessionId, events: &mut Vec<LinkEvent>) {
        let Some(name) = self.group_of(target) else {
            return;
        };
        for s in self.groups[name].clone() {
            if s != target {
                self.sync(target, s, events);
            }
        }
    }

    /// Checks the store's invariants, for tests.
    #[cfg(test)]
    fn check(&self) {
        let visited = winlink_flags::WINLINK_VISITED;
        for (id, wl) in &self.winlinks {
            assert_eq!(*id, wl.id);
            let sl = &self.sessions[&wl.session];
            assert_eq!(sl.windows.get(&wl.idx), Some(id), "winlink at its index");
            let in_window = self.windows[&wl.window].iter().filter(|&&x| x == *id);
            assert_eq!(in_window.count(), 1, "winlink once in its window");
            assert!(wl.id.0 < self.next_id);
            assert_eq!(
                wl.flags.intersects(visited),
                sl.lastw.contains(id),
                "visited flag matches the stack"
            );
        }
        for (s, sl) in &self.sessions {
            for (idx, id) in &sl.windows {
                let wl = self.winlinks.get(id).expect("indexed winlink exists");
                assert_eq!((wl.session, wl.idx), (*s, *idx));
            }
            assert_eq!(sl.curw.is_none(), sl.windows.is_empty(), "current window");
            if let Some(cur) = sl.curw {
                assert_eq!(self.winlinks[&cur].session, *s);
            }
            let mut seen = Vec::new();
            for id in &sl.lastw {
                assert_eq!(self.winlinks[id].session, *s, "stack holds own winlinks");
                assert!(!seen.contains(id), "no duplicates on the stack");
                seen.push(*id);
            }
        }
        for (w, ids) in &self.windows {
            assert!(!ids.is_empty());
            for id in ids {
                assert_eq!(self.winlinks[id].window, *w);
            }
        }
        for sessions in self.groups.values() {
            assert!(!sessions.is_empty());
            let first: Vec<_> = self
                .windows(sessions[0])
                .iter()
                .map(|wl| (wl.idx, wl.window))
                .collect();
            for &s in sessions {
                let these: Vec<_> = self
                    .windows(s)
                    .iter()
                    .map(|wl| (wl.idx, wl.window))
                    .collect();
                assert_eq!(these, first, "grouped sessions have the same windows");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: SessionId = SessionId(1);
    const B: SessionId = SessionId(2);
    const C: SessionId = SessionId(3);

    fn w(n: u32) -> WindowId {
        WindowId(n)
    }

    /// Session A with windows 1, 2 and 3 at indexes 0, 1 and 2, the first
    /// current.
    fn three() -> (Links, Vec<WinlinkId>) {
        let mut l = Links::new();
        let ids: Vec<_> = (0..3).map(|i| l.add(A, i, w(i as u32 + 1)).unwrap().id).collect();
        l.set_current(A, ids[0]);
        l.check();
        (l, ids)
    }

    fn indexes(l: &Links, s: SessionId) -> Vec<i32> {
        l.windows(s).iter().map(|wl| wl.idx).collect()
    }

    #[test]
    fn add_finds_free_indexes() {
        let (mut l, _) = three();
        assert!(l.add(A, 1, w(9)).is_none(), "index in use");
        assert_eq!(l.add(A, -1, w(9)).unwrap().idx, 3);
        assert_eq!(l.add(A, -2, w(9)).unwrap().idx, 4);
        assert_eq!(l.add(A, -11, w(9)).unwrap().idx, 10);
        assert_eq!(l.free_index(A, -(i32::MAX) - 1), Some(i32::MAX));
        l.add(A, i32::MAX, w(9)).unwrap();
        // From INT_MAX the search wraps to 0, then on to the first free.
        assert_eq!(l.free_index(A, -(i32::MAX) - 1), Some(5));
        l.check();
    }

    #[test]
    fn ids_are_not_reused() {
        let (mut l, ids) = three();
        l.remove(ids[2]);
        let new = l.add(A, 2, w(3)).unwrap();
        assert!(!ids.contains(&new.id));
        assert!(l.get(ids[2]).is_none());
    }

    #[test]
    fn set_current_keeps_the_stack() {
        let (mut l, ids) = three();
        let change = l.set_current(A, ids[2]).unwrap();
        assert_eq!(change.old.map(|o| o.id), Some(ids[0]));
        assert!(l.set_current(A, ids[2]).is_none(), "already current");
        l.set_current(A, ids[1]);
        let stack: Vec<_> = l.last_stack(A).iter().map(|wl| wl.id).collect();
        assert_eq!(stack, [ids[2], ids[0]]);
        assert_eq!(l.last_target(A), LastTarget::Window(ids[2]));
        l.check();
    }

    #[test]
    fn set_current_clears_alerts_of_the_window() {
        let (mut l, ids) = three();
        l.add(B, 0, w(2)).unwrap();
        l.set_current(B, l.first(B).unwrap().id);
        for wl in l.window_links(w(2)) {
            l.add_flags(wl.id, winlink_flags::WINLINK_BELL);
        }
        let change = l.set_current(A, ids[1]).unwrap();
        assert_eq!(change.cleared, [A, B]);
        assert!(l.window_links(w(2)).iter().all(|wl| !wl.flags.intersects(WINLINK_ALERTFLAGS)));
    }

    #[test]
    fn next_and_previous_wrap_and_find_alerts() {
        let (mut l, ids) = three();
        assert_eq!(l.next_target(A, false), Some(ids[1]));
        assert_eq!(l.previous_target(A, false), Some(ids[2]));
        assert_eq!(l.next_target(A, true), None);
        l.add_flags(ids[2], winlink_flags::WINLINK_ACTIVITY);
        assert_eq!(l.next_target(A, true), Some(ids[2]));
        assert_eq!(l.previous_target(A, true), Some(ids[2]));
        assert_eq!(l.next_by_number(ids[2], 2).unwrap().id, ids[1]);
        assert_eq!(l.previous_by_number(ids[0], 4).unwrap().id, ids[2]);
    }

    #[test]
    fn detach_moves_to_the_last_window() {
        let (mut l, ids) = three();
        l.set_current(A, ids[2]);
        l.set_current(A, ids[1]);
        let mut events = Vec::new();
        assert!(l.detach_prepare(ids[1], &mut events));
        assert!(matches!(&events[..], [LinkEvent::CurrentChanged(c)] if c.new.id == ids[2]));
        l.remove(ids[1]);
        assert_eq!(l.current(A).unwrap().id, ids[2]);
        l.check();
    }

    #[test]
    fn detach_leaves_the_last_winlink() {
        let mut l = Links::new();
        let id = l.add(A, 0, w(1)).unwrap().id;
        l.set_current(A, id);
        let mut events = Vec::new();
        assert!(!l.detach_prepare(id, &mut events));
        assert!(events.is_empty());
        assert_eq!(l.count(A), 1);
    }

    #[test]
    fn shuffle_up_and_renumber() {
        let mut l = Links::new();
        for i in [0, 1, 2, 5] {
            l.add(A, i, w(i as u32 + 1)).unwrap();
        }
        l.set_current(A, l.first(A).unwrap().id);
        let one = l.find_by_index(A, 1).unwrap().id;
        assert_eq!(l.shuffle_up(A, one, false), Some(2));
        assert_eq!(indexes(&l, A), [0, 1, 3, 5]);
        assert_eq!(l.shuffle_up(A, one, true), Some(1));
        assert_eq!(indexes(&l, A), [0, 2, 3, 5]);
        l.renumber(A, 1);
        assert_eq!(indexes(&l, A), [1, 2, 3, 4]);
        assert_eq!(l.get(one).unwrap().idx, 2, "winlinks keep their IDs");
        l.renumber(A, i32::MAX - 2);
        assert_eq!(indexes(&l, A), [1, 2, 3, 4], "does not fit");
        l.check();
    }

    #[test]
    fn linked_counts() {
        let (mut l, _) = three();
        let b = l.add(B, 0, w(1)).unwrap();
        l.set_current(B, b.id);
        l.group_add("g", A);
        l.group_add("g", C);
        let mut events = Vec::new();
        l.sync_to(C, &mut events);
        l.check();
        assert!(l.is_linked_anywhere(w(2)));
        assert_eq!(l.linked_sessions(w(2)), 1);
        assert_eq!(l.linked_sessions(w(1)), 2);
        assert!(l.has(B, w(1)));
        assert!(!l.has(B, w(2)));
    }

    #[test]
    fn sync_links_and_unlinks() {
        let (mut l, ids) = three();
        l.group_add("g", A);
        l.group_add("g", B);
        let mut events = Vec::new();
        l.sync_to(B, &mut events);
        let linked = events
            .iter()
            .filter(|e| matches!(e, LinkEvent::Linked(_)))
            .count();
        assert_eq!(linked, 3);
        assert_eq!(l.current(B).unwrap().idx, 0);
        l.check();

        // A loses window 3: B follows, and it is unlinked from B.
        l.remove(ids[2]);
        events.clear();
        l.sync_from(A, &mut events);
        let unlinked: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                LinkEvent::Unlinked(wl) => Some(wl.window),
                _ => None,
            })
            .collect();
        assert_eq!(unlinked, [w(3)]);
        assert_eq!(indexes(&l, B), [0, 1]);
        l.check();
    }

    /// A new group by name holds only the new session, which is not
    /// synchronized with itself.
    #[test]
    fn sync_alone_does_nothing() {
        let (mut l, _) = three();
        l.group_add("g", A);
        let mut events = Vec::new();
        l.sync_to(A, &mut events);
        assert!(events.is_empty());
        l.check();
    }

    #[test]
    fn swap_windows() {
        let (mut l, ids) = three();
        l.swap_windows(ids[0], ids[2]);
        assert_eq!(l.get(ids[0]).unwrap().window, w(3));
        assert_eq!(l.window_links(w(1))[0].id, ids[2]);
        l.check();
    }

    mod sequences {
        use proptest::prelude::*;

        use super::*;

        /// One step of a random sequence. Numbers pick sessions, windows,
        /// indexes and winlinks modulo what exists.
        #[derive(Clone, Debug)]
        enum Op {
            Add(u8, i8, u8),
            Detach(u8),
            Select(u8),
            Next(u8, bool),
            Previous(u8, bool),
            Last(u8),
            Shuffle(u8, bool),
            Renumber(u8, u8),
            Swap(u8, u8),
            Alert(u8),
            Group(u8, u8),
            Ungroup(u8),
            Destroy(u8),
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                3 => (0..4u8, -4..8i8, 0..6u8).prop_map(|(s, i, w)| Op::Add(s, i, w)),
                2 => any::<u8>().prop_map(Op::Detach),
                2 => any::<u8>().prop_map(Op::Select),
                1 => (0..4u8, any::<bool>()).prop_map(|(s, a)| Op::Next(s, a)),
                1 => (0..4u8, any::<bool>()).prop_map(|(s, a)| Op::Previous(s, a)),
                1 => (0..4u8).prop_map(Op::Last),
                1 => (any::<u8>(), any::<bool>()).prop_map(|(w, b)| Op::Shuffle(w, b)),
                1 => (0..4u8, 0..3u8).prop_map(|(s, b)| Op::Renumber(s, b)),
                1 => (any::<u8>(), any::<u8>()).prop_map(|(a, b)| Op::Swap(a, b)),
                1 => any::<u8>().prop_map(Op::Alert),
                1 => (0..4u8, 0..2u8).prop_map(|(s, g)| Op::Group(s, g)),
                1 => (0..4u8).prop_map(Op::Ungroup),
                1 => (0..4u8).prop_map(Op::Destroy),
            ]
        }

        fn pick(l: &Links, n: u8) -> Option<Winlink> {
            let all: Vec<_> = l.winlinks.values().copied().collect();
            (!all.is_empty()).then(|| all[n as usize % all.len()])
        }

        /// Applies an op as the adapter would, keeping a session's current
        /// window set as `session_attach` and spawn do.
        fn apply(l: &mut Links, op: Op) {
            let mut ev = Vec::new();
            match op {
                Op::Add(s, i, w) => {
                    let s = SessionId(s as u32);
                    if let Some(wl) = l.add(s, i as i32, WindowId(w as u32)) {
                        if l.current(s).is_none() {
                            l.set_current(s, wl.id);
                        }
                        l.sync_from(s, &mut ev);
                    }
                }
                Op::Detach(n) => {
                    if let Some(wl) = pick(l, n)
                        && l.detach_prepare(wl.id, &mut ev)
                    {
                        l.remove(wl.id);
                        l.sync_from(wl.session, &mut ev);
                    }
                }
                Op::Select(n) => {
                    if let Some(wl) = pick(l, n) {
                        l.set_current(wl.session, wl.id);
                    }
                }
                Op::Next(s, a) => {
                    let s = SessionId(s as u32);
                    if let Some(id) = l.next_target(s, a) {
                        l.set_current(s, id);
                    }
                }
                Op::Previous(s, a) => {
                    let s = SessionId(s as u32);
                    if let Some(id) = l.previous_target(s, a) {
                        l.set_current(s, id);
                    }
                }
                Op::Last(s) => {
                    let s = SessionId(s as u32);
                    if let LastTarget::Window(id) = l.last_target(s) {
                        l.set_current(s, id);
                    }
                }
                Op::Shuffle(n, before) => {
                    if let Some(wl) = pick(l, n) {
                        l.shuffle_up(wl.session, wl.id, before);
                        l.sync_from(wl.session, &mut ev);
                    }
                }
                Op::Renumber(s, base) => {
                    let s = SessionId(s as u32);
                    l.renumber(s, base as i32);
                    l.sync_from(s, &mut ev);
                }
                Op::Swap(a, b) => {
                    if let (Some(a), Some(b)) = (pick(l, a), pick(l, b)) {
                        // swap-window refuses within a group across
                        // sessions.
                        let ga = l.group_of(a.session).map(str::to_string);
                        if a.session != b.session
                            && ga.is_some()
                            && ga.as_deref() == l.group_of(b.session)
                        {
                            return;
                        }
                        l.swap_windows(a.id, b.id);
                        l.sync_from(a.session, &mut ev);
                        if a.session != b.session {
                            l.sync_from(b.session, &mut ev);
                        }
                    }
                }
                Op::Alert(n) => {
                    if let Some(wl) = pick(l, n) {
                        l.add_flags(wl.id, winlink_flags::WINLINK_BELL);
                    }
                }
                Op::Group(s, g) => {
                    // new-session -t: a session joins a group and copies
                    // it. Only an empty session can join, as a new one is.
                    let s = SessionId(s as u32 + 10);
                    if l.count(s) == 0 && l.group_of(s).is_none() {
                        let name = format!("g{g}");
                        if l.group_sessions(&name).is_empty() {
                            // A group from an existing session.
                            let src = SessionId(g as u32);
                            if l.count(src) == 0 || l.group_of(src).is_some() {
                                return;
                            }
                            l.group_add(&name, src);
                        }
                        l.group_add(&name, s);
                        l.sync_to(s, &mut ev);
                    }
                }
                Op::Ungroup(s) => {
                    l.group_remove(SessionId(s as u32));
                }
                Op::Destroy(s) => {
                    let s = SessionId(s as u32);
                    l.group_remove(s);
                    for wl in l.windows(s) {
                        l.remove(wl.id);
                    }
                    l.remove_session(s);
                }
            }
            // A session with windows but no current one only exists
            // while being built.
            for s in l.sessions.keys().copied().collect::<Vec<_>>() {
                if l.current(s).is_none()
                    && let Some(first) = l.first(s)
                {
                    l.set_current(s, first.id);
                }
            }
        }

        proptest! {
            #[test]
            fn invariants_hold(ops in proptest::collection::vec(op(), 1..60)) {
                let mut l = Links::new();
                let mut seen = std::collections::BTreeSet::new();
                for op in ops {
                    apply(&mut l, op);
                    l.check();
                    for id in l.winlinks.keys() {
                        seen.insert(*id);
                    }
                    prop_assert!(l.next_id as usize >= seen.len());
                }
            }
        }
    }
}
