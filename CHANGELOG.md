# Changelog

## main

- Cross-platform: build now succeeds on macOS x86_64 (libevent backend
  shim) and Linux aarch64 (`c_char` and `wchar_t` signedness differ from
  x86_64 — hardcoded casts replaced with platform-correct inference).
- `input.rs` (terminal escape sequence parser) taken vertical to safe
  Rust:
  - 32 byte-in/screen-state-out unit tests added as a regression net.
  - `input_ctx` ownership cleanup: `state` → `&'static input_state`,
    `input_buf` → `Vec<u8>`, `since_ground` → owned `Evbuffer`,
    `param_list` → `Vec<InputParam>` (replacing a fixed array of C
    union + manual type tag).
  - All ~50 internal handlers and helpers converted from
    `unsafe fn(*mut input_ctx)` to `fn(&mut input_ctx)`. Function
    pointer dispatch is now a safe call.
- Unrelated test-isolation fixes: parallel test races on `static mut`
  buffers in `key_string_lookup_key` and `style_tostring` exposed by
  the new test load; both now serialized with a mutex.

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

