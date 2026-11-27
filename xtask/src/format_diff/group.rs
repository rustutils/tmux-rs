//! Grouping differences by kind.
//!
//! A difference is put in a group named after the first construct of the
//! format: a modifier (`modifier R`), an operator (`operator ==`), a
//! conditional, a variable (`variable pane_flags`), a short alias or plain
//! text. Before that the checker reduces a differing format to its
//! smallest `#{...}` part that still differs (see [`subterms`]), so a
//! missing modifier deep inside a conditional is grouped under the
//! modifier and not under the conditional.

/// Single character modifiers without arguments in 3.8 (format.c).
const SINGLE: &[u8] = b"labdnwETSWPOVL!<>A";
/// Two character operators in 3.8.
const DOUBLE: &[&[u8]] = &[b"||", b"&&", b"!!", b"!=", b"==", b"<=", b">="];
/// Single character modifiers that may take arguments in 3.8.
const WITH_ARGS: &[u8] = b"ImCLNPSOVst=pReqWcA";

/// The end of a modifier, as `format_is_end` in format.c.
fn is_end(c: Option<&u8>) -> bool {
    matches!(c, Some(b';') | Some(b':'))
}

/// Returns the name of the group a format belongs to.
pub fn classify(format: &str) -> String {
    let b = format.as_bytes();
    let mut i = 0;
    let mut alias = None;
    while i < b.len() {
        if b[i] == b'#' {
            match b.get(i + 1) {
                Some(b'{') => return classify_braces(&b[i + 2..]),
                Some(b'#' | b',' | b'}') => i += 2,
                Some(c) if c.is_ascii_alphabetic() && alias.is_none() => {
                    alias = Some(*c as char);
                    i += 2;
                }
                _ => i += 1,
            }
        } else {
            i += 1;
        }
    }
    match alias {
        Some(c) => format!("alias #{c}"),
        None => "plain text and escapes".to_string(),
    }
}

/// Classifies what follows `#{`.
fn classify_braces(b: &[u8]) -> String {
    if b.first() == Some(&b'?') {
        return match top_level_args(&b[1..]) {
            0..=2 => "conditional without else",
            3 => "conditional",
            _ => "conditional with else-if",
        }
        .to_string();
    }
    if !b.contains(&b'}') {
        return "unclosed #{".to_string();
    }
    if let Some(m) = first_modifier(b) {
        return m;
    }
    let end = b.iter().position(|&c| c == b'}').unwrap_or(b.len());
    let name = String::from_utf8_lossy(&b[..end]);
    let name: String = name.chars().take(40).collect();
    format!("variable {name}")
}

/// Counts the comma separated arguments up to the `}` that closes the
/// current `#{`, skipping nested `#{...}` and the escapes. An unclosed
/// brace counts to the end.
fn top_level_args(b: &[u8]) -> usize {
    let mut depth = 0;
    let mut args = 1;
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1)) {
            (b'#', Some(b'{')) => {
                depth += 1;
                i += 1;
            }
            (b'#', Some(b'#' | b',' | b'}')) => i += 1,
            (b'}', _) if depth == 0 => break,
            (b'}', _) => depth -= 1,
            (b',', _) if depth == 0 => args += 1,
            _ => {}
        }
        i += 1;
    }
    args
}

/// Returns the variable a format like `#{=5;p3:name}` applies its
/// modifiers to, when that is a plain name and there is no nesting. The
/// checker compares `#{name}` on its own, so a missing variable is not
/// put in the group of the modifier around it.
pub fn plain_subject(format: &str) -> Option<&str> {
    let inner = format.strip_prefix("#{")?.strip_suffix('}')?;
    if inner.contains("#{") {
        return None;
    }
    let (_, name) = inner.rsplit_once(':')?;
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@-[]".contains(c);
    if !name.is_empty() && name.chars().all(plain) {
        Some(name)
    } else {
        None
    }
}

