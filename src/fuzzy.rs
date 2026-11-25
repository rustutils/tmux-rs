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
//! Fuzzy matching in the style of fzf (fuzzy.c).
//!
//! The pattern is split into groups by `|` and each group on spaces into
//! terms. Text matches if any group matches. Within a group every positive
//! term must match and no inverse term may.
//!
//! Plain terms are fuzzy: their characters appear in order, not
//! necessarily next to each other. `'` makes a term an exact substring,
//! `^` anchors it at the start and `$` at the end, `!` inverts it (an
//! inverse term is always exact, as in fzf). Matching is smart case: ASCII
//! case is ignored unless the pattern has an uppercase letter.
//!
//! The text may contain `#[...]` style directives. They take no columns,
//! but `align=` moves the text after them the way `format_draw` lays it
//! out, so the matched columns are where the characters are drawn. Styles
//! are parsed by the caller (see [`fuzzy_match`]).
//!
//! A match gives the display columns of the matched characters and a
//! score: matches at the start, after word boundaries and in runs score
//! higher, so callers can show the best match first.

#![forbid(unsafe_code)]

use crate::bitstr::BitStr;
use crate::style_align;
use crate::utf8::{Utf8Data, Utf8State};

const BONUS_EXACT: i32 = 1000;
const BONUS_PREFIX: i32 = 200;
const BONUS_SUFFIX: i32 = 100;
const BONUS_START: i32 = 12;
const BONUS_BOUNDARY: i32 = 8;
const BONUS_CONSECUTIVE: i32 = 6;
const PENALTY_LEADING: i32 = 1;
const PENALTY_LEADING_MAX: i32 = 10;
const PENALTY_GAP: i32 = 1;

/// The number of alignments, as indices of per-alignment arrays.
const ALIGNS: usize = style_align::STYLE_ALIGN_ABSOLUTE_CENTRE as usize + 1;

/// A successful match.
pub struct FuzzyMatch {
    /// A bit for each display column of a matched character.
    pub columns: BitStr,
    /// Higher is better, never below 0.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "switch-mode ranks its items by it (Track 3)")
    )]
    pub score: u32,
}

/// A visible character of the text.
struct Char {
    align: style_align,
    ud: Utf8Data,
    width: u32,
    /// Its column within its alignment.
    offset: u32,
}

/// One parsed term of the pattern.
#[derive(Default)]
struct Term<'a> {
    inverse: bool,
    exact: bool,
    prefix: bool,
    suffix: bool,
    text: &'a [u8],
}

/// The bytes of a character.
fn bytes(ud: &Utf8Data) -> &[u8] {
    &ud.data[..ud.size as usize]
}

/// Whether a match after this character starts a word.
fn is_boundary(ud: &Utf8Data) -> bool {
    ud.size == 1 && b" -_/.:".contains(&ud.data[0])
}

/// Whether two characters are equal, ignoring ASCII case if `fold`.
fn char_equal(a: &Utf8Data, b: &Utf8Data, fold: bool) -> bool {
    if fold && a.size == 1 && b.size == 1 && a.data[0] < 0x80 && b.data[0] < 0x80 {
        return a.data[0].eq_ignore_ascii_case(&b.data[0]);
    }
    bytes(a) == bytes(b)
}

/// Decodes the character at the start of `s`: a UTF-8 sequence, or the
/// first byte alone. Returns it and the bytes it took.
fn decode_one(s: &[u8]) -> (Utf8Data, usize) {
    let mut ud = Utf8Data::empty();
    let mut more = ud.open(s[0]);
    if more == Utf8State::More {
        let mut i = 1;
        while i < s.len() && more == Utf8State::More {
            more = ud.append(s[i]);
            i += 1;
        }
        if more == Utf8State::Done {
            return (ud, i);
        }
    }
    (Utf8Data::single(s[0]), 1)
}

