// Copyright (c) 2023 Nicholas Marriott <nicholas.marriott@gmail.com>
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
//! Combined characters (utf8-combined.c) and the decision
//! screen_write_combine makes: whether a character joins the cell before
//! it (zero width characters, zero width joiners, variation selectors,
//! flags, skin tones and Hangul Jamo).
//!
//! Both the screen writer and format drawing (tmux-format) put
//! characters into cells this way, so the decision lives here as
//! [`utf8_combine`].

#![forbid(unsafe_code)]

use libc::wchar_t;

use crate::{UTF8_SIZE, Utf8Data};

/// The zero width joiner, U+200D.
const ZWJ: &[u8] = "\u{200d}".as_bytes();

/// Whether `ud` ends with a zero width joiner.
pub fn utf8_has_zwj(ud: &Utf8Data) -> bool {
    ud.initialized_slice().ends_with(ZWJ)
}

/// Whether `ud` is a zero width joiner.
pub fn utf8_is_zwj(ud: &Utf8Data) -> bool {
    ud.initialized_slice() == ZWJ
}

/// Whether this is variation selector 16, U+FE0F, which asks for the
/// emoji presentation of the character before it.
pub fn utf8_is_vs(ud: &Utf8Data) -> bool {
    ud.initialized_slice() == "\u{fe0f}".as_bytes()
}

/// Whether this is the Hangul filler U+3164, which tmux ignores.
pub fn utf8_is_hangul_filler(ud: &Utf8Data) -> bool {
    ud.initialized_slice() == "\u{3164}".as_bytes()
}

/// Count the regional indicator characters (U+1F1E6 to U+1F1FF) in `ud`.
fn utf8_regional_count(ud: &Utf8Data) -> usize {
    ud.initialized_slice()
        .windows(4)
        .filter(|w| w[0] == 0xf0 && w[1] == 0x9f && w[2] == 0x87 && (0xa6..=0xbf).contains(&w[3]))
        .count()
}

/// Emoji that take a skin tone modifier (U+1F3FB to U+1F3FF) after them.
static UTF8_SKIN_TONE_BASES: [wchar_t; 72] = [
    0x1F44B, 0x1F44C, 0x1F44D, 0x1F44E, 0x1F44F, 0x1F450, 0x1F466, 0x1F467, 0x1F468, 0x1F469,
    0x1F46E, 0x1F470, 0x1F471, 0x1F472, 0x1F473, 0x1F474, 0x1F475, 0x1F476, 0x1F477, 0x1F478,
    0x1F47C, 0x1F481, 0x1F482, 0x1F483, 0x1F485, 0x1F486, 0x1F487, 0x1F4AA, 0x1F575, 0x1F57A,
    0x1F590, 0x1F595, 0x1F596, 0x1F645, 0x1F646, 0x1F647, 0x1F64B, 0x1F64C, 0x1F64D, 0x1F64E,
    0x1F64F, 0x1F6B4, 0x1F6B5, 0x1F6B6, 0x1F926, 0x1F937, 0x1F938, 0x1F939, 0x1F93D, 0x1F93E,
    0x1F9B5, 0x1F9B6, 0x1F9B8, 0x1F9B9, 0x1F9CD, 0x1F9CE, 0x1F9CF, 0x1F9D1, 0x1F9D2, 0x1F9D3,
    0x1F9D4, 0x1F9D5, 0x1F9D6, 0x1F9D7, 0x1F9D8, 0x1F9D9, 0x1F9DA, 0x1F9DB, 0x1F9DC, 0x1F9DD,
    0x1F9DE, 0x1F9DF,
];

/// Whether `add` should combine with `with`, the cell before it. Two
/// single regional indicators make a flag, and a skin tone modifier
/// combines with an emoji that takes one. Only the first character of
/// each is looked at.
pub fn utf8_should_combine(with: &Utf8Data, add: &Utf8Data) -> bool {
    let (Some(w), Some(a)) = (with.to_wchar(), add.to_wchar()) else {
        return false;
    };
    let regional = 0x1F1E6..=0x1F1FF;
    if regional.contains(&a) && regional.contains(&w) {
        return utf8_regional_count(with) == 1 && utf8_regional_count(add) == 1;
    }
    UTF8_SKIN_TONE_BASES.contains(&a) && (0x1F3FB..=0x1F3FF).contains(&w)
}

/// The kind of a Korean Hangul Jamo character: an initial consonant
/// (choseong), a vowel (jungseong) or a final consonant (jongseong).
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
enum HangulJamoClass {
    NotHangulJamo,
    Choseong,
    Jungseong,
    Jongseong,
}

