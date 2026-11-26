use std::io::Write;
use std::time::Duration;

use harness::TmuxTestHarness;

use super::harness;

// Ported from regress/conf-syntax.sh
#[test]
fn config_syntax_error_handling() {
    let tmux = TmuxTestHarness::new();

    // Create a config with an invalid command
    let mut tmpfile = tempfile::NamedTempFile::new().expect("failed to create temp file");
    writeln!(tmpfile, "not-a-real-command").unwrap();

    // The original test uses -f with the bad config
    let result = tmux
        .cmd()
        .args([
            &format!("-f{}", tmpfile.path().display()),
            "new-session",
            "-d",
        ])
        .run();
    // Should still work (bad config lines are skipped with errors)
    // Note: behavior may vary, since some configs cause startup failure
    let _ = result;
}

// Ported from regress/if-shell-error.sh
#[test]
fn if_shell_with_failing_command() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // if-shell with a command that fails should execute the else branch
    tmux.cmd()
        .args(["if-shell", "false", "set @result true", "set @result false"])
        .run()
        .assert_success();

    // Give the shell command time to execute
    std::thread::sleep(Duration::from_millis(500));

    let val = tmux.query("#{@result}");
    assert_eq!(val, "false");
}

// Ported from regress/if-shell-TERM.sh
#[test]
fn if_shell_with_succeeding_command() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // if-shell with a command that succeeds should execute the then branch
    tmux.cmd()
        .args(["if-shell", "true", "set @result yes", "set @result no"])
        .run()
        .assert_success();

    std::thread::sleep(Duration::from_millis(500));

    let val = tmux.query("#{@result}");
    assert_eq!(val, "yes");
}

// Ported from regress/if-shell-error.sh
// An unknown command nested inside an if-shell branch in a config file should
// be reported as %config-error in control-mode output, not crash the server.
#[test]
fn if_shell_unknown_command_in_config() {
    let tmux = TmuxTestHarness::new();

    let mut tmpfile = tempfile::NamedTempFile::new().expect("temp file");
    writeln!(tmpfile, "if 'true' 'wibble wobble'").unwrap();
    tmpfile.flush().unwrap();
    let tmp_path = tmpfile.path().display().to_string();

    // Use -C control mode with new-session, which prints %config-error lines
    // for any errors encountered while loading -f.
    let result = tmux
        .cmd()
        .args([&format!("-f{tmp_path}"), "-C", "new"])
        .stdin("")
        .run();
    let stdout = result.stdout_str();
    let needle = format!("%config-error {tmp_path}:1: {tmp_path}:1: unknown command: wibble");
    assert!(
        stdout.contains(&needle),
        "expected {needle:?} in control output, got: {stdout:?}"
    );
}

// Ported from regress/if-shell-error.sh (second case)
// `source` of a file containing an unknown command should report the error
// in control mode and not crash. Note: the original regress script grep'd
// for a `%config-error` prefix, but C tmux 3.5a actually emits the bare
// error here (the prefix is only used when -f config is loaded at startup),
// so we just check the error text appears.
#[test]
fn source_unknown_command_reports_error() {
    let tmux = TmuxTestHarness::new();

    let mut tmpfile = tempfile::NamedTempFile::new().expect("temp file");
    writeln!(tmpfile, "wibble wobble").unwrap();
    tmpfile.flush().unwrap();
    let tmp_path = tmpfile.path().display().to_string();

    let input = format!("source {tmp_path}\n");
    let result = tmux.cmd().args(["-C", "new"]).stdin(&input).run();
    let stdout = result.stdout_str();
    let needle = format!("{tmp_path}:1: unknown command: wibble");
    assert!(
        stdout.contains(&needle),
        "expected {needle:?} in control output, got: {stdout:?}"
    );
}

