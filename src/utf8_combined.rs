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
use core::ffi::c_void;

use libc::memcmp;

use crate::{Utf8Data, wchar_t};

pub unsafe fn utf8_has_zwj(ud: *const Utf8Data) -> bool {
    unsafe {
        if (*ud).size < 3 {
            return false;
        }

        memcmp(
            &raw const (*ud).data[((*ud).size - 3) as usize] as *const c_void,
            b"\xe2\x80\x8d\x00" as *const u8 as *const c_void,
            3,
        ) == 0
    }
}

pub unsafe fn utf8_is_zwj(ud: *const Utf8Data) -> bool {
    unsafe {
        if (*ud).size != 3 {
            return false;
        }
        memcmp(
            &raw const (*ud).data as *const u8 as *const c_void,
            b"\xe2\x80\x8d\x00" as *const u8 as *const c_void,
            3,
        ) == 0
    }
}

/// Whether this is variation selector 16, U+FE0F, which asks for the
/// emoji presentation of the character before it.
pub unsafe fn utf8_is_vs(ud: *const Utf8Data) -> bool {
    // SAFETY: caller guarantees `ud` is valid for reads.
    unsafe { (*ud).initialized_slice() == "\u{fe0f}".as_bytes() }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ud(s: &str) -> Utf8Data {
        let mut ud = Utf8Data::new([0u8; 0], 0, 0, 0);
        ud.data[..s.len()].copy_from_slice(s.as_bytes());
        ud.size = s.len() as u8;
        ud.have = ud.size;
        ud
    }

    #[test]
    fn variation_selector_16() {
        assert!(unsafe { utf8_is_vs(&ud("\u{fe0f}")) });
        assert!(!unsafe { utf8_is_vs(&ud("\u{ffcf}")) });
        assert!(!unsafe { utf8_is_vs(&ud("\u{fe0e}")) });
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
        assert_eq!(hanguljamo_check_state(&ud("\u{115f}"), &ud("\u{1160}")), Composable);
        assert_eq!(hanguljamo_check_state(&ud("\u{a960}"), &ud("\u{d7b0}")), Composable);
        assert_eq!(hanguljamo_check_state(&ud("\u{d7c6}"), &ud("\u{d7cb}")), Composable);
        assert_eq!(hanguljamo_check_state(&ud("\u{11a7}"), &ud("\u{11ff}")), Composable);
    }

    #[test]
    fn should_combine_flags_and_skin_tones() {
        // Decoding uses mbtowc, which needs a UTF-8 locale as in tmux.
        unsafe {
            use crate::libc::{LC_CTYPE, setlocale};
            if setlocale(LC_CTYPE, c"C.UTF-8".as_ptr().cast()).is_null() {
                setlocale(LC_CTYPE, c"en_US.UTF-8".as_ptr().cast());
            }
        }
        let gb = ud("\u{1f1ec}\u{1f1e7}");
        assert!(utf8_should_combine(&ud("\u{1f1ec}"), &ud("\u{1f1e7}")));
        assert!(!utf8_should_combine(&gb, &ud("\u{1f1eb}")));
        // A skin tone modifier and an emoji that takes one, in either
        // order as screen_write_combine checks both.
        assert!(utf8_should_combine(&ud("\u{1f3fb}"), &ud("\u{1f44d}")));
        assert!(!utf8_should_combine(&ud("\u{1f3fb}"), &ud("\u{1f600}")));
        assert!(!utf8_should_combine(&ud("a"), &ud("b")));
    }

    #[test]
    fn hangul_filler() {
        assert!(utf8_is_hangul_filler(&ud("\u{3164}")));
        assert!(!utf8_is_hangul_filler(&ud("\u{3163}")));
        assert!(!utf8_is_hangul_filler(&ud("a")));
    }
}
