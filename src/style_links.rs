// Copyright (c) 2026 Patrick Elsen
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

//! The hyperlinks of styles (`link=` in tmux 3.8), so a style keeps a
//! small number instead of the URI.
//!
//! style.c keeps them in a global hyperlinks set (`style_hyperlinks`). Here
//! the store is a plain intern table reached through the bridge
//! (global.rs): the same URI always gets the same number, numbers start
//! at 1 and 0 means no link.
#![forbid(unsafe_code)]

use std::collections::HashMap;

/// The URIs that styles link to, by number.
#[derive(Default)]
pub struct StyleLinks {
    uris: Vec<Vec<u8>>,
    by_uri: HashMap<Vec<u8>, u32>,
}

impl StyleLinks {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of `uri`, added when it is new.
    pub fn put(&mut self, uri: &[u8]) -> u32 {
        if let Some(&id) = self.by_uri.get(uri) {
            return id;
        }
        self.uris.push(uri.to_vec());
        let id = u32::try_from(self.uris.len()).expect("too many style links");
        self.by_uri.insert(uri.to_vec(), id);
        id
    }

    /// The URI of link `id`, None for 0 or a number never handed out.
    pub fn get(&self, id: u32) -> Option<&[u8]> {
        let index = usize::try_from(id).ok()?.checked_sub(1)?;
        self.uris.get(index).map(Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_uri_same_number() {
        let mut links = StyleLinks::new();
        let a = links.put(b"https://a");
        let b = links.put(b"https://b");
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(links.put(b"https://a"), a);
        assert_eq!(links.get(a), Some(&b"https://a"[..]));
        assert_eq!(links.get(b), Some(&b"https://b"[..]));
        assert_eq!(links.get(0), None);
        assert_eq!(links.get(3), None);
    }
}