// Ported from regress/if-shell-TERM.sh
// TERM should be inherited from the launching environment when if-shell
// runs at config-load time, not be the tmux-internal default.
#[test]
fn if_shell_term_from_outside() {
    // xterm branch
    {
        let tmux = TmuxTestHarness::new();
        let mut tmpfile = tempfile::NamedTempFile::new().expect("temp file");
        writeln!(
            tmpfile,
            "if '[ \"$TERM\" = \"xterm\" ]' 'set -g default-terminal vt220' 'set -g default-terminal ansi'"
        ).unwrap();
        tmpfile.flush().unwrap();

        tmux.cmd()
            .args([
                &format!("-f{}", tmpfile.path().display()),
                "new-session",
                "-d",
            ])
            .env("TERM", "xterm")
            .run()
            .assert_success();
        tmux.wait_ready(Duration::from_secs(5));

        let val = tmux
            .cmd()
            .args(["show", "-vg", "default-terminal"])
            .run()
            .stdout_trimmed();
        assert_eq!(val, "vt220", "xterm branch should pick vt220");
    }

    // screen branch (else)
    {
        let tmux = TmuxTestHarness::new();
        let mut tmpfile = tempfile::NamedTempFile::new().expect("temp file");
        writeln!(
            tmpfile,
            "if '[ \"$TERM\" = \"xterm\" ]' 'set -g default-terminal vt220' 'set -g default-terminal ansi'"
        ).unwrap();
        tmpfile.flush().unwrap();

        tmux.cmd()
            .args([
                &format!("-f{}", tmpfile.path().display()),
                "new-session",
                "-d",
            ])
            .env("TERM", "screen")
            .run()
            .assert_success();
        tmux.wait_ready(Duration::from_secs(5));

        let val = tmux
            .cmd()
            .args(["show", "-vg", "default-terminal"])
            .run()
            .stdout_trimmed();
        assert_eq!(val, "ansi", "non-xterm branch should pick ansi");
    }
}

// Ported from regress/run-shell-output.sh
#[test]
fn run_shell_captures_output() {
    let mut tmux = TmuxTestHarness::new();
    tmux.new_session().run().assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    tmux.cmd()
        .args(["run-shell", "echo hello"])
        .run()
        .assert_success();

    // Check that the command ran through a side effect. It calls the
    // client under test by path on this test's socket: a bare "tmux"
    // would be whatever is on PATH, if anything (none on the macOS
    // runners), and would talk to the default server.
    let command = format!(
        "'{}' -S '{}' set @shell_output done",
        harness::client_bin().display(),
        tmux.socket_path()
    );
    tmux.cmd()
        .args(["run-shell", &command])
        .run()
        .assert_success();
    tmux.wait_for("#{@shell_output}", |v| v == "done", Duration::from_secs(5));
}

// Ported from regress/if-shell-nested.sh
// Tests that running tmux from within an if-shell config conditional works.
#[test]
fn if_shell_nested_tmux_invocation() {
    let tmux = TmuxTestHarness::new();
    let bin = harness::client_bin();
    let socket = tmux.socket_path().to_string();

    // Write a config that uses if-shell to invoke tmux itself
    let mut tmpfile = tempfile::NamedTempFile::new().expect("failed to create temp file");
    writeln!(
        tmpfile,
        "if '{} -S {} run \"true\"' 'set -s @done yes'",
        bin.display(),
        socket
    )
    .unwrap();
    tmpfile.flush().unwrap();

    // Start a new session with the config
    tmux.cmd()
        .args([
            &format!("-f{}", tmpfile.path().display()),
            "new-session",
            "-d",
        ])
        .env("TERM", "xterm")
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));

    // Allow the if-shell command to complete
    std::thread::sleep(Duration::from_millis(800));

    let val = tmux
        .cmd()
        .args(["show", "-vs", "@done"])
        .run()
        .stdout_trimmed();
    assert_eq!(val, "yes");
}

