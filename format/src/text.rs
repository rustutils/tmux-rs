//! Byte string helpers the modifiers use: display width, trimming and
//! padding (format-draw.c and utf8.c), quoting, basename and dirname,
//! and C's number parsing and printing.

use tmux_compat::vis::{vis__, vis_flags};

use crate::Tm;
use crate::parse::skip;

/// What reading one UTF-8 character at a position found, as
/// utf8_open and utf8_append report it.
enum Char {
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
fn read_char(s: &[u8], i: usize) -> Option<Char> {
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
fn printable(b: u8) -> bool {
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

/// POSIX dirname.
pub(crate) fn dirname(s: &[u8]) -> Vec<u8> {
    let mut end = s.len();
    while end > 1 && s[end - 1] == b'/' {
        end -= 1;
    }
    let trimmed = &s[..end];
    let Some(mut p) = trimmed.iter().rposition(|&c| c == b'/') else {
        return b".".to_vec();
    };
    while p > 0 && trimmed[p - 1] == b'/' {
        p -= 1;
    }
    if p == 0 {
        return b"/".to_vec();
    }
    trimmed[..p].to_vec()
}

/// strtonum: a base 10 number between `min` and `max`, the whole string.
pub(crate) fn strtonum(s: &[u8], min: i64, max: i64) -> Option<i64> {
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
    let mut value: i64 = 0;
    while i < s.len() && s[i].is_ascii_digit() {
        let d = (s[i] - b'0') as i64;
        value = if negative {
            value.checked_mul(10)?.checked_sub(d)?
        } else {
            value.checked_mul(10)?.checked_add(d)?
        };
        i += 1;
    }
    if i == start || i != s.len() || value < min || value > max {
        return None;
    }
    Some(value)
}

/// strtod with C's end pointer check: Some only when the whole string
/// was read. An empty string reads as 0, as `*endch == '\0'` there.
/// Hexadecimal floats are not handled (a gap of the spike).
pub(crate) fn strtod(s: &[u8]) -> Option<f64> {
    if s.is_empty() {
        return Some(0.0);
    }
    let mut i = 0;
    while i < s.len() && s[i].is_ascii_whitespace() {
        i += 1;
    }
    let text = std::str::from_utf8(&s[i..]).ok()?;
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    let lower = body.to_ascii_lowercase();
    if matches!(lower.as_str(), "inf" | "infinity" | "nan") {
        let v = if lower == "nan" {
            f64::NAN
        } else {
            f64::INFINITY
        };
        return Some(if text.starts_with('-') { -v } else { v });
    }
    if !body
        .bytes()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c == b'.')
    {
        return None;
    }
    if !body
        .bytes()
        .all(|c| c.is_ascii_digit() || b".eE+-".contains(&c))
    {
        return None;
    }
    text.parse::<f64>().ok()
}

/// A double to long long as x86_64 converts it: out of range and NaN
/// become i64::MIN (arm64 saturates instead, so C tmux's output for
/// `#{e|/|:1,0}` depends on the machine).
pub(crate) fn to_long_long(v: f64) -> f64 {
    let limit = -(i64::MIN as f64);
    if v.is_nan() || v >= limit || v < -limit {
        return i64::MIN as f64;
    }
    v.trunc()
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

/// format_time_difference: "1d2h", "3m4s" and so on, None in the future.
pub(crate) fn time_difference(now: i64, t: i64) -> Option<Vec<u8>> {
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
