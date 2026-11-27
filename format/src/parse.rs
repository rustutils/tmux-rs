//! Parsing a format into a tree.
//!
//! The tree keeps the structure that 3.8's format_expand1 finds by
//! scanning: literal text, `#()` jobs and `#{}` replacements (also the
//! single letter aliases such as `#S`). A replacement holds its modifiers
//! with their arguments already parsed, and the text after the modifiers
//! (C's `copy`) as raw bytes. How that text is read depends on the
//! modifiers (a condition, a comparison, a loop, a name), and in two cases
//! (`N` and `I`) on what an argument expands to, so each reading is built
//! the first time it is needed and kept in the node.
//!
//! Every node keeps its raw bytes too. C applies strftime to the raw text
//! of every expansion level when expanding with time (`T:` and status
//! lines), before it scans, so in that mode the evaluator formats the raw
//! text and parses the result again.

use std::cell::OnceCell;

/// The characters that `#` escapes inside a replacement.
const ESCAPED: &[u8] = b",#{}:";

/// The single letter aliases (format_upper and format_lower in C).
pub(crate) fn alias(ch: u8) -> Option<&'static [u8]> {
    Some(match ch {
        b'D' => b"pane_id",
        b'F' => b"window_flags",
        b'H' => b"host",
        b'I' => b"window_index",
        b'P' => b"pane_index",
        b'S' => b"session_name",
        b'T' => b"pane_title",
        b'W' => b"window_name",
        b'h' => b"host_short",
        _ => return None,
    })
}

/// A parsed format.
#[derive(Debug)]
pub struct Format {
    /// The text this was parsed from.
    pub(crate) raw: Vec<u8>,
    /// What expanding it produces, in order.
    pub(crate) parts: Vec<Part>,
}

/// One piece of a format.
#[derive(Debug)]
pub(crate) enum Part {
    /// Bytes copied to the output as they are.
    Text(Vec<u8>),
    /// `#(command)`: the raw command and the command parsed.
    Job(Vec<u8>, Box<Format>),
    /// `#{...}`.
    Replace(Box<Replace>),
    /// A single letter alias such as `#S`, with its letter. C finds no
    /// end for it, so it skips the time check that the scan for the end
    /// of a `#{` makes.
    Alias(u8, Box<Replace>),
    /// Nothing to output, but C logs it here when it expands verbosely.
    Mark(Mark),
    /// C stops expanding here: a `#{` or `#(` without its end. Output
    /// so far is kept, the rest is dropped.
    Stop,
}

/// What format_expand1 logs while it copies text.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mark {
    /// A style, `#[` after this many characters ("found #*%zu[").
    Style(usize),
    /// An escaped `}` or `,` ("found #%c").
    Escape(u8),
}

/// A modifier and its arguments.
#[derive(Debug)]
pub(crate) struct Modifier {
    /// One or two characters (`=`, `s`, `||`, `!=`).
    pub(crate) name: Vec<u8>,
    /// The arguments, unescaped and parsed. C expands them while it
    /// builds the modifier list, before it looks at anything else.
    pub(crate) args: Vec<Format>,
}

/// A replacement, `#{modifiers:text}`.
#[derive(Debug)]
pub(crate) struct Replace {
    /// The whole text inside the `#{}` (or the name of an alias), which
    /// C logs.
    pub(crate) key: Vec<u8>,
    /// The modifiers. Empty when the list was malformed, then `copy` is
    /// the whole key, as in C.
    pub(crate) mods: Vec<Modifier>,
    /// The text after the modifiers, raw.
    pub(crate) copy: Vec<u8>,
    /// `copy` parsed as one format.
    whole: OnceCell<Format>,
    /// `copy` split at its first unescaped comma (format_choose).
    choose: OnceCell<Option<(Format, Format)>>,
    /// `copy` split at every unescaped comma, each part with its raw
    /// bytes (the operands of `||` and `&&`).
    split: OnceCell<Vec<(Vec<u8>, Format)>>,
    /// `copy` after the `?`, split the same way (the conditional).
    cond: OnceCell<Vec<(Vec<u8>, Format)>>,
    /// `copy` unescaped (the `l` modifier).
    literal: OnceCell<Vec<u8>>,
}