/// The end of a style directive starting at `s` (`format_skip` with "]"):
/// the index of the first `]` outside any `#{...}`, skipping `#`
/// escapes.
fn skip_to_bracket(s: &[u8]) -> Option<usize> {
    let mut brackets = 0i32;
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'#' && s.get(i + 1) == Some(&b'{') {
            brackets += 1;
        }
        if s[i] == b'#' && s.get(i + 1).is_some_and(|c| b",#{}:".contains(c)) {
            i += 2;
            continue;
        }
        if s[i] == b'}' {
            brackets -= 1;
        }
        if s[i] == b']' && brackets == 0 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Maps the default alignment to left.
fn layout_align(align: style_align) -> style_align {
    if align == style_align::STYLE_ALIGN_DEFAULT {
        style_align::STYLE_ALIGN_LEFT
    } else {
        align
    }
}

/// Scans `text` into its visible characters and the width of each
/// alignment. `parse_style` parses the inside of a style directive and
/// returns the alignment it leaves, or `None` if it is not a valid style.
fn scan(
    text: &[u8],
    parse_style: &mut dyn FnMut(&[u8]) -> Option<style_align>,
) -> (Vec<Char>, [u32; ALIGNS]) {
    let mut cs = Vec::new();
    let mut widths = [0u32; ALIGNS];
    let mut current = style_align::STYLE_ALIGN_LEFT;
    let add = |cs: &mut Vec<Char>, widths: &mut [u32; ALIGNS], a: style_align, ud: Utf8Data| {
        let width = u32::from(ud.width);
        cs.push(Char {
            align: a,
            ud,
            width,
            offset: widths[a as usize],
        });
        widths[a as usize] += width;
    };
    let hash = Utf8Data::single(b'#');
    let bracket = Utf8Data::single(b'[');

    let mut i = 0;
    while i < text.len() {
        if text[i] == b'#' {
            let n = text[i..].iter().take_while(|&&c| c == b'#').count();
            if text.get(i + n) != Some(&b'[') {
                // Escaped #s: ## is #, so half, rounded up.
                for _ in 0..n.div_ceil(2) {
                    add(&mut cs, &mut widths, current, hash);
                }
                i += n;
                continue;
            }
            // An even count: all #s are escaped and the [ is text.
            for _ in 0..n / 2 {
                add(&mut cs, &mut widths, current, hash);
            }
            if n % 2 == 0 {
                add(&mut cs, &mut widths, current, bracket);
                i += n + 1;
                continue;
            }
            // An odd count starts a style.
            let start = i + n + 1;
            let Some(end) = skip_to_bracket(&text[start..]) else {
                break;
            };
            if let Some(align) = parse_style(&text[start..start + end]) {
                current = layout_align(align);
            }
            i = start + end + 1;
            continue;
        }

        let (ud, used) = decode_one(&text[i..]);
        i += used;
        // Control characters and bytes left over from a failed decode
        // take no place.
        if ud.size == 1 && (ud.data[0] <= 0x1f || ud.data[0] >= 0x7f) {
            continue;
        }
        add(&mut cs, &mut widths, current, ud);
    }
    (cs, widths)
}

/// Decodes a term, or `None` if it has more characters than `limit`.
fn decode_term(text: &[u8], limit: usize) -> Option<Vec<Utf8Data>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < text.len() {
        if out.len() == limit {
            return None;
        }
        let (ud, used) = decode_one(&text[i..]);
        out.push(ud);
        i += used;
    }
    Some(out)
}

/// The score of a fuzzy term matched at `pos`.
fn score_positions(pos: &[usize], cs: &[Char]) -> i32 {
    let Some(&first) = pos.first() else {
        return 0;
    };
    let mut score = 0;
    if first == 0 {
        score += BONUS_START;
    } else {
        if is_boundary(&cs[first - 1].ud) {
            score += BONUS_BOUNDARY;
        }
        score -= (first as i32).min(PENALTY_LEADING_MAX) * PENALTY_LEADING;
    }
    for w in pos.windows(2) {
        if w[1] == w[0] + 1 {
            score += BONUS_CONSECUTIVE;
        } else if is_boundary(&cs[w[1] - 1].ud) {
            score += BONUS_BOUNDARY;
        }
    }
    let span = pos[pos.len() - 1] - first + 1;
    let gap = (span - pos.len()) as i32;
    score - gap * PENALTY_GAP
}

/// Matches `tok` as a subsequence of `cs`: the first match from the
/// start, then pulled together from its end for a shorter span.
fn match_fuzzy(tok: &[Utf8Data], cs: &[Char], fold: bool) -> Option<(i32, Vec<usize>)> {
    if tok.is_empty() || cs.is_empty() {
        return None;
    }
    let mut pos = vec![0; tok.len()];
    let mut ci = 0;
    for (pi, t) in tok.iter().enumerate() {
        while ci != cs.len() && !char_equal(t, &cs[ci].ud, fold) {
            ci += 1;
        }
        if ci == cs.len() {
            return None;
        }
        pos[pi] = ci;
        ci += 1;
    }

    let mut ci = pos[tok.len() - 1];
    for pi in (1..=tok.len()).rev() {
        loop {
            if char_equal(&tok[pi - 1], &cs[ci].ud, fold) {
                pos[pi - 1] = ci;
                break;
            }
            if ci == 0 {
                return None;
            }
            ci -= 1;
        }
        if pi != 1 {
            ci -= 1;
        }
    }
    Some((score_positions(&pos, cs), pos))
}

