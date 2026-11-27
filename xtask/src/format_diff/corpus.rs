//! The corpus file and the list of what the checker leaves out.
//!
//! The corpus has one format per line. Lines are taken as they are, with
//! no trimming, so a format may start or end with spaces. These lines are
//! not formats:
//!
//! - an empty line
//! - a comment: a line that is exactly `#` or starts with `# `
//!
//! A line that starts with a backslash is a format without that first
//! backslash. So `\# x` is the format `# x`, `\\x` is `\x` and a lone `\`
//! is the empty format. Formats cannot contain a newline or a tab (tabs
//! separate the fields of the known differences file).

/// Variables whose value differs between two servers on the same host
/// for reasons that have nothing to do with formats: time, pids, the
/// tty, the socket, the version. The host is left out too, as the brief
/// asks. Formats that use one are skipped.
pub const EXCLUDED_VARIABLES: &[&str] = &[
    "buffer_created",
    "client_activity",
    "client_created",
    "client_discarded",
    "client_name",
    "client_pid",
    "client_tty",
    "client_written",
    "hook_fire_time",
    "host",
    "host_short",
    "pane_command_duration",
    "pane_command_end_time",
    "pane_command_start_time",
    "pane_dead_time",
    "pane_last_output_time",
    "pane_last_prompt_time",
    "pane_pid",
    "pane_pipe_pid",
    "pane_tty",
    "pid",
    "session_activity",
    "session_created",
    "session_last_attached",
    "socket_path",
    "start_time",
    "version",
    "window_activity",
];

/// Pieces of text that make a format time or host dependent: the host
/// aliases, jobs (their output arrives later and depends on timing), the
/// `t` modifier forms relative to now, and `status-right`, whose default
/// holds strftime specifiers that `T:` would expand to the current time.
pub const EXCLUDED_TEXT: &[&str] = &["#H", "#h", "#(", "t/p", "t/r", "t/d", "status-right"];

/// Reads a corpus from its text.
pub fn parse(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            if line.is_empty() || line == "#" || line.starts_with("# ") {
                None
            } else if let Some(rest) = line.strip_prefix('\\') {
                Some(rest.to_string())
            } else {
                Some(line.to_string())
            }
        })
        .collect()
}

/// Returns why a format is left out, or None when it can be checked.
pub fn excluded(format: &str) -> Option<String> {
    for t in EXCLUDED_TEXT {
        if format.contains(t) {
            return Some(format!("contains {t}"));
        }
    }
    for word in words(format) {
        if EXCLUDED_VARIABLES.contains(&word) {
            return Some(format!("uses {word}"));
        }
    }
    None
}

/// Splits a format into runs of name characters (letters, digits and
/// `_`), so `pane_pid` is one word and does not match `pid`.
fn words(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|w| !w.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_blank_lines_and_escapes() {
        let text = "# a comment\n\n#\n#{session_name}\n##\n#S\n\\# not a comment\n\\\\x\n\\\n  spaced  \n#,\n";
        assert_eq!(
            parse(text),
            vec![
                "#{session_name}",
                "##",
                "#S",
                "# not a comment",
                "\\x",
                "",
                "  spaced  ",
                "#,",
            ]
        );
    }

    #[test]
    fn crlf_lines() {
        assert_eq!(parse("a\r\nb\r\n"), vec!["a", "b"]);
    }

    #[test]
    fn exclusion_matches_whole_names() {
        assert!(excluded("#{pid}").is_some());
        assert!(excluded("#{pane_pid}").is_some());
        assert!(excluded("#{t:session_created}").is_some());
        assert!(excluded("#{=5:host}").is_some());
        assert!(excluded("#H").is_some());
        assert!(excluded("#(echo hi)").is_some());
        assert!(excluded("#{t/r:@num}").is_some());
        assert!(excluded("#{pane_id}").is_none());
        assert!(excluded("#{pidx}").is_none());
        assert!(excluded("#{session_name}").is_none());
        assert!(excluded("#{T:status-left}").is_none());
    }
}
