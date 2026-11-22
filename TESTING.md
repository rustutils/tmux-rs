# Testing

Because this is a codebase with a lot of unsafe code and raw pointer
juggling, thorough testing is neccessary and can help spot bugs early.

## Unit Tests

There are extensive unit tests that you can run. Keep in mind that if you use
`cargo test` to run them, you have to pass `--test-threads=1`.  This is because
`cargo test` will run multiple tests in threads, which means mutable global
data structures are shared. If you use `cargo-nextest`, you don't need to do
this, because it uses a process-per-test model instead.

    cargo test -- --test-threads=1
    cargo nextest run

You can run tests with `cargo-careful`, which adds some annotations to the Rust
standard library that will catch additional bugs at runtime:

    cargo +nightly careful test -- --test-threads=1

You can also run the unit tests using the LLVM sanitizers. These add additional
hooks that will catch invalid behaviour at runtime. This is how you run the
tests with address sanitizer enabled. This also disables the leak checks (some
memory leaks are expected at the moment, these will be addressed later)

    ASAN_OPTIONS=detect_leaks=0 RUSTFLAGS="-Zsanitizer=address" cargo +nightly test --target x86_64-unknown-linux-gnu

## Compatibility with C tmux

The integration tests can run against C tmux to check that tmux-rs behaves
the same and speaks the same protocol. The target is tmux 3.8. Point the
tests at a C tmux binary for the client, the server or both:

    TMUX_CLIENT_BIN=/usr/bin/tmux cargo nextest run --tests
    TMUX_SERVER_BIN=/usr/bin/tmux cargo nextest run --tests
    TMUX_SERVER_BIN=/usr/bin/tmux TMUX_CLIENT_BIN=/usr/bin/tmux cargo nextest run --tests

`just test` runs the unit tests and all of these combinations, and fails if
a tmux-rs server panicked during the run.

## Running the code

You can run the tool under Valgrind. This will catch some classes of
undefined behaviour.

    cargo build
    valgrind --log-file=target/valgrind-$RANDOM.txt ./target/debug/tmux-rs