/// Classify the three byte UTF-8 sequence at the start of `s`. Covers the
/// Hangul Jamo block (U+1100 to U+11FF, including old forms and fillers)
/// and the extended blocks A (U+A960 to U+A97C) and B (U+D7B0 to U+D7FB).
fn hanguljamo_get_class(s: &[u8]) -> HangulJamoClass {
    use HangulJamoClass::*;
    let [a, b, c] = s[..3] else { unreachable!() };
    match (a, b) {
        (0xe1, 0x84) => Choseong,
        (0xe1, 0x85) if c <= 0x9f => Choseong,
        (0xe1, 0x85) => Jungseong,
        (0xe1, 0x86) if c <= 0xa7 => Jungseong,
        (0xe1, 0x86) | (0xe1, 0x87) => Jongseong,
        (0xea, 0xa5) if (0xa0..=0xbc).contains(&c) => Choseong,
        (0xed, 0x9e) if (0xb0..=0xbf).contains(&c) => Jungseong,
        (0xed, 0x9f) if c <= 0x86 => Jungseong,
        (0xed, 0x9f) if (0x8b..=0xbb).contains(&c) => Jongseong,
        _ => NotHangulJamo,
    }
}

/// How a Hangul Jamo character relates to the cell before it.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum HangulJamoState {
    /// Not a Hangul Jamo character.
    NotHangulJamo,
    /// An initial consonant, which starts a new syllable.
    Choseong,
    /// A vowel after an initial consonant, or a final consonant after a
    /// vowel, so it joins the syllable in the previous cell.
    Composable,
    /// A vowel or final consonant that cannot join the previous cell.
    NotComposable,
}

/// Work out whether `ud` composes with `prev`, the cell before it.
pub fn hanguljamo_check_state(prev: &Utf8Data, ud: &Utf8Data) -> HangulJamoState {
    use HangulJamoState::*;
    if ud.size != 3 {
        return NotHangulJamo;
    }
    let follows = match hanguljamo_get_class(ud.initialized_slice()) {
        HangulJamoClass::NotHangulJamo => return NotHangulJamo,
        HangulJamoClass::Choseong => return Choseong,
        HangulJamoClass::Jungseong => HangulJamoClass::Choseong,
        HangulJamoClass::Jongseong => HangulJamoClass::Jungseong,
    };
    let prev = prev.initialized_slice();
    if prev.len() >= 3 && hanguljamo_get_class(&prev[prev.len() - 3..]) == follows {
        Composable
    } else {
        NotComposable
    }
}

/// What to do with a character written after a cell, the decision of
/// screen_write_combine.
#[derive(Copy, Clone)]
pub enum Combine {
    /// Drop the character: it makes no sense on its own and cannot join
    /// the cell before (C returns 1 without combining).
    Discard,
    /// Write it in cells of its own.
    Separate,
    /// Replace the data of the cell before with `data`, the two joined.
    /// When `widen` is set the cell was one column wide and becomes two
    /// (a modifier or variation selector), so the column after it turns
    /// into padding and the cursor moves one further.
    Combined {
        /// The joined character.
        data: Utf8Data,
        /// Whether the cell becomes two columns wide.
        widen: bool,
    },
}