impl Replace {
    /// The text after the modifiers as one format.
    pub(crate) fn whole(&self) -> &Format {
        self.whole.get_or_init(|| parse(&self.copy))
    }

    /// The text split at the first comma, both sides parsed, or None
    /// without a comma.
    pub(crate) fn choose(&self) -> Option<&(Format, Format)> {
        self.choose
            .get_or_init(|| {
                let at = skip(&self.copy, 0, b",")?;
                Some((parse(&self.copy[..at]), parse(&self.copy[at + 1..])))
            })
            .as_ref()
    }

    /// The text split at every comma.
    pub(crate) fn split(&self) -> &[(Vec<u8>, Format)] {
        self.split.get_or_init(|| split_commas(&self.copy))
    }

    /// The conditional's pieces, the text after `?` split at every comma.
    pub(crate) fn cond(&self) -> &[(Vec<u8>, Format)] {
        self.cond.get_or_init(|| split_commas(&self.copy[1..]))
    }

    /// The text unescaped, for `l`.
    pub(crate) fn literal(&self) -> &[u8] {
        self.literal.get_or_init(|| unescape(&self.copy))
    }
}

/// Splits at unescaped commas the way C does when it calls format_skip
/// again after each comma: each scan starts with no open brackets.
fn split_commas(s: &[u8]) -> Vec<(Vec<u8>, Format)> {
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        match skip(s, start, b",") {
            Some(at) => {
                out.push((s[start..at].to_vec(), parse(&s[start..at])));
                start = at + 1;
            }
            None => {
                out.push((s[start..].to_vec(), parse(&s[start..])));
                return out;
            }
        }
    }
}

/// The byte at `i`, or 0 past the end, so code ported from C can read
/// one past the end of a string as C reads its terminator.
fn at(s: &[u8], i: usize) -> u8 {
    s.get(i).copied().unwrap_or(0)
}

