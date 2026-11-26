// Copyright (c) 2013 Nicholas Marriott <nicholas.marriott@gmail.com>
// Copyright (c) 2013 Thiago de Arruda <tpadilha84@gmail.com>
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

//! Wait channels, the channel part of cmd-wait-for.c from tmux 3.8 in safe
//! Rust.
//!
//! A channel is named by its first use. Commands wait on it until it is
//! signalled (wait-for and wait-for -S), or queue for its lock
//! (wait-for -L and -U). A signal with nobody waiting is kept for the next
//! waiter. Channels go away once nothing is left in them, with C's two
//! rules: after a signal or unlock, a channel is dropped when it is woken
//! and has no waiters and no lock, and after a waiter goes (-w, a lost
//! client) when it has nothing at all.
//!
//! The store does not run anything. Every method returns the waiters that
//! run on, and the caller continues them. A waiter is whatever handle the
//! caller uses (a queue item and its client); the store only keeps and
//! compares them. Names are bytes.
#![forbid(unsafe_code)]

use std::collections::BTreeMap;

/// One channel.
#[derive(Debug)]
struct Channel<W> {
    locked: bool,
    woken: bool,
    waiters: Vec<W>,
    lockers: Vec<W>,
}

impl<W> Default for Channel<W> {
    fn default() -> Self {
        Self {
            locked: false,
            woken: false,
            waiters: Vec::new(),
            lockers: Vec::new(),
        }
    }
}

/// Whether a command runs on now or waits to be continued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitOutcome {
    Done,
    Wait,
}

/// unlock of a channel that is not locked.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("channel {0} not locked")]
pub struct NotLocked(pub String);

/// All wait channels, by name.
#[derive(Debug)]
pub struct WaitChannels<W> {
    channels: BTreeMap<Vec<u8>, Channel<W>>,
}

impl<W> Default for WaitChannels<W> {
    fn default() -> Self {
        Self {
            channels: BTreeMap::new(),
        }
    }
}

impl<W: Clone> WaitChannels<W> {
    /// No channels.
    pub fn new() -> Self {
        Self::default()
    }

    fn channel(&mut self, name: &[u8]) -> &mut Channel<W> {
        self.channels.entry(name.to_vec()).or_default()
    }

    /// Drops the channel if it is woken, unlocked and nobody waits
    /// (`cmd_wait_for_remove`).
    fn remove(&mut self, name: &[u8]) {
        if let Some(c) = self.channels.get(name)
            && !c.locked
            && c.waiters.is_empty()
            && c.woken
        {
            self.channels.remove(name);
        }
    }

    /// Drops the channel if nothing is left in it
    /// (`cmd_wait_for_remove_empty`).
    fn remove_empty(&mut self, name: &[u8]) {
        if let Some(c) = self.channels.get(name)
            && !c.locked
            && !c.woken
            && c.waiters.is_empty()
            && c.lockers.is_empty()
        {
            self.channels.remove(name);
        }
    }

    /// wait-for -S: the waiters that run on. With nobody waiting the
    /// signal is kept.
    pub fn signal(&mut self, name: &[u8]) -> Vec<W> {
        let c = self.channel(name);
        if c.waiters.is_empty() && !c.woken {
            c.woken = true;
            return Vec::new();
        }
        let woken = std::mem::take(&mut c.waiters);
        self.remove(name);
        woken
    }

    /// wait-for: `waiter` runs on at once if the channel was signalled,
    /// otherwise it waits.
    pub fn wait(&mut self, name: &[u8], waiter: W) -> WaitOutcome {
        let c = self.channel(name);
        if c.woken {
            self.remove(name);
            return WaitOutcome::Done;
        }
        c.waiters.push(waiter);
        WaitOutcome::Wait
    }

    /// wait-for -L: `waiter` takes the lock, or queues for it.
    pub fn lock(&mut self, name: &[u8], waiter: W) -> WaitOutcome {
        let c = self.channel(name);
        if c.locked {
            c.lockers.push(waiter);
            return WaitOutcome::Wait;
        }
        c.locked = true;
        WaitOutcome::Done
    }

    /// wait-for -U: hands the lock to the next locker, which runs on, or
    /// frees it.
    pub fn unlock(&mut self, name: &[u8]) -> Result<Option<W>, NotLocked> {
        let Some(c) = self.channels.get_mut(name).filter(|c| c.locked) else {
            return Err(NotLocked(String::from_utf8_lossy(name).into_owned()));
        };
        if !c.lockers.is_empty() {
            return Ok(Some(c.lockers.remove(0)));
        }
        c.locked = false;
        self.remove(name);
        Ok(None)
    }

    /// wait-for -w: the first waiter, then locker, that `is_it` picks runs
    /// on, without the lock.
    pub fn wake(&mut self, name: &[u8], is_it: impl Fn(&W) -> bool) -> Option<W> {
        let c = self.channels.get_mut(name)?;
        let woken = if let Some(i) = c.waiters.iter().position(&is_it) {
            c.waiters.remove(i)
        } else {
            let i = c.lockers.iter().position(&is_it)?;
            c.lockers.remove(i)
        };
        self.remove_empty(name);
        Some(woken)
    }

