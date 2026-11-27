//! Byte string helpers the modifiers use: display width, trimming and
//! padding (format-draw.c and utf8.c), quoting, basename and dirname,
//! and C's number parsing and printing.

use tmux_compat::vis::{vis__, vis_flags};

use crate::Tm;
use crate::parse::skip;

/// What reading one UTF-8 character at a position found, as
/// utf8_open and utf8_append report it.
pub(crate) enum Char {
    /// A complete character of `size` bytes and `width` columns.
    Done { size: usize, width: u32 },
    /// A lead byte, then `have` bytes before it went wrong (a bad
    /// continuation byte, an invalid code point or the end).
    Bad { have: usize },
}

use tmux_utf8::char_width;

/// Reads a UTF-8 character at `i`, or None when `s[i]` is not a lead
/// byte (utf8_open fails). Like utf8_append, it takes as many bytes as
/// the lead byte says before it decides.
pub(crate) fn read_char(s: &[u8], i: usize) -> Option<Char> {
    let size = match s[i] {
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return None,
    };
    let mut have = 1;
    let mut bad = false;
    while have < size {
        if i + have >= s.len() {
            return Some(Char::Bad { have });
        }
        if s[i + have] & 0xc0 != 0x80 {
            bad = true;
        }
        have += 1;
    }
    if bad {
        return Some(Char::Bad { have });
    }
    match std::str::from_utf8(&s[i..i + size]) {
        Ok(text) => {
            let c = text.chars().next().unwrap_or(' ');
            Some(Char::Done {
                size,
                width: char_width(c),
            })
        }
        Err(_) => Some(Char::Bad { have }),
    }
}

/// Printable ASCII, as C's `*cp > 0x1f && *cp < 0x7f` on a signed char.
pub(crate) fn printable(b: u8) -> bool {
    (0x20..0x7f).contains(&b)
}

/// format_leading_hashes: counts the #s at `i` and their width. Returns
/// where to go on (the [ of a style is left for the caller), the count
/// and the width.
fn leading_hashes(s: &[u8], i: usize) -> (usize, usize, u32) {
    let mut n = 0;
    while i + n < s.len() && s[i + n] == b'#' {
        n += 1;
    }
    if s.get(i + n) != Some(&b'[') {
        let width = n.div_ceil(2) as u32;
        return (i + n, n, width);
    }
    let width = (n / 2) as u32;
    if n % 2 == 0 {
        return (i + n, n, width);
    }
    (i + n - 1, n, width)
}

