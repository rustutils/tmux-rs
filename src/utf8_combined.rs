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

use crate::{Utf8Data, utf8_in_table, wchar_t};

static UTF8_MODIFIER_TABLE: [wchar_t; 31] = [
    0x1F1E6, 0x1F1E7, 0x1F1E8, 0x1F1E9, 0x1F1EA, 0x1F1EB, 0x1F1EC, 0x1F1ED, 0x1F1EE, 0x1F1EF,
    0x1F1F0, 0x1F1F1, 0x1F1F2, 0x1F1F3, 0x1F1F4, 0x1F1F5, 0x1F1F6, 0x1F1F7, 0x1F1F8, 0x1F1F9,
    0x1F1FA, 0x1F1FB, 0x1F1FC, 0x1F1FD, 0x1F1FE, 0x1F1FF, 0x1F3FB, 0x1F3FC, 0x1F3FD, 0x1F3FE,
    0x1F3FF,
];

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

pub unsafe fn utf8_is_modifier(ud: *const Utf8Data) -> bool {
    // SAFETY: caller guarantees `ud` is valid for reads.
    let ud_ref = unsafe { &*ud };
    match ud_ref.to_wchar() {
        Some(wc) => utf8_in_table(wc, &UTF8_MODIFIER_TABLE),
        None => false,
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
    fn hangul_filler() {
        assert!(utf8_is_hangul_filler(&ud("\u{3164}")));
        assert!(!utf8_is_hangul_filler(&ud("\u{3163}")));
        assert!(!utf8_is_hangul_filler(&ud("a")));
    }
}
