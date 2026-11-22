<section class="warning">

> [!WARNING]
> This project is alpha quality and has many known bugs. It's written in
> almost entirely unsafe Rust. Don't use it yet unless you're willing to deal
> with frequent crashes.
>
> THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
> WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
> MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
> ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
> WHATSOEVER RESULTING FROM LOSS OF MIND, USE, DATA OR PROFITS, WHETHER
> IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
> OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.

</section>

# tmux-rs

A rust port of [tmux](https://github.com/tmux/tmux).

## Why?

Why not? This a fun hobby project for me. It's been my gardening for the past year.

Why not just use [zellij](https://zellij.dev/)? I like tmux. I want tmux,
not something else.

## Compatibility

tmux-rs aims to behave like tmux 3.8. It started as a port of tmux 3.5a and
is being brought up to 3.8. It already speaks the tmux 3.8 protocol, so a
tmux-rs client works with a tmux 3.8 server and the other way around. Most
commands, options and formats added after 3.5a are not available yet.

The integration tests run with tmux-rs on both sides and with C tmux as the
client, the server or both, see [TESTING.md](TESTING.md).

## Installation

### Linux

It requires a terminfo database (usually packaged with ncurses).

```sh
sudo apt-get install libncurses
cargo install tmux-rs
tmux-rs
```

### macOS

```sh
brew install ncurses
cargo install tmux-rs
tmux-rs
```
