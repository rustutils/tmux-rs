# Changelog

## Unreleased

### Compatibility with tmux 3.8

- New panes and jobs get `COLORTERM=truecolor` with `TERM`, as in 3.8.
- The `set-environment` and `show-environment` usage says `variable`, as
  in 3.8.
- `bind-key` without a command only changes what it is given: `-N` sets
  the note and `-r` makes the key repeat, without clearing the other.

### Fixes

- A variable removed with `set-environment -u` or `-r` no longer reaches
  new panes and jobs when the server process itself had it. They now get
  exactly the environment tmux builds, as in tmux.
- A variable name that is not UTF-8 in the environment no longer crashes
  the client or the server, and `show-environment` prints names and
  values as they are instead of replacing bytes that are not UTF-8.
- A key table or `wait-for` channel name that is not UTF-8 no longer
  crashes the server.
- A client whose key table lost its last binding no longer reads freed
  memory. As in tmux, its next key goes to the root table.
- `bind-key` and `unbind-key` report an unknown key as `unknown key: X`,
  as tmux does.

## 0.2.0 (2026-11-26)

This release brings the command prompt up to tmux 3.8. Prompts are drawn
through the new `message-format` option and show the terminal cursor, which
four new options style. They can open inside a pane with
`command-prompt -P`, and the searches and prompts of the tree modes open
inside the mode. The mouse moves the cursor and picks completions, and the
editing keys behave as in 3.8. `paste-buffer` now escapes what it pastes,
as 3.8 does. Releases now come with .deb and .rpm packages and tarballs
for x86_64 and aarch64 Linux, with a static binary linked against musl.

### Compatibility with tmux 3.8

- The `message-format` option decides how prompts and messages are drawn,
  and `message-style` and `message-command-style` are expanded with the
  prompt formats (`prompt_input`, `prompt_flags`, `prompt_type` and
  others).
- The prompt shows the terminal cursor at the input instead of a reversed
  cell. The new options `prompt-cursor-style`, `prompt-cursor-colour`,
  `prompt-command-cursor-style` and `prompt-command-cursor-colour` set its
  shape and colour.
- `command-prompt -P` opens the prompt in a pane, over its last line, and
  fires the new `pane-prompt-opened` and `pane-prompt-closed` hooks. The
  copy mode bindings that prompt use it, as in 3.8.
- `command-prompt -C` keeps the panes updating while the prompt is open,
  `-e` closes the prompt on Backspace when it is empty, and `-l` takes the
  `-p` and `-I` values as one prompt.
- Searching and filtering in the tree, buffer, client and customize modes,
  and the prompts of tree and customize mode, open inside the mode.
- A left click in the prompt moves the cursor or picks a completion.
- Prompt editing as in 3.8: C-v inserts the next key as it is, keypad keys
  insert their text, Escape in vi mode moves the cursor left, cursor keys
  leave an incremental prompt, and Tab shows the possible completions
  inline after the text. Only command names are completed.
- There are two prompt types, `command` and `search`, so
  `command-prompt -T target` and `-T window-target` fail as in 3.8.
- Fuzzy matching in formats with the `z` and `p` flags of the `m`
  modifier.
- `paste-buffer` escapes control characters and bytes that are not UTF-8,
  so a paste cannot send escape sequences to a program. `-S` pastes the
  buffer as it is.
- Command output is no longer escaped.

### Fixes

- `show-buffer` printed garbage to control mode clients.
- Killing the pane that tree mode runs in, from the mode's own kill
  prompt, no longer crashes the server (tmux 3.8 crashes).

### Changes

- tmux-rs builds and passes its tests linked against musl.
- Releases on GitLab and GitHub with .deb and .rpm packages and tarballs
  for x86_64 and aarch64 Linux. The binary is static, so the packages
  install on any distribution release. They recommend ncurses-base for
  the terminfo database.

## 0.1.0 (2026-11-24)

The first release of this fork, which continues the original port by Collin
Richards. tmux-rs now aims for compatibility with tmux 3.8 instead of 3.5a.

### Compatibility with tmux 3.8

- The client-server protocol of tmux 3.8. A tmux-rs client can talk to a
  tmux 3.8 server and the other way around.
- The bug, crash and leak fixes from tmux 3.6 to 3.8 that apply to the code
  ported from 3.5a, and the 3.8 robustness checks: malformed client messages
  drop the client, and the server checks the peer's user ID.
- The 3.8 option defaults, and the 3.8 default key bindings that need no new
  features.