/// Decides whether `ud` joins `last`, the character of the cell the
/// cursor follows, as screen_write_combine does. `last` is None when
/// there is no cell to join: at the left edge, or when the cell before
/// is padding or its width does not match how far back it starts (the
/// caller looks one cell back, or two when the cell before is padding
/// and not in the first column).
///
/// The variation selector always widens, as 3.8 does with its
/// variation-selector-always-wide option on (the default).
pub fn utf8_combine(last: Option<&Utf8Data>, ud: &Utf8Data) -> Combine {
    // Ignore U+3164 HANGUL FILLER entirely.
    if utf8_is_hangul_filler(ud) {
        return Combine::Discard;
    }

    // Is this a character which makes no sense without being combined?
    // Then it is discarded if it cannot be combined.
    let mut force_wide = false;
    let zero_width = if utf8_is_zwj(ud) {
        true
    } else if utf8_is_vs(ud) {
        force_wide = true;
        true
    } else {
        ud.width == 0
    };
    let alone = if zero_width {
        Combine::Discard
    } else {
        Combine::Separate
    };

    // Cannot combine an empty character or with nothing.
    if ud.size < 2 {
        return alone;
    }
    let Some(last) = last else {
        return alone;
    };

    // Check if we need to combine characters. This could be a Korean
    // Hangul Jamo character, zero width (set above), a modifier character
    // (with an existing Unicode character) or a previous ZWJ.
    if !zero_width {
        match hanguljamo_check_state(last, ud) {
            HangulJamoState::NotComposable => return Combine::Discard,
            HangulJamoState::Choseong => return Combine::Separate,
            HangulJamoState::Composable => (),
            HangulJamoState::NotHangulJamo => {
                if utf8_should_combine(last, ud) || utf8_should_combine(ud, last) {
                    force_wide = true;
                } else if !utf8_has_zwj(last) {
                    return Combine::Separate;
                }
            }
        }
    }

    // Check if this combined character would be too long.
    let (old, add) = (last.size as usize, ud.size as usize);
    if old + add > UTF8_SIZE {
        return alone;
    }

    let mut data = *last;
    data.data[old..old + add].copy_from_slice(ud.initialized_slice());
    data.size += ud.size;
    // Force the width to 2 for modifiers and variation selector.
    let widen = data.width == 1 && force_wide;
    if widen {
        data.width = 2;
    }
    Combine::Combined { data, widen }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ud(s: &str) -> Utf8Data {
        let mut ud = Utf8Data::new([0u8; 0], 0, 0, 0);
        ud.data[..s.len()].copy_from_slice(s.as_bytes());
        ud.size = s.len() as u8;
        ud.have = ud.size;
        ud.width = 1;
        ud
    }

    #[test]
    fn variation_selector_16() {
        assert!(utf8_is_vs(&ud("\u{fe0f}")));
        assert!(!utf8_is_vs(&ud("\u{ffcf}")));
        assert!(!utf8_is_vs(&ud("\u{fe0e}")));
    }

    #[test]
    fn zero_width_joiner() {
        assert!(utf8_is_zwj(&ud("\u{200d}")));
        assert!(!utf8_is_zwj(&ud("a\u{200d}")));
        assert!(utf8_has_zwj(&ud("a\u{200d}")));
        assert!(!utf8_has_zwj(&ud("a")));
    }

    #[test]
    fn hangul_jamo_states() {
        use HangulJamoState::*;
        let (cho, jung, jong) = (ud("\u{1100}"), ud("\u{1161}"), ud("\u{11a8}"));
        let syllable = ud("\u{1100}\u{1161}");
        assert_eq!(hanguljamo_check_state(&ud("a"), &ud("b")), NotHangulJamo);
        assert_eq!(hanguljamo_check_state(&jung, &cho), Choseong);
        assert_eq!(hanguljamo_check_state(&cho, &jung), Composable);
        assert_eq!(hanguljamo_check_state(&syllable, &jong), Composable);
        assert_eq!(hanguljamo_check_state(&ud("a"), &jung), NotComposable);
        assert_eq!(hanguljamo_check_state(&cho, &jong), NotComposable);
        // Fillers, old forms and the extended blocks.
        assert_eq!(
            hanguljamo_check_state(&ud("\u{115f}"), &ud("\u{1160}")),
            Composable
        );
        assert_eq!(
            hanguljamo_check_state(&ud("\u{a960}"), &ud("\u{d7b0}")),
            Composable
        );
        assert_eq!(
            hanguljamo_check_state(&ud("\u{d7c6}"), &ud("\u{d7cb}")),
            Composable
        );
        assert_eq!(
            hanguljamo_check_state(&ud("\u{11a7}"), &ud("\u{11ff}")),
            Composable
        );
    }

    #[test]
    fn hangul_filler() {
        assert!(utf8_is_hangul_filler(&ud("\u{3164}")));
        assert!(!utf8_is_hangul_filler(&ud("\u{3163}")));
        assert!(!utf8_is_hangul_filler(&ud("a")));
    }

    #[test]
    fn combine_decisions() {
        let mut acute = ud("\u{301}");
        acute.width = 0;
        // A zero width character joins the cell before, or is dropped.
        assert!(matches!(utf8_combine(None, &acute), Combine::Discard));
        match utf8_combine(Some(&ud("e")), &acute) {
            Combine::Combined { data, widen } => {
                assert_eq!(data.initialized_slice(), "e\u{301}".as_bytes());
                assert_eq!(data.width, 1);
                assert!(!widen);
            }
            _ => panic!("not combined"),
        }
        // The variation selector widens a narrow character.
        let mut vs = ud("\u{fe0f}");
        vs.width = 0;
        assert!(matches!(
            utf8_combine(Some(&ud("a")), &vs),
            Combine::Combined { widen: true, .. }
        ));
        // Plain characters stay separate, the filler is dropped.
        assert!(matches!(
            utf8_combine(Some(&ud("a")), &ud("b")),
            Combine::Separate
        ));
        assert!(matches!(
            utf8_combine(Some(&ud("a")), &ud("\u{3164}")),
            Combine::Discard
        ));
        // A vowel that cannot join is dropped, an initial consonant not.
        assert!(matches!(
            utf8_combine(Some(&ud("a")), &ud("\u{1161}")),
            Combine::Discard
        ));
        assert!(matches!(
            utf8_combine(Some(&ud("a")), &ud("\u{1100}")),
            Combine::Separate
        ));
        // After a joiner anything joins.
        assert!(matches!(
            utf8_combine(Some(&ud("a\u{200d}")), &ud("\u{1f600}")),
            Combine::Combined { .. }
        ));
        // Too long to join: kept apart, or dropped when zero width.
        let long = ud("\u{1f600}\u{200d}\u{1f600}\u{200d}\u{1f600}\u{301}");
        assert_eq!(long.size, 20);
        assert!(matches!(
            utf8_combine(Some(&long), &ud("\u{1f600}")),
            Combine::Separate
        ));
        assert!(matches!(
            utf8_combine(Some(&long), &acute),
            Combine::Discard
        ));
    }
}
