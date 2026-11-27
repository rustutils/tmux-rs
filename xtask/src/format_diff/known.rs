//! The file of known differences.
//!
//! Each line is a kind, a key and a reason, separated by tabs:
//!
//! ```text
//! group<TAB>modifier R<TAB>3.8 modifier not ported yet
//! format<TAB>#{pane_flags}<TAB>3.8 variable not ported yet
//! ```
//!
//! A `group` entry covers every difference in that group (the names the
//! report prints), a `format` entry one format. Empty lines and lines
//! starting with `#` are ignored. The reason is for the reader and may be
//! left out. Known differences do not fail a run, new ones do, and
//! entries that matched nothing are reported so they can be removed.

/// What an entry matches.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    /// Every difference in a group.
    Group(String),
    /// One format.
    Format(String),
}

/// One known difference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// What it matches.
    pub key: Key,
    /// Why it is known, for example the roadmap item that removes it.
    pub reason: String,
}

/// Parses the file. Returns the line numbers and texts of lines it does
/// not understand as the error.
pub fn parse(text: &str) -> Result<Vec<Entry>, String> {
    let mut out = Vec::new();
    let mut bad = Vec::new();
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(3, '\t');
        let kind = fields.next().unwrap_or("");
        let key = fields.next();
        let reason = fields.next().unwrap_or("").to_string();
        match (kind, key) {
            ("group", Some(k)) => out.push(Entry {
                key: Key::Group(k.to_string()),
                reason,
            }),
            ("format", Some(k)) => out.push(Entry {
                key: Key::Format(k.to_string()),
                reason,
            }),
            _ => bad.push(format!("line {}: {line}", n + 1)),
        }
    }
    if bad.is_empty() {
        Ok(out)
    } else {
        Err(bad.join("\n"))
    }
}

/// Returns the index of the first entry that matches a difference.
pub fn find(entries: &[Entry], group: &str, format: &str) -> Option<usize> {
    entries.iter().position(|e| match &e.key {
        Key::Group(g) => g == group,
        Key::Format(f) => f == format,
    })
}

/// Renders entries in the file's form, one per line.
pub fn render(entries: &[Entry]) -> String {
    let mut s = String::new();
    for e in entries {
        let (kind, key) = match &e.key {
            Key::Group(g) => ("group", g),
            Key::Format(f) => ("format", f),
        };
        s.push_str(kind);
        s.push('\t');
        s.push_str(key);
        if !e.reason.is_empty() {
            s.push('\t');
            s.push_str(&e.reason);
        }
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_render() {
        let text = "# comment\n\ngroup\tmodifier R\tnot ported\nformat\t#{a,b}\n";
        let entries = parse(text).unwrap();
        assert_eq!(
            entries,
            vec![
                Entry {
                    key: Key::Group("modifier R".into()),
                    reason: "not ported".into()
                },
                Entry {
                    key: Key::Format("#{a,b}".into()),
                    reason: String::new()
                },
            ]
        );
        assert_eq!(parse(&render(&entries)).unwrap(), entries);
    }

    #[test]
    fn bad_lines() {
        assert!(parse("group\n").is_err());
        assert!(parse("other\tx\n").is_err());
    }

    #[test]
    fn matching() {
        let entries = parse("format\t#{x}\ngroup\tmodifier R\n").unwrap();
        assert_eq!(find(&entries, "modifier R", "#{R:a,1}"), Some(1));
        assert_eq!(find(&entries, "variable x", "#{x}"), Some(0));
        assert_eq!(find(&entries, "variable y", "#{y}"), None);
    }
}
