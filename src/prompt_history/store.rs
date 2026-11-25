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
//! Prompt history (prompt-history.c): one list of lines per prompt type,
//! oldest first.
//!
//! The store knows nothing outside itself. The history limit comes in as
//! a parameter, and reading and writing the history file is left to the
//! adapter, which hands the file's contents to [`PromptHistory::load`] and
//! writes what [`PromptHistory::to_file`] returns. Lines are bytes, as
//! they came from the prompt or the file.

#![forbid(unsafe_code)]

/// The kinds of prompt, each with its own history. Command is 0, so a
/// zeroed client holds a valid type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PromptType {
    /// A tmux command (command-prompt without -T).
    #[default]
    Command,
    /// Text to search for (copy mode and the tree modes).
    Search,
}

impl PromptType {
    /// All types, in the order the history file and show-prompt-history
    /// list them.
    pub const ALL: [PromptType; 2] = [PromptType::Command, PromptType::Search];

    /// The number of types.
    pub const COUNT: usize = Self::ALL.len();

    /// The type's name, as -T and the history file use it.
    pub fn name(self) -> &'static str {
        match self {
            PromptType::Command => "command",
            PromptType::Search => "search",
        }
    }

    /// The type called `name`, if there is one.
    pub fn from_name(name: &[u8]) -> Option<PromptType> {
        Self::ALL.into_iter().find(|t| t.name().as_bytes() == name)
    }

    /// The type's index, for per-type arrays.
    pub fn index(self) -> usize {
        self as usize
    }
}

/// Where a prompt is in each history list while the user moves through
/// it with up and down: 0 is the line being typed, 1 the newest entry.
pub type HistoryPositions = [usize; PromptType::COUNT];

/// The prompt history of all types.
#[derive(Debug, Default)]
pub struct PromptHistory {
    lists: [Vec<Vec<u8>>; PromptType::COUNT],
}