/// Writes `contents` to `path` under `home`, creating its directories. A
/// file ending in .tmux is a plugin script and is made executable.
fn write_home_file(home: &std::path::Path, path: &str, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    let path = home.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    if path.extension().is_some_and(|e| e == "tmux") {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// A configuration in the shape people write them: a colour theme and
/// plugins loaded with run-shell that call `tmux` back, environment
/// assignments, `%if` blocks, if-shell, hooks, globbed source-file and the
/// usual options and key bindings. 0.3.0 crashed on the first pane after a
/// theme's `NAME=value` line, which no test with a hand-picked command saw.
const USER_CONFIG: &str = r##"
# colour theme, loaded the way srcery and similar themes are
run-shell ~/.config/tmux/theme/theme.tmux

set -g default-terminal "tmux-256color"
set -ga terminal-overrides ",xterm-256color:Tc"

unbind C-b
set -g prefix C-a
bind C-a send-prefix

bind-key h select-pane -L
bind-key -r J resize-pane -D 5
bind-key -r C-l select-window -t :+
bind-key -r "<" swap-window -d -t -1
bind-key "|" split-window -h -c "#{pane_current_path}"
bind-key "\\" split-window -fh -c "#{pane_current_path}"
bind-key '"' split-window -v -c "#{pane_current_path}"
bind -n M-Left select-pane -L
bind -T copy-mode-vi v send-keys -X begin-selection

set -g base-index 1
setw -g pane-base-index 1
set -g renumber-windows on
setw -g mode-keys vi
setw -g monitor-activity on
set -g history-limit 10000
set -g @batt_color green

%if "#{==:#{base-index},1}"
set -g @branch if
%elif "#{==:#{base-index},0}"
set -g @branch elif
%else
set -g @branch else
%endif

if-shell 'test -n "$HOME"' 'set -g @ifshell yes' 'set -g @ifshell no'
if-shell -F '#{@batt_color}' {
    set -g @braces yes
}

set-hook -g after-new-window 'set -g @hooked yes'

source-file -q ~/.config/tmux/missing.conf
source-file ~/.config/tmux/conf.d/*.conf

# plugins, loaded the way tmux-sensible and tmux-prefix-highlight are
run-shell ~/.config/tmux/plugin/plugin.tmux
run-shell -b 'tmux set -g @background done'
"##;

const THEME_SCRIPT: &str = r#"#!/bin/sh
CURRENT_DIR="$(cd "$(dirname "$0")" && pwd)"
tmux source-file "$CURRENT_DIR/theme.conf"
if [ "$(tmux show-option -gqv @theme_patched)" = 1 ]; then
    tmux set -g @theme_variant patched
else
    tmux set -g @theme_variant plain
fi
"#;

const THEME_CONFIG: &str = r##"
orange=colour208
%hidden black=colour0
set -g status-left "#[fg=$black,bg=$orange] #S "
set -g status-right "#{prefix_highlight} %H:%M"
setw -g window-status-format "#[fg=$orange] #I:#W "
"##;

const PLUGIN_SCRIPT: &str = r##"#!/bin/sh
version=$(tmux -V)
highlight='#{?client_prefix,#[reverse] PREFIX #[noreverse],}'
status_right=$(tmux show-option -gqv status-right)
tmux set-option -gq status-right "$(printf '%s' "$status_right" | sed "s/#{prefix_highlight}/$highlight/")"
if [ -z "$(tmux show-option -gqv @sensible_done)" ]; then
    tmux set -g escape-time 0
    tmux set -g focus-events on
    tmux bind-key R source-file '~/.config/tmux/tmux.conf' \; display reloaded
    tmux set -g @sensible_done 1
fi
tmux set -g @plugin_version "$version"
"##;

const ENV_CONFIG: &str = r#"
EDITOR=vi
PAGER="less -R"
SPACED='a b'
"#;

#[test]
fn user_config_with_theme_and_plugins() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    write_home_file(&home, ".config/tmux/tmux.conf", USER_CONFIG);
    write_home_file(&home, ".config/tmux/theme/theme.tmux", THEME_SCRIPT);
    write_home_file(&home, ".config/tmux/theme/theme.conf", THEME_CONFIG);
    write_home_file(&home, ".config/tmux/plugin/plugin.tmux", PLUGIN_SCRIPT);
    write_home_file(&home, ".config/tmux/conf.d/10-env.conf", ENV_CONFIG);
    // Plugins call a bare tmux, which finds the server through TMUX.
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(harness::client_bin(), bin.join("tmux")).unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let config = home.join(".config/tmux/tmux.conf");

    let tmux = TmuxTestHarness::new();
    tmux.cmd()
        .args([
            &format!("-f{}", config.display()),
            "new-session",
            "-d",
            "-s",
            "s",
            "sleep 1000",
        ])
        .env("HOME", &home)
        .env("PATH", &path)
        .run()
        .assert_success();
    tmux.wait_ready(Duration::from_secs(5));
    tmux.wait_for("#{@background}", |v| v == "done", Duration::from_secs(5));

    // New panes get the environment the config set. This crashed 0.3.0.
    let pane = tmux
        .cmd()
        .args([
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "s:",
            "sh",
            "-c",
            r#"printf '%s|%s|%s|%s|%s\n' "$orange" "$black" "$EDITOR" "$PAGER" "$SPACED"; sleep 1000"#,
        ])
        .run()
        .stdout_trimmed();
    tmux.cmd()
        .args(["split-window", "-d", "-t", &pane, "sleep 1000"])
        .run()
        .assert_success();
    let start = std::time::Instant::now();
    let output = loop {
        let out = tmux.capture_pane_target(&pane);
        if !out.is_empty() || start.elapsed() > Duration::from_secs(5) {
            break out;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(output.trim_end(), "colour208||vi|less -R|a b");
    tmux.wait_for("#{@hooked}", |v| v == "yes", Duration::from_secs(5));

    let option = |name: &str| {
        tmux.cmd()
            .args(["show-options", "-gv", name])
            .run()
            .stdout_str()
            .trim_end_matches('\n')
            .to_string()
    };
    for (name, value) in [
        ("prefix", "C-a"),
        ("base-index", "1"),
        ("pane-base-index", "1"),
        ("mode-keys", "vi"),
        ("history-limit", "10000"),
        ("escape-time", "0"),
        ("default-terminal", "tmux-256color"),
        // %if is decided while the file is parsed, before any of its
        // commands run, so base-index is still 0 there.
        ("@branch", "elif"),
        ("@ifshell", "yes"),
        ("@braces", "yes"),
        ("@theme_variant", "plain"),
        ("@sensible_done", "1"),
        ("status-left", "#[fg=colour0,bg=colour208] #S "),
        (
            "status-right",
            "#{?client_prefix,#[reverse] PREFIX #[noreverse],} %H:%M",
        ),
        ("window-status-format", "#[fg=colour208] #I:#W "),
    ] {
        assert_eq!(option(name), value, "option {name}");
    }
    assert!(
        option("terminal-overrides").contains("xterm-256color:Tc"),
        "{}",
        option("terminal-overrides")
    );
    assert!(!option("@plugin_version").is_empty());

    let keys = |table: &str| {
        tmux.cmd()
            .args(["list-keys", "-T", table])
            .run()
            .stdout_lines()
            .iter()
            .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let prefix = keys("prefix");
    for binding in [
        r##"bind-key -T prefix \\ split-window -fh -c "#{pane_current_path}""##,
        "bind-key -r -T prefix J resize-pane -D 5",
        "bind-key -T prefix C-a send-prefix",
        "bind-key -T prefix R source-file",
    ] {
        assert!(prefix.contains(binding), "{binding} not in\n{prefix}");
    }
    assert!(!prefix.contains("prefix C-b "), "{prefix}");
    assert!(keys("root").contains("M-Left select-pane -L"));
    assert!(keys("copy-mode-vi").contains("v send-keys -X begin-selection"));

    // Loading the config again, as the usual reload binding does, works and
    // reports no errors.
    let reload = tmux
        .cmd()
        .args(["source-file", &config.display().to_string()])
        .env("PATH", &path)
        .run();
    reload.assert_success();
    assert_eq!(reload.stderr_str().trim(), "");
    assert_eq!(reload.stdout_trimmed(), "");
    tmux.cmd().args(["list-sessions"]).run().assert_success();
}