/// Returns the conditions of a conditional like `#{?name,a,b}` that are
/// plain names. 3.8 looks a condition up as a name, so a conditional on
/// a variable tmux-rs lacks differs only because of that variable. The
/// checker compares `#{name}` on its own and groups the difference with
/// the variable.
pub fn condition_subjects(format: &str) -> Vec<&str> {
    let Some(inner) = format.strip_prefix("#{?").and_then(|s| s.strip_suffix('}')) else {
        return Vec::new();
    };
    let b = inner.as_bytes();
    let mut args = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        match (b[i], b.get(i + 1)) {
            (b'#', Some(b'{')) => {
                depth += 1;
                i += 1;
            }
            (b'#', Some(b'#' | b',' | b'}')) => i += 1,
            (b'}', _) if depth > 0 => depth -= 1,
            (b',', _) if depth == 0 => {
                args.push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    args.push(&inner[start..]);
    let plain = |c: char| c.is_ascii_alphanumeric() || "_@-[]".contains(c);
    // Conditions are every other argument, except a last one on its own
    // (the else).
    let conditions = args.len() / 2 + usize::from(args.len() == 1);
    args.iter()
        .step_by(2)
        .take(conditions)
        .copied()
        .filter(|a| !a.is_empty() && a.chars().all(plain))
        .collect()
}

/// Returns the first modifier as 3.8 parses it, or None when the text
/// does not start with one followed somewhere by a colon.
fn first_modifier(b: &[u8]) -> Option<String> {
    let colon = b.iter().position(|&c| c == b':')?;
    let close = b.iter().position(|&c| c == b'}').unwrap_or(b.len());
    if colon > close {
        return None;
    }
    let c0 = *b.first()?;
    if SINGLE.contains(&c0) && is_end(b.get(1)) {
        return Some(name_of(&b[..1]));
    }
    if b.len() >= 2 && DOUBLE.contains(&&b[..2]) && is_end(b.get(2)) {
        return Some(name_of(&b[..2]));
    }
    if WITH_ARGS.contains(&c0) {
        return Some(name_of(&b[..1]));
    }
    None
}

/// Names a modifier, operators apart from the rest.
fn name_of(m: &[u8]) -> String {
    let m = String::from_utf8_lossy(m);
    match m.as_ref() {
        "||" | "&&" | "!!" | "!=" | "==" | "<=" | ">=" | "!" | "<" | ">" => {
            format!("operator {m}")
        }
        _ => format!("modifier {m}"),
    }
}

/// What [`refine`] adds to the group of a variable that 3.8 has and
/// tmux-rs does not.
pub const MISSING_VARIABLE: &str = " missing in tmux-rs";

/// Marks a variable that tmux-rs expands to nothing where 3.8 has a value
/// as missing (`variable pane_z missing in tmux-rs`), so the known list
/// names each variable that is not ported and why. Other groups are
/// returned as they are. `rs` and `c` are what the two sides printed.
pub fn refine(group: String, rs: &[u8], c: &[u8]) -> String {
    let empty = |b: &[u8]| b.is_empty() || b == b"\n";
    if group.starts_with("variable ") && empty(rs) && !empty(c) {
        group + MISSING_VARIABLE
    } else {
        group
    }
}

/// Returns the balanced `#{...}` parts of a format, shortest first and
/// without repeats, leaving out the format itself. `##`, `#,` and `#}`
/// are skipped as escapes, as format.c does when it looks for the end of
/// a `#{`.
pub fn subterms(format: &str) -> Vec<String> {
    let b = format.as_bytes();
    let mut stack = Vec::new();
    let mut found: Vec<&[u8]> = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'#' {
            match b.get(i + 1) {
                Some(b'{') => {
                    stack.push(i);
                    i += 2;
                    continue;
                }
                Some(b'#' | b',' | b'}') => {
                    i += 2;
                    continue;
                }
                _ => {}
            }
        } else if b[i] == b'}'
            && let Some(start) = stack.pop()
        {
            found.push(&b[start..=i]);
        }
        i += 1;
    }
    let mut out: Vec<String> = Vec::new();
    for f in found {
        if f.len() == b.len() {
            continue;
        }
        // Parts start at an ASCII '#' and end at an ASCII '}', so they
        // are on character boundaries.
        let s = String::from_utf8_lossy(f).into_owned();
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out.sort_by_key(|s| s.len());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions() {
        assert_eq!(condition_subjects("#{?pane_flags,a,b}"), ["pane_flags"]);
        assert_eq!(condition_subjects("#{?x,1,#{y},2,z}"), ["x"]);
        assert_eq!(condition_subjects("#{?x,1,y,2,z}"), ["x", "y"]);
        assert_eq!(condition_subjects("#{?x,1,y,2}"), ["x", "y"]);
        assert!(condition_subjects("#{=5:x}").is_empty());
    }

    #[test]
    fn classify_constructs() {
        assert_eq!(classify("#{R:a,3}"), "modifier R");
        assert_eq!(classify("x#{s/a/b/:@plain}"), "modifier s");
        assert_eq!(classify("#{=/5/...:@long}"), "modifier =");
        assert_eq!(classify("#{||:1,0}"), "operator ||");
        assert_eq!(classify("#{!:1}"), "operator !");
        assert_eq!(classify("#{<:a,b}"), "operator <");
        assert_eq!(classify("#{!!:x}"), "operator !!");
        assert_eq!(classify("#{?1,a,b}"), "conditional");
        assert_eq!(classify("#{?1,a}"), "conditional without else");
        assert_eq!(
            classify("#{?1,#{?2,x,y},b,c,d}"),
            "conditional with else-if"
        );
        assert_eq!(classify("#{?1,a#,b,c}"), "conditional");
        assert_eq!(classify("#{pane_flags}"), "variable pane_flags");
        assert_eq!(classify("#{l:#{x}}"), "modifier l");
        assert_eq!(classify("#{e|+|:1,2}"), "modifier e");
        assert_eq!(classify("##x#S"), "alias #S");
        assert_eq!(classify("a#,b"), "plain text and escapes");
        assert_eq!(classify("#{session_name"), "unclosed #{");
        assert_eq!(classify("#{W/nr:x}"), "modifier W");
        assert_eq!(classify("#{n;=3:x}"), "modifier n");
    }

    #[test]
    fn subject_of_modifiers() {
        assert_eq!(plain_subject("#{=5;p3:pane_flags}"), Some("pane_flags"));
        assert_eq!(plain_subject("#{b:@path}"), Some("@path"));
        assert_eq!(plain_subject("#{t/f/%H#:%M:@num}"), Some("@num"));
        assert_eq!(plain_subject("#{b:#{x}}"), None);
        assert_eq!(plain_subject("#{R:a,3}"), None);
        assert_eq!(plain_subject("#{pane_flags}"), None);
        assert_eq!(plain_subject("x#{b:y}"), None);
    }

    #[test]
    fn missing_variables_by_name() {
        let g = |s: &str| s.to_string();
        assert_eq!(
            refine(g("variable x"), b"\n", b"0\n"),
            "variable x missing in tmux-rs"
        );
        assert_eq!(refine(g("variable x"), b"1\n", b"0\n"), "variable x");
        assert_eq!(refine(g("variable x"), b"1\n", b"\n"), "variable x");
        assert_eq!(refine(g("modifier R"), b"\n", b"aaa\n"), "modifier R");
    }

    #[test]
    fn subterms_shortest_first() {
        assert_eq!(
            subterms("#{?#{==:#{a},b},#{R:x,3},y}"),
            vec!["#{a}", "#{R:x,3}", "#{==:#{a},b}"]
        );
    }

    #[test]
    fn subterms_skip_escapes_and_repeats() {
        assert_eq!(subterms("#{a}##{b}#}#{a}x"), vec!["#{a}"]);
        assert_eq!(subterms("#{a"), Vec::<String>::new());
        assert_eq!(subterms("#{a}"), Vec::<String>::new());
    }
}
