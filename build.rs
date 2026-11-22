use std::path::Path;

fn main() {
    println!("cargo::rerun-if-changed=src/cmd_parse.lalrpop");
    lalrpop::process_root().unwrap();

    default_term();
}

/// Choose the default for the default-terminal option, like the configure
/// script of C tmux: the best of screen, screen-256color, tmux and
/// tmux-256color that has a terminfo entry on the build machine. Setting
/// `TMUX_TERM` in the environment overrides it.
fn default_term() {
    println!("cargo::rerun-if-env-changed=TMUX_TERM");
    if std::env::var_os("TMUX_TERM").is_some() {
        return;
    }
    let mut term = "screen";
    for candidate in ["screen-256color", "tmux", "tmux-256color"] {
        if has_terminfo(candidate) {
            term = candidate;
        }
    }
    println!("cargo::rustc-env=TMUX_TERM={term}");
}

/// Whether a compiled terminfo entry for `name` exists in one of the usual
/// directories. Entries live under a directory named after their first
/// letter, or its hex code on macOS.
fn has_terminfo(name: &str) -> bool {
    let mut dirs: Vec<String> = Vec::new();
    for var in ["TERMINFO", "TERMINFO_DIRS"] {
        if let Ok(value) = std::env::var(var) {
            dirs.extend(value.split(':').filter(|d| !d.is_empty()).map(String::from));
        }
    }
    dirs.extend(
        [
            "/etc/terminfo",
            "/lib/terminfo",
            "/usr/share/terminfo",
            "/usr/lib/terminfo",
            "/usr/share/lib/terminfo",
            "/usr/local/share/terminfo",
            "/opt/homebrew/share/terminfo",
        ]
        .map(String::from),
    );
    let first = name.as_bytes()[0];
    dirs.iter().any(|dir| {
        let letter = Path::new(dir).join((first as char).to_string()).join(name);
        let hex = Path::new(dir).join(format!("{first:x}")).join(name);
        letter.exists() || hex.exists()
    })
}