- Sorting with `-O` and `-r` in the list commands, `switch-client -O`, and
  next and previous session in sort order. `list-keys` uses a format, as in
  3.8.
- Events and hooks as in 3.8: the new events and their payloads, events
  delivered when they happen, `set-hook -E`, user hooks in `show-hooks`,
  and `wait-for -E` with `-F`, `-v`, `-l` and `-w`.
- Hook monitors: `set-hook -B`, `-T` and `-u -B`, and `show-hooks -B`.
- Control mode subscriptions (`refresh-client -B`) checked as in 3.8.
- Array options with named items, such as `command-alias[foo]`, and the
  3.8 error messages for options.
- `-F` on `show-options`, `show-window-options` and `show-hooks`, and the
  hook fire count and time.
- Lowering `history-limit` trims the history of existing panes.
- The 3.8 rules for paste buffer names, and renaming a buffer to its own
  name.
- The message log is numbered from 0, and `show-messages` works without a
  client.
- Server access lists with group entries.
- The default for `default-terminal` is chosen at build time from the
  terminfo entries that exist, as the configure script of tmux does.

### Fixes

- Many crashes, hangs, memory leaks and use after free bugs, both from
  upstream and in the port itself. Among them: `kill-pane -a` with many
  panes, `#{l:...}` in formats, customize mode freeing a name twice,
  `set-buffer -w` and `load-buffer` using freed data, and event callbacks
  using freed memory.
- Killing a window, or moving its last pane away, read the window after it
  was freed.
- The server could redraw a client whose terminal had just closed, using
  freed memory.
- Five pane hooks had misspelled names, so `set-hook pane-focus-in` failed.
- `set-option` and the paste buffer commands no longer change values that
  are not valid UTF-8, and the client no longer panics on such arguments.
- The copy mode position indicator was one column off.
- Output written by a pane just before it exits is no longer lost.
- A NUL byte from a pane no longer rings the bell.
- `refresh-client -f` had no effect.
- Client flags were never set from the command line.

### Changes

- The event loop is [calloop] instead of libevent.
- tmux-rs no longer links against ncurses. It reads the terminfo database
  with [terminfo-lean].
- Character widths come from [unicode-width] instead of utf8proc.

### Internals

- The code is split into a workspace of crates: tmux-types, tmux-utf8,
  tmux-grid, tmux-protocol, tmux-compat, tmux-regsub, tmux-log and
  tmux-event, with project tooling in xtask.
- All C trees and lists are replaced by Rust collections, sessions,
  windows, panes and clients live in registries keyed by ID, and layout
  cells in an arena.
- The grid, the input parser and the event API have safe interfaces.
  Paste buffers, the message log, the server access list, sorting, events,
  monitors and options are safe Rust modules without `unsafe`.
- Functions that need their callers to uphold something are marked
  `unsafe`, and a per-file limit on `unsafe` makes sure it only goes down.
- Over 1,100 tests. The integration tests run against tmux 3.8 as the
  client, the server or both, and compare options, commands and key
  bindings with it.

[calloop]: https://crates.io/crates/calloop
[terminfo-lean]: https://crates.io/crates/terminfo-lean
[unicode-width]: https://crates.io/crates/unicode-width

## 0.0.3

- Add sixel support
- Improved macOS installation process
- Change tmux-rs socket path to /tmp/tmux-rs-1000/
- Refactoring to use more Rust idioms and reduce unsafe code
- Fixed unnecessary cursor updates causing high CPU usage
- Fixed panic in debug build due to subtraction overflow in `grid_view_delete_cells`
- Fixed path display issues (due to incorrect strrchr implementation)
- Fixed mouse mode issues (selecting window by mouse, etc.)
- Fixed multiple bugs due to incorrect bitwise negation translation
- Fixed memory leak due to shadowing
- Fixed clear terminal on vim close (alternate screen check in `screen_write_alternateoff`)
- Fixed broken display-message command
- Fixed hex color parsing logic

## 0.0.2

- macOS and linux aarch64 supported added
- fixed numerous crashes
- fixed broken command line argument parsing
- Slightly improved panic log generation
- XDG\_CONFIG\_HOME now included in default config path
- added support to configure some variables at build time using environment variables:
  - TMUX\_VERSION
  - TMUX\_CONF
  - TMUX\_SOCK
  - TMUX\_TERM
  - TMUX\_LOCK\_CMD

## 0.0.1

initial release