/// format_width: the display width, without #[] styles.
pub fn format_width(s: &[u8]) -> u32 {
    let mut width = 0;
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'#' {
            let (end, _, lw) = leading_hashes(s, i);
            width += lw;
            i = end;
            if s.get(i) == Some(&b'#') {
                match skip(s, i + 2, b"]") {
                    Some(e) => i = e + 1,
                    None => return 0,
                }
            }
        } else if let Some(c) = read_char(s, i) {
            match c {
                Char::Done { size, width: w } => {
                    width += w;
                    i += size;
                }
                Char::Bad { have } => i += have,
            }
        } else if printable(s[i]) {
            width += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    width
}

/// format_trim_left: keeps the first `limit` columns, with styles.
pub fn trim_left(s: &[u8], limit: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut width = 0;
    let mut i = 0;
    while i < s.len() {
        if width >= limit {
            break;
        }
        if s[i] == b'#' {
            let (end, n, mut lw) = leading_hashes(s, i);
            lw = lw.min(limit - width);
            if lw != 0 {
                if n == 1 {
                    out.push(b'#');
                } else {
                    out.extend(std::iter::repeat_n(b'#', 2 * lw as usize));
                }
                width += lw;
            }
            i = end;
            if s.get(i) == Some(&b'#') {
                let Some(e) = skip(s, i + 2, b"]") else {
                    break;
                };
                out.extend_from_slice(&s[i..=e]);
                i = e + 1;
            }
        } else if let Some(c) = read_char(s, i) {
            match c {
                Char::Done { size, width: w } => {
                    if width + w <= limit {
                        out.extend_from_slice(&s[i..i + size]);
                    }
                    width += w;
                    i += size;
                }
                Char::Bad { .. } => i += 1,
            }
        } else if printable(s[i]) {
            if width < limit {
                out.push(s[i]);
            }
            width += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// format_trim_right: keeps the last `limit` columns, with styles.
pub fn trim_right(s: &[u8], limit: u32) -> Vec<u8> {
    let total = format_width(s);
    if total <= limit {
        return s.to_vec();
    }
    let skip_width = total - limit;
    let mut out = Vec::with_capacity(s.len());
    let mut width = 0;
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'#' {
            let (end, n, lw) = leading_hashes(s, i);
            let mut copy_width = lw;
            if width <= skip_width {
                if skip_width - width >= copy_width {
                    copy_width = 0;
                } else {
                    copy_width -= skip_width - width;
                }
            }
            if copy_width != 0 {
                if n == 1 {
                    out.push(b'#');
                } else {
                    out.extend(std::iter::repeat_n(b'#', 2 * copy_width as usize));
                }
            }
            width += lw;
            i = end;
            if s.get(i) == Some(&b'#') {
                let Some(e) = skip(s, i + 2, b"]") else {
                    break;
                };
                out.extend_from_slice(&s[i..=e]);
                i = e + 1;
            }
        } else if let Some(c) = read_char(s, i) {
            match c {
                Char::Done { size, width: w } => {
                    if width >= skip_width {
                        out.extend_from_slice(&s[i..i + size]);
                    }
                    width += w;
                    i += size;
                }
                Char::Bad { .. } => i += 1,
            }
        } else if printable(s[i]) {
            if width >= skip_width {
                out.push(s[i]);
            }
            width += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    out
}

/// utf8_cstrwidth: the width without looking at styles (used by `p`).
fn cstr_width(s: &[u8]) -> u32 {
    let mut width = 0;
    let mut i = 0;
    while i < s.len() {
        if let Some(Char::Done { size, width: w }) = read_char(s, i) {
            width += w;
            i += size;
            continue;
        }
        if s[i] > 0x1f && s[i] != 0x7f && s[i] < 0x80 {
            width += 1;
        }
        i += 1;
    }
    width
}

/// utf8_padcstr and utf8_rpadcstr: pads with spaces on the right
/// (`left` true, the value stays on the left) or on the left.
pub fn pad(s: &[u8], width: u32, left: bool) -> Vec<u8> {
    let n = cstr_width(s);
    if n >= width {
        return s.to_vec();
    }
    let spaces = std::iter::repeat_n(b' ', (width - n) as usize);
    let mut out = Vec::with_capacity(s.len() + (width - n) as usize);
    if left {
        out.extend_from_slice(s);
        out.extend(spaces);
    } else {
        out.extend(spaces);
        out.extend_from_slice(s);
    }
    out
}

/// format_quote_shell.
pub(crate) fn quote_shell(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for &c in s {
        if b"|&;<>(){}$`\\\"'*?[# =%\n\t".contains(&c) {
            out.push(b'\\');
        }
        out.push(c);
    }
    out
}

/// format_quote_shell_single.
pub(crate) fn quote_shell_single(s: &[u8]) -> Vec<u8> {
    let mut out = vec![b'\''];
    for &c in s {
        if c == b'\'' {
            out.extend_from_slice(b"'\\''");
        } else {
            out.push(c);
        }
    }
    out.push(b'\'');
    out
}

/// format_quote_style: doubles each #.
pub(crate) fn quote_style(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for &c in s {
        if c == b'#' {
            out.push(b'#');
        }
        out.push(c);
    }
    out
}

/// utf8_stravis: vis() on everything but complete UTF-8 characters.
fn stravis(s: &[u8], flag: vis_flags) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    let mut i = 0;
    while i < s.len() {
        if let Some(Char::Done { size, .. }) = read_char(s, i) {
            out.extend_from_slice(&s[i..i + size]);
            i += size;
            continue;
        }
        let next = s.get(i + 1).copied();
        if flag.contains(vis_flags::VIS_DQ) && s[i] == b'$' && next.is_some() {
            let n = next.unwrap_or(0);
            if n.is_ascii_alphabetic() || n == b'_' || n == b'{' {
                out.push(b'\\');
            }
            out.push(b'$');
        } else {
            vis__(&mut out, s[i] as i32, flag, next.unwrap_or(0) as i32);
        }
        i += 1;
    }
    out
}

/// args_escape: quotes a value so the command parser reads it back as
/// one argument (`q/a`).
pub(crate) fn args_escape(s: &[u8]) -> Vec<u8> {
    if s.is_empty() {
        return b"''".to_vec();
    }
    let quotes = if s.iter().any(|c| b" #';${}%".contains(c)) {
        b'"'
    } else if s.iter().any(|c| b" \"".contains(c)) {
        b'\''
    } else {
        0
    };
    if s[0] != b' ' && s.len() == 1 && (quotes != 0 || s[0] == b'~') {
        return vec![b'\\', s[0]];
    }
    let mut flag =
        vis_flags::VIS_OCTAL | vis_flags::VIS_CSTYLE | vis_flags::VIS_TAB | vis_flags::VIS_NL;
    if quotes == b'"' {
        flag |= vis_flags::VIS_DQ;
    }
    let escaped = stravis(s, flag);
    let tilde = escaped.first() == Some(&b'~');
    let mut out = Vec::with_capacity(escaped.len() + 4);
    match quotes {
        b'\'' => {
            out.push(b'\'');
            out.extend_from_slice(&escaped);
            out.push(b'\'');
        }
        b'"' => {
            out.push(b'"');
            if tilde {
                out.push(b'\\');
            }
            out.extend_from_slice(&escaped);
            out.push(b'"');
        }
        _ => {
            if tilde {
                out.push(b'\\');
            }
            out.extend_from_slice(&escaped);
        }
    }
    out
}

/// POSIX basename (the libgen.h one tmux uses).
pub(crate) fn basename(s: &[u8]) -> Vec<u8> {
    if s.is_empty() {
        return b".".to_vec();
    }
    let mut end = s.len();
    while end > 1 && s[end - 1] == b'/' {
        end -= 1;
    }
    let trimmed = &s[..end];
    if trimmed == b"/" {
        return b"/".to_vec();
    }
    match trimmed.iter().rposition(|&c| c == b'/') {
        Some(p) => trimmed[p + 1..].to_vec(),
        None => trimmed.to_vec(),
    }
}

/// dirname as glibc does it, which keeps exactly two leading slashes
/// ("//a" gives "//").
pub(crate) fn dirname(s: &[u8]) -> Vec<u8> {
    let Some(mut last) = s.iter().rposition(|&c| c == b'/') else {
        return b".".to_vec();
    };
    if last != 0 && last + 1 == s.len() {
        // A trailing slash: find the slash before the last name.
        let mut run = last;
        while run != 0 && s[run - 1] == b'/' {
            run -= 1;
        }
        if run != 0 {
            match s[..run].iter().rposition(|&c| c == b'/') {
                Some(p) => last = p,
                None => return b".".to_vec(),
            }
        }
    }
    let mut run = last;
    while run != 0 && s[run - 1] == b'/' {
        run -= 1;
    }
    let end = if run == 0 {
        if last == 1 { 2 } else { 1 }
    } else {
        run
    };
    s[..end].to_vec()
}

/// strtonum: a base 10 number between `min` and `max`, the whole string.
pub(crate) fn strtonum(s: &[u8], min: i64, max: i64) -> Option<i64> {
    strtonum_err(s, min, max).ok()
}

/// strtonum with its error string ("invalid", "too small" or "too
/// large"), which C logs.
pub(crate) fn strtonum_err(s: &[u8], min: i64, max: i64) -> Result<i64, &'static str> {
    match parse_ll(s) {
        None => Err("invalid"),
        Some(Err(true)) => Err("too small"),
        Some(Err(false)) => Err("too large"),
        Some(Ok(v)) if v < min => Err("too small"),
        Some(Ok(v)) if v > max => Err("too large"),
        Some(Ok(v)) => Ok(v),
    }
}

/// strtoll over the whole string: None when it is not a number, an
/// error with whether it was negative when it does not fit.
fn parse_ll(s: &[u8]) -> Option<Result<i64, bool>> {
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    let negative = match s.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let start = i;
    let mut value: Option<i64> = Some(0);
    while i < s.len() && s[i].is_ascii_digit() {
        let d = (s[i] - b'0') as i64;
        value = value.and_then(|v| {
            if negative {
                v.checked_mul(10)?.checked_sub(d)
            } else {
                v.checked_mul(10)?.checked_add(d)
            }
        });
        i += 1;
    }
    if i == start || i != s.len() {
        return None;
    }
    Some(value.ok_or(negative))
}

/// strtod with C's end pointer check (`*endch == '\0'`): Some only when
/// the whole string was read. It reads what glibc's strtod reads:
/// leading white space, a sign, decimal and hexadecimal numbers (with a
/// binary exponent), "inf", "infinity" and "nan" with an optional
/// "(chars)" in any case. An empty string reads as 0, because strtod
/// then leaves the end pointer on the terminator.
pub(crate) fn strtod(s: &[u8]) -> Option<f64> {
    if s.is_empty() {
        return Some(0.0);
    }
    let mut i = 0;
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\x0b' | b'\x0c' | b'\r') {
        i += 1;
    }
    let negative = s.get(i) == Some(&b'-');
    if matches!(s.get(i), Some(b'-' | b'+')) {
        i += 1;
    }
    let (value, end) = unsigned_strtod(s, i)?;
    if end != s.len() {
        return None;
    }
    Some(if negative { -value } else { value })
}

/// Whether `s` has `word` at `i`, ignoring case.
fn has_word(s: &[u8], i: usize, word: &[u8]) -> bool {
    s.len() >= i + word.len() && s[i..i + word.len()].eq_ignore_ascii_case(word)
}

/// The number strtod reads at `i` after the sign, and where it ends.
/// None when there is no number.
fn unsigned_strtod(s: &[u8], i: usize) -> Option<(f64, usize)> {
    if has_word(s, i, b"infinity") {
        return Some((f64::INFINITY, i + 8));
    }
    if has_word(s, i, b"inf") {
        return Some((f64::INFINITY, i + 3));
    }
    if has_word(s, i, b"nan") {
        let mut end = i + 3;
        if s.get(end) == Some(&b'(') {
            let mut j = end + 1;
            while j < s.len() && (s[j].is_ascii_alphanumeric() || s[j] == b'_') {
                j += 1;
            }
            if s.get(j) == Some(&b')') {
                end = j + 1;
            }
        }
        return Some((f64::NAN, end));
    }
    if s.get(i) == Some(&b'0')
        && matches!(s.get(i + 1), Some(b'x' | b'X'))
        && let Some(found) = hex_strtod(s, i + 2)
    {
        return Some(found);
    }

    // Decimal: digits with an optional point, at least one digit, then
    // an exponent only when it has digits.
    let mut j = i;
    let mut digits = 0;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
        digits += 1;
    }
    if s.get(j) == Some(&b'.') {
        j += 1;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if matches!(s.get(j), Some(b'e' | b'E')) {
        let mut k = j + 1;
        if matches!(s.get(k), Some(b'+' | b'-')) {
            k += 1;
        }
        if s.get(k).is_some_and(u8::is_ascii_digit) {
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            j = k;
        }
    }
    // Only ASCII digits, a point, an e and a sign were taken, which
    // Rust's parser reads the same way.
    let text = std::str::from_utf8(&s[i..j]).ok()?;
    Some((text.parse::<f64>().ok()?, j))
}

/// A hexadecimal number after its "0x": hex digits with an optional
/// point, at least one digit, then an optional binary exponent. None
/// when there is no digit, then strtod reads only the "0".
fn hex_strtod(s: &[u8], i: usize) -> Option<(f64, usize)> {
    let mut j = i;
    let mut mantissa: u64 = 0;
    // Binary exponent for digits that did not fit and for those after
    // the point.
    let mut exp: i64 = 0;
    let mut sticky = false;
    let mut digits = 0;
    let mut point = false;
    loop {
        match s.get(j) {
            Some(b'.') if !point => point = true,
            Some(&c) if c.is_ascii_hexdigit() => {
                let d = (c as char).to_digit(16).unwrap_or(0) as u64;
                digits += 1;
                if mantissa >> 56 == 0 {
                    mantissa = mantissa << 4 | d;
                    if point {
                        exp -= 4;
                    }
                } else {
                    sticky |= d != 0;
                    if !point {
                        exp += 4;
                    }
                }
            }
            _ => break,
        }
        j += 1;
    }
    if digits == 0 {
        return None;
    }
    if matches!(s.get(j), Some(b'p' | b'P')) {
        let mut k = j + 1;
        let negative = s.get(k) == Some(&b'-');
        if matches!(s.get(k), Some(b'+' | b'-')) {
            k += 1;
        }
        if s.get(k).is_some_and(u8::is_ascii_digit) {
            let mut e: i64 = 0;
            while k < s.len() && s[k].is_ascii_digit() {
                e = (e * 10 + (s[k] - b'0') as i64).min(1 << 20);
                k += 1;
            }
            exp += if negative { -e } else { e };
            j = k;
        }
    }
    // The low bit stands for any digits that were dropped, so rounding
    // to 53 bits stays right.
    let mantissa = if sticky { mantissa | 1 } else { mantissa };
    Some((scale2(mantissa as f64, exp), j))
}

/// `v` times two to the power `exp`, in steps that stay exact.
fn scale2(mut v: f64, mut exp: i64) -> f64 {
    while exp > 1000 {
        v *= 2f64.powi(1000);
        exp -= 1000;
    }
    while exp < -1000 {
        v *= 2f64.powi(-1000);
        exp += 1000;
    }
    v * 2f64.powi(exp as i32)
}

/// A double to long long and back, as x86_64 converts it: out of range
/// values and NaN become INT64_MIN (arm64 saturates instead, so C tmux's
/// output for `#{e|/|:1,0}` depends on the machine). The result is
/// never negative zero.
pub(crate) fn to_long_long(v: f64) -> f64 {
    let limit = -(i64::MIN as f64);
    if v.is_nan() || v >= limit || v < -limit {
        return i64::MIN as f64;
    }
    v as i64 as f64
}

/// A NaN with the given sign.
fn nan(negative: bool) -> f64 {
    if negative { -f64::NAN } else { f64::NAN }
}

/// The NaN x86_64 gives for `left op right` when one is NaN or the
/// operation is invalid: the left NaN, else the right one, else the
/// default NaN, which is negative. Rust on other machines may give
/// another NaN, so this picks it.
pub(crate) fn x86_nan(left: f64, right: f64, result: f64) -> f64 {
    if !result.is_nan() {
        return result;
    }
    if left.is_nan() {
        return nan(left.is_sign_negative());
    }
    if right.is_nan() {
        return nan(right.is_sign_negative());
    }
    nan(true)
}

/// glibc's fmod on x86_64, NaNs included: with two NaNs the result is
/// negative only when both are, with one it keeps that one's sign, and
/// an invalid operation gives the negative default NaN. Checked against
/// tmux 3.8 on x86_64.
pub(crate) fn fmod(left: f64, right: f64) -> f64 {
    match (left.is_nan(), right.is_nan()) {
        (true, true) => nan(left.is_sign_negative() && right.is_sign_negative()),
        (true, false) => nan(left.is_sign_negative()),
        (false, true) => nan(right.is_sign_negative()),
        (false, false) => {
            let r = left % right;
            if r.is_nan() { nan(true) } else { r }
        }
    }
}

/// printf("%.*f", prec, v). A negative precision means the default, 6.
pub(crate) fn print_float(prec: i64, v: f64) -> Vec<u8> {
    let prec = if prec < 0 { 6 } else { prec as usize };
    if v.is_nan() {
        return if v.is_sign_negative() {
            b"-nan".to_vec()
        } else {
            b"nan".to_vec()
        };
    }
    if v.is_infinite() {
        return if v < 0.0 {
            b"-inf".to_vec()
        } else {
            b"inf".to_vec()
        };
    }
    format!("{v:.prec$}").into_bytes()
}

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// ctime_r without its newline. ctime's names are always English.
pub(crate) fn ctime(tm: &Tm) -> Vec<u8> {
    let day = DAYS.get(tm.wday as usize).unwrap_or(&"???");
    let month = MONTHS.get(tm.mon as usize).unwrap_or(&"???");
    format!(
        "{day} {month} {:2} {:02}:{:02}:{:02} {}",
        tm.mday,
        tm.hour,
        tm.min,
        tm.sec,
        tm.year + 1900
    )
    .into_bytes()
}

/// format_relative_time: "1d2h", "3m4s" and so on, None in the future.
pub(crate) fn relative_time(now: i64, t: i64) -> Option<Vec<u8>> {
    if t > now {
        return None;
    }
    if t == now {
        return Some(b"0s".to_vec());
    }
    let age = now - t;
    let (d, h, m, s) = (
        age / 86400,
        (age % 86400) / 3600,
        (age % 3600) / 60,
        age % 60,
    );
    let out = if d != 0 {
        if h != 0 {
            format!("{d}d{h}h")
        } else {
            format!("{d}d")
        }
    } else if h != 0 {
        if m != 0 {
            format!("{h}h{m}m")
        } else {
            format!("{h}h")
        }
    } else if m != 0 {
        if s != 0 {
            format!("{m}m{s}s")
        } else {
            format!("{m}m")
        }
    } else {
        format!("{s}s")
    };
    Some(out.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths() {
        assert_eq!(basename(b"/usr/lib/"), b"lib");
        assert_eq!(basename(b"/"), b"/");
        assert_eq!(dirname(b"/usr/lib"), b"/usr");
        assert_eq!(dirname(b"/usr"), b"/");
        assert_eq!(dirname(b"usr"), b".");
        // glibc keeps exactly two leading slashes.
        assert_eq!(dirname(b"//a"), b"//");
        assert_eq!(dirname(b"//"), b"//");
        assert_eq!(dirname(b"///"), b"/");
        assert_eq!(dirname(b"///a"), b"/");
        assert_eq!(dirname(b"a//"), b".");
        assert_eq!(dirname(b"a/b//"), b"a");
        assert_eq!(dirname(b"a//b"), b"a");
        assert_eq!(dirname(b""), b".");
        assert_eq!(basename(b""), b".");
        assert_eq!(basename(b"//"), b"/");
        assert_eq!(basename(b"a/b//"), b"b");
    }

    #[test]
    fn strtod_as_glibc() {
        let ok = |s: &str| strtod(s.as_bytes());
        assert_eq!(ok(""), Some(0.0));
        assert_eq!(ok(" 5"), Some(5.0));
        assert_eq!(ok("\t-5"), Some(-5.0));
        assert_eq!(ok("5 "), None);
        assert_eq!(ok(" "), None);
        assert_eq!(ok("+"), None);
        assert_eq!(ok("."), None);
        assert_eq!(ok(".5"), Some(0.5));
        assert_eq!(ok("5."), Some(5.0));
        assert_eq!(ok("1e3"), Some(1000.0));
        assert_eq!(ok("1e"), None);
        assert_eq!(ok("1e+"), None);
        assert_eq!(ok("0x10"), Some(16.0));
        assert_eq!(ok("-0x10"), Some(-16.0));
        assert_eq!(ok("0x"), None);
        assert_eq!(ok("0x.8"), Some(0.5));
        assert_eq!(ok("0x1.8p1"), Some(3.0));
        assert_eq!(ok("0X1P-2"), Some(0.25));
        assert_eq!(ok("0x1p"), None);
        assert_eq!(ok("0x1p1024"), Some(f64::INFINITY));
        assert_eq!(ok("0x1p-1074"), Some(f64::from_bits(1)));
        assert_eq!(
            ok("0x123456789abcdef01"),
            Some(0x1_2345_6789_abcd_ef01u128 as f64)
        );
        assert_eq!(ok("INF"), Some(f64::INFINITY));
        assert_eq!(ok("-infinity"), Some(f64::NEG_INFINITY));
        assert_eq!(ok("infin"), None);
        assert!(ok("nan").is_some_and(|v| v.is_nan() && v.is_sign_positive()));
        assert!(ok("-nan").is_some_and(|v| v.is_nan() && v.is_sign_negative()));
        assert!(ok("NaN(abc_1)").is_some_and(f64::is_nan));
        assert!(ok("nan()").is_some_and(f64::is_nan));
        assert_eq!(ok("nan(-)"), None);
        assert_eq!(ok("1e400"), Some(f64::INFINITY));
        assert_eq!(ok("1e-400"), Some(0.0));
    }

    #[test]
    fn strtonum_errors() {
        assert_eq!(strtonum_err(b"5", 0, 10), Ok(5));
        assert_eq!(strtonum_err(b"x", 0, 10), Err("invalid"));
        assert_eq!(strtonum_err(b"", 0, 10), Err("invalid"));
        assert_eq!(strtonum_err(b"-1", 0, 10), Err("too small"));
        assert_eq!(strtonum_err(b"11", 0, 10), Err("too large"));
        assert_eq!(
            strtonum_err(b"99999999999999999999", 0, 10),
            Err("too large")
        );
        assert_eq!(
            strtonum_err(b"-99999999999999999999", 0, 10),
            Err("too small")
        );
    }

    #[test]
    fn nan_rules() {
        let n = |v: f64| (v.is_nan(), v.is_sign_negative());
        assert_eq!(n(x86_nan(0.0, 0.0, f64::NAN)), (true, true));
        assert_eq!(n(x86_nan(f64::NAN, -f64::NAN, f64::NAN)), (true, false));
        assert_eq!(n(x86_nan(1.0, -f64::NAN, f64::NAN)), (true, true));
        assert_eq!(n(fmod(-f64::NAN, f64::NAN)), (true, false));
        assert_eq!(n(fmod(-f64::NAN, -f64::NAN)), (true, true));
        assert_eq!(n(fmod(1.0, 0.0)), (true, true));
        assert_eq!(fmod(-5.0, 3.0), -2.0);
        assert_eq!(to_long_long(-0.5).to_bits(), 0f64.to_bits());
    }

    #[test]
    fn numbers() {
        assert_eq!(strtonum(b" 5", 0, 10), Some(5));
        assert_eq!(strtonum(b"5 ", 0, 10), None);
        assert_eq!(strtonum(b"-3", -10, 10), Some(-3));
        assert_eq!(strtod(b"1.5"), Some(1.5));
        assert_eq!(strtod(b"x"), None);
        assert_eq!(print_float(2, 3.5), b"3.50");
    }

    #[test]
    fn widths() {
        assert_eq!(format_width("#[fg=red]ab#[default]cd".as_bytes()), 4);
        assert_eq!(format_width("日本".as_bytes()), 4);
        assert_eq!(trim_left("日本語x".as_bytes(), 3), "日".as_bytes());
    }
}