/// The score of an exact term matched at `start`.
fn score_exact(start: usize, toklen: usize, ncs: usize, cs: &[Char], term: &Term) -> i32 {
    let mut score = BONUS_EXACT + toklen as i32 * BONUS_CONSECUTIVE;
    if term.prefix {
        score += BONUS_PREFIX;
    }
    if term.suffix {
        score += BONUS_SUFFIX;
    }
    if start == 0 {
        score += BONUS_START;
    } else if is_boundary(&cs[start - 1].ud) {
        score += BONUS_BOUNDARY;
    }
    score -= (start as i32).min(PENALTY_LEADING_MAX) * PENALTY_LEADING;
    if !term.prefix && !term.suffix {
        score -= (ncs - (start + toklen)) as i32;
    }
    score
}

/// Matches `tok` exactly (anchored for a prefix or suffix term), keeping
/// the best scoring place.
fn match_exact(tok: &[Utf8Data], cs: &[Char], fold: bool, term: &Term) -> Option<(i32, Vec<usize>)> {
    let (toklen, ncs) = (tok.len(), cs.len());
    if toklen == 0 || toklen > ncs {
        return None;
    }
    let starts = match (term.prefix, term.suffix) {
        (true, true) if toklen != ncs => return None,
        (true, _) => 0..1,
        (false, true) => ncs - toklen..ncs - toklen + 1,
        (false, false) => 0..ncs - toklen + 1,
    };
    let mut best: Option<(i32, usize)> = None;
    for i in starts {
        let ok = tok
            .iter()
            .zip(&cs[i..])
            .all(|(t, c)| char_equal(t, &c.ud, fold));
        if !ok {
            continue;
        }
        let value = score_exact(i, toklen, ncs, cs, term);
        if best.is_none_or(|(score, _)| value > score) {
            best = Some((value, i));
        }
    }
    let (score, start) = best?;
    Some((score, (start..start + toklen).collect()))
}

/// Parses one term, `None` if nothing is left after its markers.
fn parse_term(mut s: &[u8]) -> Option<Term<'_>> {
    let mut term = Term::default();
    if let Some(rest) = s.strip_prefix(b"!") {
        term.inverse = true;
        s = rest;
    }
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix(b"'") {
        term.exact = true;
        s = rest;
    } else if let Some(rest) = s.strip_prefix(b"^") {
        term.exact = true;
        term.prefix = true;
        s = rest;
    }
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_suffix(b"$") {
        term.exact = true;
        term.suffix = true;
        s = rest;
    }
    if s.is_empty() {
        return None;
    }
    if term.inverse {
        term.exact = true;
    }
    term.text = s;
    Some(term)
}

/// Matches one term, adding its score and marking its characters.
fn match_term(term: &Term, cs: &[Char], fold: bool, score: &mut i32, matched: &mut [bool]) -> bool {
    let Some(tok) = decode_term(term.text, cs.len()) else {
        return term.inverse;
    };
    let found = if term.exact {
        match_exact(&tok, cs, fold, term)
    } else {
        match_fuzzy(&tok, cs, fold)
    };
    if term.inverse {
        return found.is_none();
    }
    let Some((value, pos)) = found else {
        return false;
    };
    *score += value;
    for p in pos {
        matched[p] = true;
    }
    true
}

/// Matches one group of terms: all must match, and there must be one.
fn match_group(group: &[u8], cs: &[Char], fold: bool, matched: &mut [bool]) -> Option<i32> {
    let mut score = 0;
    let mut any = false;
    for word in group.split(|&c| c == b' ').filter(|w| !w.is_empty()) {
        let term = parse_term(word)?;
        any = true;
        if !match_term(&term, cs, fold, &mut score, matched) {
            return None;
        }
    }
    any.then_some(score)
}

