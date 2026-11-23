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
#![forbid(unsafe_code)]
//! The server message log: the commands and messages show-messages
//! prints.
//!
//! Messages are numbered from 0 in the order they are added. Adding a
//! message drops the oldest ones until no more than the message-limit
//! option allows are left, so a limit of 0 keeps nothing. The caller
//! passes in the time and the limit.

use std::collections::VecDeque;
use std::time::SystemTime;

/// One logged message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    text: String,
    number: u32,
    time: SystemTime,
}

impl Message {
    /// The message text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The message number, counting from 0 since the server started.
    pub fn number(&self) -> u32 {
        self.number
    }

    /// When the message was added.
    pub fn time(&self) -> SystemTime {
        self.time
    }
}

/// The message log, oldest message first.
#[derive(Debug, Default)]
pub struct MessageLog {
    messages: VecDeque<Message>,
    next: u32,
}

impl MessageLog {
    /// An empty log whose first message gets number 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a message and drops the messages that are more than `limit`
    /// behind it. Returns the number the message got.
    ///
    /// The numbers wrap like C's unsigned counter, and so does the limit
    /// check.
    pub fn add(&mut self, text: String, time: SystemTime, limit: u32) -> u32 {
        let number = self.next;
        self.next = self.next.wrapping_add(1);
        self.messages.push_back(Message { text, number, time });
        while let Some(oldest) = self.messages.front() {
            if oldest.number.wrapping_add(limit) >= self.next {
                break;
            }
            self.messages.pop_front();
        }
        number
    }

    /// The messages, newest first.
    pub fn iter(&self) -> impl Iterator<Item = &Message> {
        self.messages.iter().rev()
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn texts(log: &MessageLog) -> Vec<&str> {
        log.iter().map(Message::text).collect()
    }

    #[test]
    fn numbers_from_zero() {
        let mut log = MessageLog::new();
        assert_eq!(log.add("a".into(), at(1), 10), 0);
        assert_eq!(log.add("b".into(), at(2), 10), 1);
        let numbers: Vec<u32> = log.iter().map(Message::number).collect();
        assert_eq!(numbers, [1, 0]);
    }

    #[test]
    fn newest_first_with_time() {
        let mut log = MessageLog::new();
        log.add("a".into(), at(1), 10);
        log.add("b".into(), at(2), 10);
        assert_eq!(texts(&log), ["b", "a"]);
        assert_eq!(log.iter().next().map(Message::time), Some(at(2)));
    }

    #[test]
    fn keeps_limit_messages() {
        let mut log = MessageLog::new();
        for i in 0..5 {
            log.add(format!("m{i}"), at(i), 3);
        }
        assert_eq!(texts(&log), ["m4", "m3", "m2"]);
    }

    #[test]
    fn limit_zero_keeps_nothing() {
        let mut log = MessageLog::new();
        assert_eq!(log.add("a".into(), at(1), 0), 0);
        assert!(texts(&log).is_empty());
        // Numbering goes on.
        assert_eq!(log.add("b".into(), at(1), 0), 1);
    }

    #[test]
    fn lower_limit_drops_on_next_add() {
        let mut log = MessageLog::new();
        for i in 0..5 {
            log.add(format!("m{i}"), at(i), 100);
        }
        assert_eq!(texts(&log).len(), 5);
        log.add("m5".into(), at(5), 1);
        assert_eq!(texts(&log), ["m5"]);
    }

    #[test]
    fn numbers_wrap() {
        let mut log = MessageLog {
            messages: VecDeque::new(),
            next: u32::MAX,
        };
        assert_eq!(log.add("a".into(), at(1), 2), u32::MAX);
        assert_eq!(log.add("b".into(), at(1), 2), 0);
        assert_eq!(texts(&log), ["b", "a"]);
    }
}