/// format_skip: finds the first byte from `start` that is in `end` and
/// not inside a `#{}` or escaped with `#`. A `}` closes a bracket even
/// when none is open, so the count can go below zero, as in C.
pub fn skip(s: &[u8], start: usize, end: &[u8]) -> Option<usize> {
    let mut brackets = 0i32;
    let mut i = start;
    while i < s.len() {
        if s[i] == b'#' && at(s, i + 1) == b'{' {
            brackets += 1;
        }
        if s[i] == b'#' && i + 1 < s.len() && ESCAPED.contains(&s[i + 1]) {
            i += 2;
            continue;
        }
        if s[i] == b'}' {
            brackets -= 1;
        }
        if end.contains(&s[i]) && brackets == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// format_unescape: removes the `#` from escapes outside any `#{}`.
pub fn unescape(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut brackets = 0i32;
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'#' && at(s, i + 1) == b'{' {
            brackets += 1;
        }
        if brackets == 0 && s[i] == b'#' && i + 1 < s.len() && ESCAPED.contains(&s[i + 1]) {
            out.push(s[i + 1]);
            i += 2;
            continue;
        }
        if s[i] == b'}' {
            brackets -= 1;
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// format_strip: drops the `#` of each escape outside `#{}` and keeps
/// the character after it (used for the time format of `t/f`). A `#` at
/// the end is dropped too, because C's strchr finds the terminator in the
/// set.
pub(crate) fn strip(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut brackets = 0i32;
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'#' && at(s, i + 1) == b'{' {
            brackets += 1;
        }
        if s[i] == b'#' && (i + 1 == s.len() || ESCAPED.contains(&s[i + 1])) {
            if brackets != 0 {
                out.push(b'#');
            }
            i += 1;
            continue;
        }
        if s[i] == b'}' {
            brackets -= 1;
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

/// Parses a format. Never fails: input C stops at becomes a Stop part.
/// Input is read up to its first NUL, as C reads a string.
pub fn parse(input: &[u8]) -> Format {
    let raw = match input.iter().position(|&b| b == 0) {
        Some(n) => &input[..n],
        None => input,
    };
    let mut parts = Vec::new();
    let mut text = Vec::new();
    // Where the last #[ style ends. Aliases are not expanded before it.
    let mut style_end: Option<usize> = None;
    let mut i = 0;

    let flush = |text: &mut Vec<u8>, parts: &mut Vec<Part>| {
        if !text.is_empty() {
            parts.push(Part::Text(std::mem::take(text)));
        }
    };

    while i < raw.len() {
        if raw[i] != b'#' {
            text.push(raw[i]);
            i += 1;
            continue;
        }
        i += 1;
        if i == raw.len() {
            break;
        }
        let ch = raw[i];
        i += 1;
        // `i` is now C's fmt: just after the character following the #.
        match ch {
            b'(' => {
                let mut brackets = 1;
                let mut ptr = i;
                while ptr < raw.len() {
                    if raw[ptr] == b'(' {
                        brackets += 1;
                    }
                    if raw[ptr] == b')' {
                        brackets -= 1;
                        if brackets == 0 {
                            break;
                        }
                    }
                    ptr += 1;
                }
                if ptr == raw.len() || brackets != 0 {
                    flush(&mut text, &mut parts);
                    parts.push(Part::Stop);
                    break;
                }
                flush(&mut text, &mut parts);
                let cmd = raw[i..ptr].to_vec();
                let parsed = Box::new(parse(&cmd));
                parts.push(Part::Job(cmd, parsed));
                i = ptr + 1;
            }
            b'{' => {
                let Some(end) = skip(raw, i - 2, b"}") else {
                    flush(&mut text, &mut parts);
                    parts.push(Part::Stop);
                    break;
                };
                flush(&mut text, &mut parts);
                parts.push(Part::Replace(Box::new(parse_replace(&raw[i..end]))));
                i = end + 1;
            }
            b'[' | b'#' => {
                let mut ptr = if ch == b'[' { i - 1 } else { i };
                let mut n = if ch == b'[' { 1 } else { 2 };
                while at(raw, ptr) == b'#' {
                    ptr += 1;
                    n += 1;
                }
                if at(raw, ptr) == b'[' {
                    style_end = skip(raw, i - 2, b"]");
                    flush(&mut text, &mut parts);
                    parts.push(Part::Mark(Mark::Style(n)));
                    text.extend_from_slice(&raw[i - 2..i - 2 + n + 1]);
                    i = ptr + 1;
                    continue;
                }
                // Only ## reaches here, #[ always finds its [.
                debug_assert_eq!(ch, b'#');
                text.extend(std::iter::repeat_n(b'#', n / 2));
                i = if n % 2 != 0 { ptr - 1 } else { ptr };
            }
            b'}' | b',' => {
                flush(&mut text, &mut parts);
                parts.push(Part::Mark(Mark::Escape(ch)));
                text.push(ch);
            }
            _ => {
                let after_style = style_end.is_none_or(|end| i > end);
                match alias(ch).filter(|_| after_style) {
                    Some(name) => {
                        flush(&mut text, &mut parts);
                        parts.push(Part::Alias(ch, Box::new(parse_replace(name))));
                    }
                    None => {
                        text.push(b'#');
                        text.push(ch);
                    }
                }
            }
        }
    }
    flush(&mut text, &mut parts);
    Format {
        raw: raw.to_vec(),
        parts,
    }
}

/// Whether a byte ends a modifier.
fn is_end(c: u8) -> bool {
    c == b';' || c == b':'
}

/// Parses the inside of a `#{}` (format_build_modifiers).
pub(crate) fn parse_replace(key: &[u8]) -> Replace {
    let (mods, copy) = match build_modifiers(key) {
        Some((mods, rest)) => (mods, key[rest..].to_vec()),
        None => (Vec::new(), key.to_vec()),
    };
    Replace {
        key: key.to_vec(),
        mods,
        copy,
        whole: OnceCell::new(),
        choose: OnceCell::new(),
        split: OnceCell::new(),
        cond: OnceCell::new(),
        literal: OnceCell::new(),
    }
}

/// format_build_modifiers. Returns the modifiers and where the text after
/// them starts, or None when the list does not end with a colon.
fn build_modifiers(k: &[u8]) -> Option<(Vec<Modifier>, usize)> {
    const SINGLE: &[u8] = b"labdnwETSWPOVL!<>A";
    const DOUBLE: [&[u8]; 7] = [b"||", b"&&", b"!!", b"!=", b"==", b"<=", b">="];
    const WITH_ARGS: &[u8] = b"ImCLNPSOVst=pReqWcA";

    let mut list = Vec::new();
    let mut cp = 0;
    while at(k, cp) != 0 && at(k, cp) != b':' {
        if at(k, cp) == b';' {
            cp += 1;
        }
        if at(k, cp) == 0 {
            break;
        }
        let c = at(k, cp);

        if SINGLE.contains(&c) && is_end(at(k, cp + 1)) {
            list.push(Modifier {
                name: vec![c],
                args: Vec::new(),
            });
            cp += 1;
            continue;
        }
        if cp + 2 <= k.len() && DOUBLE.contains(&&k[cp..cp + 2]) && is_end(at(k, cp + 2)) {
            list.push(Modifier {
                name: k[cp..cp + 2].to_vec(),
                args: Vec::new(),
            });
            cp += 2;
            continue;
        }
        if !WITH_ARGS.contains(&c) {
            break;
        }
        if is_end(at(k, cp + 1)) {
            list.push(Modifier {
                name: vec![c],
                args: Vec::new(),
            });
            cp += 1;
            continue;
        }

        // One argument without a wrapper character. ispunct is ASCII
        // only, in the UTF-8 locale tmux runs in as well.
        let next = at(k, cp + 1);
        if !next.is_ascii_punctuation() || next == b'-' {
            let Some(end) = skip(k, cp + 1, b":;") else {
                break;
            };
            let arg = parse(&unescape(&k[cp + 1..end]));
            list.push(Modifier {
                name: vec![c],
                args: vec![arg],
            });
            cp = end;
            continue;
        }

        // Several arguments between a wrapper character.
        let last = [next, b';', b':'];
        cp += 1;
        let mut args = Vec::new();
        loop {
            if at(k, cp) == last[0] && is_end(at(k, cp + 1)) {
                cp += 1;
                break;
            }
            let Some(end) = skip(k, cp + 1, &last) else {
                break;
            };
            cp += 1;
            args.push(parse(&unescape(&k[cp..end])));
            cp = end;
            if is_end(at(k, cp)) {
                break;
            }
        }
        list.push(Modifier {
            name: vec![c],
            args,
        });
    }
    if at(k, cp) != b':' {
        return None;
    }
    Some((list, cp + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_brackets() {
        assert_eq!(skip(b"a,b", 0, b","), Some(1));
        assert_eq!(skip(b"#{a,b},c", 0, b","), Some(6));
        assert_eq!(skip(b"a#,b,c", 0, b","), Some(4));
        assert_eq!(skip(b"}a,b", 0, b","), None);
    }

    #[test]
    fn modifiers() {
        let r = parse_replace(b"s/a/b/;=5:name");
        assert_eq!(r.mods.len(), 2);
        assert_eq!(r.mods[0].args.len(), 2);
        assert_eq!(r.copy, b"name");
        let r = parse_replace(b"x:b");
        assert!(r.mods.is_empty());
        assert_eq!(r.copy, b"x:b");
    }

    #[test]
    fn strip_and_unescape() {
        assert_eq!(unescape(b"a#,b#}"), b"a,b}");
        assert_eq!(strip(b"%Y#,x#"), b"%Y,x");
    }
}