/// Matches `pattern` against `text` drawn `width` columns wide. Returns
/// the columns of the matched characters and the score, or `None` if it
/// does not match. An empty pattern matches with no columns.
///
/// `parse_style` parses the inside of a `#[...]` directive and returns
/// the alignment it leaves, or `None` if it is not a valid style. It is
/// called in order and may keep the style between calls, as C parses
/// each directive on top of the last.
pub fn fuzzy_match(
    pattern: &[u8],
    text: &[u8],
    width: u32,
    mut parse_style: impl FnMut(&[u8]) -> Option<style_align>,
) -> Option<FuzzyMatch> {
    if width == 0 {
        return None;
    }
    if pattern.iter().all(|&c| c == b' ' || c == b'|') {
        return Some(FuzzyMatch {
            columns: BitStr::new(width),
            score: 0,
        });
    }
    let fold = !pattern.iter().any(u8::is_ascii_uppercase);

    let (cs, widths) = scan(text, &mut parse_style);

    let mut best: Option<(i32, Vec<bool>)> = None;
    for group in pattern.split(|&c| c == b'|') {
        let skip = group.iter().take_while(|&&c| c == b' ').count();
        let group = &group[skip..];
        if group.is_empty() {
            continue;
        }
        let mut matched = vec![false; cs.len()];
        if let Some(score) = match_group(group, &cs, fold, &mut matched)
            && best.as_ref().is_none_or(|(b, _)| score > *b)
        {
            best = Some((score, matched));
        }
    }
    let (score, matched) = best?;

    // The trimmed widths and start columns of each alignment, as
    // format_draw_none lays them out.
    use style_align::*;
    let (left, centre, right, abs) = (
        STYLE_ALIGN_LEFT as usize,
        STYLE_ALIGN_CENTRE as usize,
        STYLE_ALIGN_RIGHT as usize,
        STYLE_ALIGN_ABSOLUTE_CENTRE as usize,
    );
    let (mut wl, mut wc, mut wr) = (widths[left], widths[centre], widths[right]);
    while wl + wc + wr > width {
        if wc > 0 {
            wc -= 1;
        } else if wr > 0 {
            wr -= 1;
        } else {
            wl -= 1;
        }
    }
    let wa = widths[abs].min(width);
    let mut start = [0u32; ALIGNS];
    let mut src = [0u32; ALIGNS];
    let mut vis = [0u32; ALIGNS];
    vis[left] = wl;
    start[right] = width - wr;
    src[right] = widths[right] - wr;
    vis[right] = wr;
    start[centre] = (wl + ((width - wr) - wl) / 2).wrapping_sub(wc / 2);
    src[centre] = widths[centre] / 2 - wc / 2;
    vis[centre] = wc;
    start[abs] = (width - wa) / 2;
    vis[abs] = wa;

    let mut columns = BitStr::new(width);
    for (c, _) in cs.iter().zip(&matched).filter(|(_, m)| **m) {
        let a = c.align as usize;
        if c.offset < src[a] || c.offset >= src[a] + vis[a] {
            continue;
        }
        let column = start[a] + (c.offset - src[a]);
        for j in 0..c.width {
            if column + j < width {
                columns.bit_set(column + j);
            }
        }
    }
    Some(FuzzyMatch {
        columns,
        score: score.max(0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Styles as tests need them: "align=..." only.
    fn parse_style(s: &[u8]) -> Option<style_align> {
        match s {
            b"align=left" => Some(style_align::STYLE_ALIGN_LEFT),
            b"align=centre" => Some(style_align::STYLE_ALIGN_CENTRE),
            b"align=right" => Some(style_align::STYLE_ALIGN_RIGHT),
            b"align=absolute-centre" => Some(style_align::STYLE_ALIGN_ABSOLUTE_CENTRE),
            b"fg=red" => Some(style_align::STYLE_ALIGN_DEFAULT),
            _ => None,
        }
    }

    /// The matched columns, or `None` without a match.
    fn columns(pattern: &str, text: &str, width: u32) -> Option<Vec<u32>> {
        let m = fuzzy_match(pattern.as_bytes(), text.as_bytes(), width, parse_style)?;
        Some((0..width).filter(|&i| m.columns.bit_test(i)).collect())
    }

    fn score(pattern: &str, text: &str) -> u32 {
        fuzzy_match(pattern.as_bytes(), text.as_bytes(), 80, parse_style)
            .expect("a match")
            .score
    }

    #[test]
    fn fuzzy_terms() {
        assert_eq!(columns("dvb", "dev:1 bash", 80), Some(vec![0, 2, 6]));
        assert_eq!(columns("bvd", "dev:1 bash", 80), None);
        // The span is pulled together from the end: the last a, not the
        // first.
        assert_eq!(columns("ab", "a a b", 80), Some(vec![2, 4]));
        assert_eq!(columns("x", "", 80), None);
    }

    #[test]
    fn exact_prefix_suffix_and_inverse() {
        assert_eq!(columns("'v:1", "dev:1 bash", 80), Some(vec![2, 3, 4]));
        assert_eq!(columns("'vb", "dev:1 bash", 80), None);
        assert_eq!(columns("^dev", "dev:1 bash", 80), Some(vec![0, 1, 2]));
        assert_eq!(columns("^ev", "dev:1 bash", 80), None);
        assert_eq!(columns("sh$", "dev:1 bash", 80), Some(vec![8, 9]));
        assert_eq!(columns("^dev$", "dev", 80), Some(vec![0, 1, 2]));
        assert_eq!(columns("^de$", "dev", 80), None);
        // Inverse terms are exact and mark nothing.
        assert_eq!(columns("!ssh", "dev:1 bash", 80), Some(vec![]));
        assert_eq!(columns("!bash", "dev:1 bash", 80), None);
        assert_eq!(columns("!dvb", "dev:1 bash", 80), Some(vec![]), "not fuzzy");
        // A term that is only markers ends the group without a match.
        assert_eq!(columns("!", "dev", 80), None);
        assert_eq!(columns("dev ^", "dev", 80), None);
        // An inverse term longer than the text cannot match it.
        assert_eq!(columns("!longer", "dev", 80), Some(vec![]));
    }

    #[test]
    fn groups_and_terms() {
        assert_eq!(columns("^dev | ^prod", "prod:1 ssh", 80), Some(vec![0, 1, 2, 3]));
        assert_eq!(columns("dev bash", "dev:1 bash", 80), Some(vec![0, 1, 2, 6, 7, 8, 9]));
        assert_eq!(columns("dev zsh", "dev:1 bash", 80), None);
        // Empty patterns match everything with nothing marked.
        assert_eq!(columns("", "x", 10), Some(vec![]));
        assert_eq!(columns(" | ", "x", 10), Some(vec![]));
        assert_eq!(columns("x", "x", 0), None);
    }

    #[test]
    fn smart_case() {
        assert!(columns("DEV", "dev", 80).is_none());
        assert!(columns("dev", "DEV", 80).is_some());
        assert!(columns("Dev", "Dev", 80).is_some());
    }

    #[test]
    fn styles_take_no_columns() {
        assert_eq!(columns("b", "#[fg=red]ab", 80), Some(vec![1]));
        assert_eq!(columns("b", "##[ab", 80), Some(vec![3]), "## then a literal [");
        assert_eq!(columns("[", "##[ab", 80), Some(vec![1]));
        assert_eq!(columns("b", "#[nonsense]ab", 80), Some(vec![1]));
        // An unclosed style ends the text.
        assert_eq!(columns("b", "a#[fg=red b", 80), None);
        // Control characters take no place.
        assert_eq!(columns("b", "a\u{1}b", 80), Some(vec![1]));
    }

    #[test]
    fn alignment_moves_columns() {
        assert_eq!(columns("x", "#[align=right]x", 10), Some(vec![9]));
        assert_eq!(columns("x", "#[align=centre]x", 11), Some(vec![5]));
        assert_eq!(columns("x", "#[align=absolute-centre]x", 11), Some(vec![5]));
        assert_eq!(columns("b", "a#[align=right]b", 10), Some(vec![9]));
        // Text that does not fit is trimmed: centre first, then right
        // (from its start), then left. Here right keeps only its z.
        assert_eq!(columns("z", "abc#[align=right]xyz", 4), Some(vec![3]));
        assert_eq!(columns("y", "abc#[align=right]xyz", 4), Some(vec![]));
        assert_eq!(columns("c", "abc#[align=right]xyz", 4), Some(vec![2]));
    }

    #[test]
    fn scores_rank_better_matches_higher() {
        assert!(score("dev", "dev") > score("dev", "a dev"));
        // Start 12, run 6: 18. A boundary (8) outweighs the gap (1): 19.
        // A plain gap: 11.
        assert_eq!(score("ab", "ab"), 18);
        assert_eq!(score("ab", "a-b"), 19);
        assert_eq!(score("ab", "axb"), 11);
        assert!(score("'dev", "dev") > score("dev", "dev"));
        assert!(score("^dev", "dev x") > score("'dev", "dev x"));
        let far = format!("{}a{}b", "x".repeat(12), "x".repeat(20));
        assert_eq!(score("ab", &far), 0, "never below 0");
    }
}
