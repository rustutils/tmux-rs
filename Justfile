# List available recipes
default:
  @just --list

# run the linters, then unit and integration tests; fails if any server panics occur
test: check
  scripts/test.sh

coverage:
  cargo llvm-cov --html

# build tarballs, .deb and .rpm packages of a static binary in target/package,
# for x86_64 and aarch64 unless targets are given (needs cargo-deb and
# cargo-generate-rpm)
package *targets:
  scripts/package.sh {{targets}}

# build the packages, then install them in a container of each distribution
# and start a server with them (needs podman)
package-test: package
  scripts/package-test.sh

# run code formatter
format:
  cargo +nightly fmt --all

# run static linters
check:
  cargo +nightly fmt --all --check
  RUSTDOCFLAGS="-Dwarnings" cargo doc --no-deps --workspace
  cargo clippy --workspace --all-targets -- -Dwarnings

# run tmux-rs under valgrind, in release mode (for manual testing)
valgrind:
  #!/usr/bin/env bash
  cargo build --release
  ID=$RANDOM
  valgrind --log-file=target/valgrind-$ID.txt ./target/release/tmux-rs
  cat target/valgrind-$ID.txt

# run fuzz targets that are known-clean (no crashes) for a short soak test
fuzz duration="10":
  scripts/fuzz.sh {{duration}}

careful:
  cargo +nightly careful test -- --test-threads=1

asan:
  ASAN_OPTIONS=detect_leaks=0 RUSTFLAGS="-Zsanitizer=address" cargo +nightly nextest run --target x86_64-unknown-linux-gnu

sanitizer:
  ASAN_OPTIONS=detect_leaks=0 RUSTFLAGS="-Zsanitizer=address" cargo +nightly test --target x86_64-unknown-linux-gnu --no-fail-fast -- --test-threads=1

# count unsafe code: totals and the 30 files with the most
progress:
  @cargo xtask metrics

# list functions with a safe signature that hide unsafe code
false-safe:
  @cargo xtask false-safe

# lower the per-file unsafe limits to the current counts (--allow-raise to raise)
unsafe-limits *args:
  @cargo xtask limits {{args}}

alias fmt := format
alias lint := check
