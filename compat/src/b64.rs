use core::mem::MaybeUninit;

// https://www.rfc-editor.org/rfc/rfc4648

pub unsafe fn b64_ntop(src: *const u8, srclength: usize, target: *mut u8, targsize: usize) -> i32 {
    let src = unsafe { std::slice::from_raw_parts(src, srclength) };
    let dst = unsafe { std::slice::from_raw_parts_mut(target.cast::<MaybeUninit<u8>>(), targsize) };

    // Like C, the length returned does not include the terminating NUL.
    match ntop(src, dst) {
        Ok(out) => out.len() as i32,
        Err(()) => -1,
    }
}

/// Decode base64 from the NUL-terminated `src` into `target`, which has
/// room for `targsize` bytes. Whitespace anywhere is skipped and `=`
/// padding is accepted. Returns the number of bytes stored, or -1 if the
/// input is not valid base64 or does not fit. Same behaviour as the BSD
/// `b64_pton` that C tmux uses. No terminating NUL is written.
///
/// # Safety
///
/// `src` must be NUL terminated and `target` valid for `targsize` bytes.
pub unsafe fn b64_pton(src: *const u8, target: *mut u8, targsize: usize) -> i32 {
    let srclength: usize = unsafe { libc::strlen(src.cast()) };
    let src = unsafe { std::slice::from_raw_parts(src.cast::<u8>(), srclength) };
    let dst = unsafe { std::slice::from_raw_parts_mut(target, targsize) };

    match pton(src, dst) {
        Ok(n) => n as i32,
        Err(()) => -1,
    }
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const PAD: u8 = b'=';

/// Value of a base64 character, or None if it is not one.
fn value(ch: u8) -> Option<u8> {
    ALPHABET.iter().position(|&a| a == ch).map(|v| v as u8)
}

/// C `isspace` in the C locale.
fn is_space(ch: u8) -> bool {
    matches!(ch, b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r')
}

/// Decode `src` into `dst`, a port of the BSD `b64_pton` state machine.
/// Returns the number of bytes written.
fn pton(src: &[u8], dst: &mut [u8]) -> Result<usize, ()> {
    let mut state = 0;
    let mut tarindex = 0;
    let mut it = src.iter().copied();
    let mut pad = false;

    while let Some(ch) = it.next() {
        if is_space(ch) {
            continue;
        }
        if ch == PAD {
            pad = true;
            break;
        }
        let v = value(ch).ok_or(())?;
        if tarindex >= dst.len() {
            return Err(());
        }
        match state {
            0 => {
                dst[tarindex] = v << 2;
                state = 1;
            }
            1 => {
                dst[tarindex] |= v >> 4;
                let nextbyte = (v & 0x0f) << 4;
                if tarindex + 1 < dst.len() {
                    dst[tarindex + 1] = nextbyte;
                } else if nextbyte != 0 {
                    return Err(());
                }
                tarindex += 1;
                state = 2;
            }
            2 => {
                dst[tarindex] |= v >> 2;
                let nextbyte = (v & 0x03) << 6;
                if tarindex + 1 < dst.len() {
                    dst[tarindex + 1] = nextbyte;
                } else if nextbyte != 0 {
                    return Err(());
                }
                tarindex += 1;
                state = 3;
            }
            _ => {
                dst[tarindex] |= v;
                tarindex += 1;
                state = 0;
            }
        }
    }

    if !pad {
        // Ended at the end of the input: no partial byte may be left.
        return if state == 0 { Ok(tarindex) } else { Err(()) };
    }

    // A pad character: one byte of information after two characters (needs
    // a second =), two bytes after three. Only whitespace may follow.
    match state {
        2 => {
            let next = it.by_ref().find(|&ch| !is_space(ch));
            if next != Some(PAD) {
                return Err(());
            }
        }
        3 => {}
        _ => return Err(()),
    }
    if it.any(|ch| !is_space(ch)) {
        return Err(());
    }
    // The bits past the last full byte must be zero.
    if tarindex < dst.len() && dst[tarindex] != 0 {
        return Err(());
    }
    Ok(tarindex)
}

/// encode
fn ntop<'out>(src: &'_ [u8], dst: &'out mut [MaybeUninit<u8>]) -> Result<&'out mut [u8], ()> {
    if dst.len() < src.len().div_ceil(3) * 4 + 1 {
        return Err(());
    }

    let mut i = 0;
    let mut it = src.chunks_exact(3);

    macro_rules! enc {
        ($e:expr) => {
            MaybeUninit::new(ALPHABET[($e & 0b00111111) as usize])
        }
    }

    for chunk in &mut it {
        dst[i] = enc!(chunk[0] >> 2);
        dst[i + 1] = enc!(chunk[0] << 4 | chunk[1] >> 4);
        dst[i + 2] = enc!(chunk[1] << 2 | chunk[2] >> 6);
        dst[i + 3] = enc!(chunk[2]);
        i += 4;
    }

    let chunk = it.remainder();
    match chunk.len() {
        0 => (),
        1 => {
            dst[i] = enc!(chunk[0] >> 2);
            dst[i + 1] = enc!(chunk[0] << 4);
            dst[i + 2] = MaybeUninit::new(b'=');
            dst[i + 3] = MaybeUninit::new(b'=');
            i += 4;
        }
        2 => {
            dst[i] = enc!(chunk[0] >> 2);
            dst[i + 1] = enc!(chunk[0] << 4 | chunk[1] >> 4);
            dst[i + 2] = enc!(chunk[1] << 2);
            dst[i + 3] = MaybeUninit::new(b'=');
            i += 4;
        }
        _ => unreachable!(),
    }

    dst[i] = MaybeUninit::new(b'\0');
    Ok(unsafe { std::slice::from_raw_parts_mut(dst.as_mut_ptr().cast::<u8>(), i) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode with a target of `size` bytes, None on error.
    fn decode(input: &std::ffi::CStr, size: usize) -> Option<Vec<u8>> {
        let mut out = vec![0u8; size];
        let n = unsafe { b64_pton(input.as_ptr().cast(), out.as_mut_ptr(), size) };
        (n >= 0).then(|| {
            out.truncate(n as usize);
            out
        })
    }

    #[test]
    fn pton_full_groups() {
        assert_eq!(decode(c"TWFu", 3).as_deref(), Some(&b"Man"[..]));
        assert_eq!(decode(c"aGVsbG8gd29ybGQh", 12).as_deref(), Some(&b"hello world!"[..]));
    }

    #[test]
    fn pton_accepts_digits_and_symbols() {
        // "\xfb\xff\xbf" encodes with + and /, "09" with digits.
        assert_eq!(decode(c"+/+/", 3).as_deref(), Some(&[0xfb, 0xff, 0xbf][..]));
        assert_eq!(decode(c"MDk=", 3).as_deref(), Some(&b"09"[..]));
    }

    #[test]
    fn pton_padding() {
        assert_eq!(decode(c"TWE=", 2).as_deref(), Some(&b"Ma"[..]));
        assert_eq!(decode(c"TQ==", 1).as_deref(), Some(&b"M"[..]));
        assert_eq!(decode(c"aGVsbG8=", 6).as_deref(), Some(&b"hello"[..]));
        assert_eq!(decode(c"TQ= =  ", 3).as_deref(), Some(&b"M"[..]));
    }

    #[test]
    fn pton_skips_whitespace() {
        assert_eq!(decode(c" TW\nFu\t", 3).as_deref(), Some(&b"Man"[..]));
    }

    #[test]
    fn pton_rejects_invalid() {
        assert_eq!(decode(c"****", 3), None);
        // Missing padding leaves a partial byte.
        assert_eq!(decode(c"aGVsbG", 6), None);
        // = in the first or second position of a group.
        assert_eq!(decode(c"=AAA", 3), None);
        assert_eq!(decode(c"T===", 3), None);
        // Single = after two characters needs a second one.
        assert_eq!(decode(c"TQ=", 3), None);
        // Data after the padding.
        assert_eq!(decode(c"TQ==TQ==", 6), None);
        // Non-zero bits after the last byte.
        assert_eq!(decode(c"TR==", 3), None);
    }

    #[test]
    fn pton_rejects_too_small_target() {
        assert_eq!(decode(c"TWFu", 2), None);
        assert_eq!(decode(c"aGVsbG8=", 4), None);
    }

    #[test]
    fn ntop_round_trip_and_length() {
        for data in [&b""[..], b"M", b"Ma", b"Man", b"hello world!"] {
            let size = data.len().div_ceil(3) * 4 + 1;
            let mut enc = vec![0u8; size];
            let n = unsafe { b64_ntop(data.as_ptr(), data.len(), enc.as_mut_ptr(), size) };
            assert_eq!(n as usize, size - 1, "length without the NUL for {data:?}");
            assert_eq!(enc[n as usize], 0);
            let mut dec = vec![0u8; data.len().max(1) + 2];
            let m = unsafe { b64_pton(enc.as_ptr(), dec.as_mut_ptr(), dec.len()) };
            assert_eq!(&dec[..m as usize], data);
        }
    }
}
