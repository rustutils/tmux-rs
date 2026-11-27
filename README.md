# tmux-rs

tmux-rs is a terminal multiplexer written in Rust. It lets you run several
terminal sessions inside one terminal window: you can split the window into
panes, open more windows, switch between them, and detach from a session and
attach to it again later, from the same or another terminal, while the
programs in it keep running. It is a port of [tmux] and aims to be a drop-in
replacement for it, with the same commands, options, key bindings,
configuration file and client-server protocol.

## History

tmux was written by Nicholas Marriott, who started it in 2007 as a BSD
licensed alternative to GNU Screen. Version 1.0 was released in 2009, and the
same year it was imported into the OpenBSD base system. It is still developed
by Nicholas Marriott together with many contributors, both in OpenBSD and on
[GitHub][tmux].

In January 2025 Collin Richards began porting tmux 3.5a to Rust in the
[richardscollin/tmux-rs] repository. The port translated the C sources file
by file and kept their structure, which meant a lot of unsafe code, but it
produced a working tmux, and it was announced in July 2025. Collin continued
the work with help from Pavel Roskin and other contributors until early 2026.

In April 2026 Patrick Elsen, who had contributed to the original port, forked
it to continue the work. Since then the project has moved to a workspace of
smaller crates, replaced libevent with [calloop], started converting the code
to safe and idiomatic Rust module by module, and is bringing the behaviour up
to tmux 3.8. The tests run tmux-rs against C tmux to check that both behave
the same and can talk to each other.

## Compatibility

tmux-rs aims to behave like [tmux] 3.8. It started as a port of tmux 3.5a
and much of what changed up to 3.8 has been ported, but some larger 3.8
features are still missing. It reads the same configuration files as tmux
(`~/.tmux.conf`, `~/.config/tmux/tmux.conf` and `/etc/tmux.conf`).

### What works

- The features of tmux 3.5a.
- The tmux 3.8 client-server protocol, in both directions: a tmux-rs client
  can talk to a tmux 3.8 server and a tmux 3.8 client to a tmux-rs server.
- The bug, crash and leak fixes from tmux 3.6 to 3.8 that apply to the
  existing code, and the 3.8 default key bindings that need no new
  features.
- From 3.8: sorting with `-O` and `-r` in the list commands and
  `switch-client`, the new events and hooks, `wait-for -E`, `set-hook -E`
  and hook monitors (`set-hook -B`, `show-hooks -B`), control mode
  subscriptions as 3.8 checks them, array options with named items
  (`command-alias[foo]`), `-F` on the show commands, and the 3.8 rules for
  paste buffer names.
- The 3.8 command prompt: `message-format`, the prompt cursor options,
  prompts in panes (`command-prompt -P`) and inside modes, mouse clicks in
  the prompt, fuzzy matching in formats, and `paste-buffer` escaping what
  it pastes.
- Formats as 3.8 expands them, with all of its modifiers, operators and
  loops. Some 3.8 format variables are still missing.

### Not ported yet

- Themes and the new default styles and colours.
- Floating panes and `new-pane`.
- `switch-mode` and the new status line formats.
- Pane scrollbars.
- The copy mode additions, and copy mode commands that take arguments
  after `--`.
- New flags on 28 commands.
- `display-panes` as a mode.
- A few new options, such as `focus-follows-mouse`, `get-clipboard` and
  `codepoint-widths`, and some changes to tree mode.

The tests compare the options, commands and key bindings with tmux 3.8, and
[tests/parity_known.rs](tests/parity_known.rs) lists every difference that
is left.

### Differences on purpose

- The binary is called `tmux-rs`, and its sockets are in
  `/tmp/tmux-rs-UID/` instead of `/tmp/tmux-UID/`, so it can be installed
  next to tmux. To use a tmux-rs client with a tmux server, or the other
  way around, give both the same socket with `-S`.
- A few edge cases where tmux 3.8 reads uninitialised memory or writes
  outside a buffer behave in a defined way in tmux-rs.
- `run-shell -d` with a target session that is killed during the delay:
  tmux 3.8 keeps the killed session around until the command has run, so
  the command still gets that session's environment. tmux-rs does not keep
  destroyed sessions, so the command gets the global environment, and
  `TMUX` does not name a session. Without `-d` there is no difference.

The integration tests run with tmux-rs on both sides and with C tmux as the
client, the server or both, see [TESTING.md](TESTING.md).

## Installation

tmux-rs is not on crates.io yet. The `tmux-rs` crate there is the original
port. Install it from this repository with Cargo (Rust 1.88 or newer):

```sh
cargo install --locked --git https://gitlab.com/rustutils/tmux-rs tmux-rs
```

Or run it with Nix:

```sh
nix run gitlab:rustutils/tmux-rs
```

tmux-rs does not link against ncurses. It reads the terminfo database
directly, which almost every Linux system and macOS has installed already
(on Debian and Ubuntu it is the `ncurses-base` package). Building needs no
libraries apart from the C library.

## Usage

Start it with `tmux-rs`. It takes the same command line arguments and
commands as tmux, see the [tmux manual page] for them.

```sh
# start a new session
tmux-rs

# attach to the last session again
tmux-rs attach

# list sessions
tmux-rs ls
```

[tmux]: https://github.com/tmux/tmux
[richardscollin/tmux-rs]: https://github.com/richardscollin/tmux-rs
[calloop]: https://crates.io/crates/calloop
[tmux manual page]: https://man.openbsd.org/tmux