    /// wait-for -l: the waiters, then the lockers.
    pub fn list(&self, name: &[u8]) -> Vec<W> {
        self.channels
            .get(name)
            .map(|c| c.waiters.iter().chain(&c.lockers).cloned().collect())
            .unwrap_or_default()
    }

    /// A client went away: its waiters and lockers (`is_its`) run on and
    /// fail, and channels left with nothing go.
    pub fn client_lost(&mut self, is_its: impl Fn(&W) -> bool) -> Vec<W> {
        let mut woken = Vec::new();
        let names: Vec<Vec<u8>> = self.channels.keys().cloned().collect();
        for name in names {
            let c = self.channels.get_mut(&name).expect("collected above");
            for list in [&mut c.waiters, &mut c.lockers] {
                let (theirs, kept) = std::mem::take(list).into_iter().partition(&is_its);
                *list = kept;
                woken.extend::<Vec<W>>(theirs);
            }
            self.remove_empty(&name);
        }
        woken
    }

    /// Everything runs on and every lock is freed (server exit).
    pub fn flush(&mut self) -> Vec<W> {
        let mut woken = Vec::new();
        let names: Vec<Vec<u8>> = self.channels.keys().cloned().collect();
        for name in names {
            let c = self.channels.get_mut(&name).expect("collected above");
            woken.append(&mut c.waiters);
            c.woken = true;
            woken.append(&mut c.lockers);
            c.locked = false;
            self.remove(&name);
        }
        woken
    }

    /// Whether a channel called `name` exists, for tests.
    #[cfg(test)]
    fn exists(&self, name: &[u8]) -> bool {
        self.channels.contains_key(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Channels = WaitChannels<(u32, char)>;

    #[test]
    fn signal_kept_for_one_waiter() {
        let mut w = Channels::new();
        assert!(w.signal(b"c").is_empty());
        assert_eq!(w.wait(b"c", (1, 'a')), WaitOutcome::Done);
        assert!(!w.exists(b"c"));
        assert_eq!(w.wait(b"c", (1, 'b')), WaitOutcome::Wait);
        assert_eq!(w.wait(b"c", (2, 'c')), WaitOutcome::Wait);
        assert_eq!(w.list(b"c"), [(1, 'b'), (2, 'c')]);
        assert_eq!(w.signal(b"c"), [(1, 'b'), (2, 'c')]);
        // An empty channel that was not woken stays, as in C.
        assert!(w.exists(b"c"));
        assert!(w.list(b"c").is_empty());
    }

    #[test]
    fn lock_queue() {
        let mut w = Channels::new();
        assert_eq!(w.lock(b"l", (1, 'a')), WaitOutcome::Done);
        assert_eq!(w.lock(b"l", (2, 'b')), WaitOutcome::Wait);
        assert_eq!(w.list(b"l"), [(2, 'b')]);
        assert_eq!(w.unlock(b"l"), Ok(Some((2, 'b'))));
        assert_eq!(w.unlock(b"l"), Ok(None));
        assert_eq!(w.unlock(b"l"), Err(NotLocked("l".into())));
        assert_eq!(w.unlock(b"x").unwrap_err().to_string(), "channel x not locked");
    }

    #[test]
    fn wake_waiters_before_lockers() {
        let mut w = Channels::new();
        w.lock(b"c", (1, 'a'));
        w.lock(b"c", (2, 'b'));
        w.wait(b"c", (2, 'c'));
        assert_eq!(w.wake(b"c", |x| x.0 == 2), Some((2, 'c')));
        assert_eq!(w.wake(b"c", |x| x.0 == 2), Some((2, 'b')));
        assert_eq!(w.wake(b"c", |x| x.0 == 2), None);
        assert_eq!(w.wake(b"none", |_| true), None);
        // Still locked by the first.
        assert_eq!(w.unlock(b"c"), Ok(None));
    }

    #[test]
    fn lost_client_and_empty_channels() {
        let mut w = Channels::new();
        w.wait(b"a", (1, 'a'));
        w.wait(b"a", (2, 'b'));
        w.wait(b"b", (1, 'c'));
        let woken = w.client_lost(|x| x.0 == 1);
        assert_eq!(woken, [(1, 'a'), (1, 'c')]);
        assert!(w.exists(b"a"));
        assert!(!w.exists(b"b"));
    }

    #[test]
    fn flush_everything() {
        let mut w = Channels::new();
        w.wait(b"a", (1, 'a'));
        w.lock(b"b", (1, 'b'));
        w.lock(b"b", (2, 'c'));
        assert_eq!(w.flush(), [(1, 'a'), (2, 'c')]);
        assert!(!w.exists(b"a") && !w.exists(b"b"));
    }

    #[test]
    fn byte_names() {
        let mut w = Channels::new();
        w.signal(b"c\xff");
        assert_eq!(w.wait(b"c\xff", (1, 'a')), WaitOutcome::Done);
    }
}
