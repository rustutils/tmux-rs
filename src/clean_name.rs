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

//! Checking and cleaning user supplied names, from tmux.c in 3.8.
//!
//! tmux 3.8 runs buffer, session and window names, pane titles and paths
//! through these before storing them. So far only paste buffers use them.
#![forbid(unsafe_code)]

use tmux_utf8::utf8_is_valid;

/// Whether `name` may be used as a name: it must be printable ASCII and
/// valid UTF-8 only, with no control characters.
pub fn check_name(name: &[u8]) -> bool {
    utf8_is_valid(name)
}

/// Return the name tmux stores for `name`, or `None` if it is not valid
/// (see [`check_name`]).
///
/// C passes the name through `utf8_stravis` with `VIS_OCTAL`, `VIS_CSTYLE`,
/// `VIS_TAB` and `VIS_NL`. Once the name is known to be valid the only byte
/// that changes is a backslash, which becomes two backslashes, so that is
/// what this does. If `untrusted` is set (names that came from a program
/// running in a pane), a `#` followed by `(` becomes `_`, so the name
/// cannot run a shell command when it is expanded as a format.
pub fn clean_name(name: &[u8], untrusted: bool) -> Option<String> {
    if !check_name(name) {
        return None;
    }
    // Valid UTF-8 was just checked.
    let name = std::str::from_utf8(name).ok()?;
    let mut out = String::with_capacity(name.len());
    let mut chars = name.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '#' if untrusted && chars.peek() == Some(&'(') => out.push('_'),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_names_are_unchanged() {
        assert_eq!(clean_name(b"buffer0", false).as_deref(), Some("buffer0"));
        assert_eq!(clean_name(b"my buf", false).as_deref(), Some("my buf"));
        assert_eq!(clean_name("caf\u{e9}".as_bytes(), false).as_deref(), Some("caf\u{e9}"));
        assert_eq!(clean_name(b"", false).as_deref(), Some(""));
    }

    #[test]
    fn backslash_is_doubled() {
        // tmux 3.8: set-buffer -b 'a\b' x, list-buffers shows a\\b.
        assert_eq!(clean_name(b"a\\b", false).as_deref(), Some("a\\\\b"));
    }

    #[test]
    fn control_characters_and_bad_utf8_are_rejected() {
        assert_eq!(clean_name(b"a\tb", false), None);
        assert_eq!(clean_name(b"n\x01", false), None);
        assert_eq!(clean_name(b"bad\xff", false), None);
        assert!(!check_name(b"line\n"));
        assert!(check_name(b"ok"));
    }

    #[test]
    fn untrusted_names_cannot_run_commands() {
        assert_eq!(clean_name(b"#(x)", false).as_deref(), Some("#(x)"));
        assert_eq!(clean_name(b"#(x)#{y}", true).as_deref(), Some("_(x)#{y}"));
        assert_eq!(clean_name(b"##(", true).as_deref(), Some("#_("));
    }
}