impl PromptHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// The entries of `ty`, oldest first.
    pub fn entries(&self, ty: PromptType) -> &[Vec<u8>] {
        &self.lists[ty.index()]
    }

    /// Adds `line` as the newest entry of `ty`, unless it is the same as
    /// the newest entry, and drops the oldest entries beyond `limit`.
    pub fn add(&mut self, ty: PromptType, line: &[u8], limit: usize) {
        let list = &mut self.lists[ty.index()];
        if list.last().map(Vec::as_slice) != Some(line) {
            list.push(line.to_vec());
        }
        if list.len() > limit {
            list.drain(..list.len() - limit);
        }
    }

    /// Deletes all entries of `ty`.
    pub fn clear(&mut self, ty: PromptType) {
        self.lists[ty.index()].clear();
    }

    /// The entry before the one at `positions` (up), moving there, or
    /// `None` at the oldest entry.
    pub fn up(&self, ty: PromptType, positions: &mut HistoryPositions) -> Option<&[u8]> {
        let list = &self.lists[ty.index()];
        let pos = &mut positions[ty.index()];
        if list.is_empty() || *pos >= list.len() {
            return None;
        }
        *pos += 1;
        Some(&list[list.len() - *pos])
    }

    /// The entry after the one at `positions` (down), moving there. Past
    /// the newest entry this is the empty line.
    pub fn down(&self, ty: PromptType, positions: &mut HistoryPositions) -> &[u8] {
        let list = &self.lists[ty.index()];
        let pos = &mut positions[ty.index()];
        if list.is_empty() || *pos == 0 {
            return b"";
        }
        *pos -= 1;
        if *pos == 0 {
            return b"";
        }
        &list[list.len() - *pos]
    }

    /// Adds the lines of a history file, each "type:line". A line with no
    /// known type is a command, as in history files from before types
    /// existed. An empty line is an empty command, as C reads it.
    pub fn load(&mut self, contents: &[u8], limit: usize) {
        if contents.is_empty() {
            return;
        }
        let contents = contents.strip_suffix(b"\n").unwrap_or(contents);
        for line in contents.split(|&b| b == b'\n') {
            let typed = line.iter().position(|&b| b == b':').and_then(|colon| {
                PromptType::from_name(&line[..colon]).map(|ty| (ty, &line[colon + 1..]))
            });
            match typed {
                Some((ty, rest)) => self.add(ty, rest, limit),
                None => self.add(PromptType::Command, line, limit),
            }
        }
    }

    /// The history file's contents: every entry as "type:line", by type,
    /// oldest first.
    pub fn to_file(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for ty in PromptType::ALL {
            for line in self.entries(ty) {
                out.extend_from_slice(ty.name().as_bytes());
                out.push(b':');
                out.extend_from_slice(line);
                out.push(b'\n');
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries(h: &PromptHistory, ty: PromptType) -> Vec<&str> {
        h.entries(ty)
            .iter()
            .map(|l| std::str::from_utf8(l).unwrap())
            .collect()
    }

    #[test]
    fn types() {
        assert_eq!(PromptType::from_name(b"command"), Some(PromptType::Command));
        assert_eq!(PromptType::from_name(b"search"), Some(PromptType::Search));
        assert_eq!(PromptType::from_name(b"target"), None);
        assert_eq!(PromptType::from_name(b""), None);
        assert_eq!(PromptType::Search.name(), "search");
    }

    #[test]
    fn add_skips_repeats_and_keeps_the_limit() {
        let mut h = PromptHistory::new();
        h.add(PromptType::Command, b"a", 3);
        h.add(PromptType::Command, b"a", 3);
        h.add(PromptType::Command, b"b", 3);
        h.add(PromptType::Command, b"a", 3);
        assert_eq!(entries(&h, PromptType::Command), ["a", "b", "a"]);
        h.add(PromptType::Command, b"c", 3);
        assert_eq!(entries(&h, PromptType::Command), ["b", "a", "c"]);
        assert!(h.entries(PromptType::Search).is_empty());

        // A lower limit drops the oldest at the next add, also a repeat.
        h.add(PromptType::Command, b"c", 1);
        assert_eq!(entries(&h, PromptType::Command), ["c"]);
        h.add(PromptType::Command, b"d", 0);
        assert!(h.entries(PromptType::Command).is_empty());

        h.add(PromptType::Search, b"s", 10);
        h.clear(PromptType::Search);
        assert!(h.entries(PromptType::Search).is_empty());
    }

    #[test]
    fn up_and_down() {
        let mut h = PromptHistory::new();
        let mut pos = HistoryPositions::default();
        assert_eq!(h.up(PromptType::Command, &mut pos), None);
        assert_eq!(h.down(PromptType::Command, &mut pos), b"");

        h.add(PromptType::Command, b"old", 10);
        h.add(PromptType::Command, b"new", 10);
        assert_eq!(h.up(PromptType::Command, &mut pos), Some(&b"new"[..]));
        assert_eq!(h.up(PromptType::Command, &mut pos), Some(&b"old"[..]));
        assert_eq!(h.up(PromptType::Command, &mut pos), None, "stays at the oldest");
        assert_eq!(pos, [2, 0]);
        assert_eq!(h.down(PromptType::Command, &mut pos), b"new");
        assert_eq!(h.down(PromptType::Command, &mut pos), b"");
        assert_eq!(h.down(PromptType::Command, &mut pos), b"");
        assert_eq!(pos, [0, 0]);
        // Each type moves on its own.
        assert_eq!(h.up(PromptType::Search, &mut pos), None);
    }

    #[test]
    fn file_round_trip() {
        let mut h = PromptHistory::new();
        h.load(
            b"command:a\nsearch:s\nold line\nwindow-target:w\n\ncommand:x:y\nsearch:",
            100,
        );
        assert_eq!(
            entries(&h, PromptType::Command),
            ["a", "old line", "window-target:w", "", "x:y"]
        );
        assert_eq!(entries(&h, PromptType::Search), ["s", ""]);
        assert_eq!(
            h.to_file(),
            b"command:a\ncommand:old line\ncommand:window-target:w\ncommand:\ncommand:x:y\nsearch:s\nsearch:\n"
        );
        let mut empty = PromptHistory::new();
        empty.load(b"", 100);
        empty.load(b"\n", 100);
        assert_eq!(entries(&empty, PromptType::Command), [""]);

        // Loading keeps the limit.
        let mut h = PromptHistory::new();
        h.load(b"command:1\ncommand:2\ncommand:3\n", 2);
        assert_eq!(entries(&h, PromptType::Command), ["2", "3"]);
    }
}
