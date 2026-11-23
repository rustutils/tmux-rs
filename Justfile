# List available recipes
default:
  @just --list

# run the linters, then unit and integration tests; fails if any server panics occur
test: check
  #!/usr/bin/env bash
  set -euo pipefail
  # Clear any stale server panic files before starting.
  rm -f server-panic-*.txt
  # run all (unit and integration) tests of every crate
  cargo nextest run --workspace
  # run integration tests against system tmux, found on PATH unless
  # SYSTEM_TMUX names it (it is not in /usr/bin on macOS)
  tmux="${SYSTEM_TMUX:-$(command -v tmux || true)}"
  if [ -n "$tmux" ]; then
    TMUX_SERVER_BIN="$tmux" TMUX_CLIENT_BIN="$tmux" cargo nextest run --tests
    TMUX_CLIENT_BIN="$tmux" cargo nextest run --tests
    TMUX_SERVER_BIN="$tmux" cargo nextest run --tests
  else
    echo "no system tmux found, skipping the tests against it"
  fi
  # Fail if any server panics occurred during the test run.
  shopt -s nullglob
  panics=(server-panic-*.txt)
  if [ ${#panics[@]} -gt 0 ]; then
    echo
    echo "FAIL: ${#panics[@]} server panic file(s) produced during tests:"
    for f in "${panics[@]}"; do
      echo "  $f:"
      head -2 "$f" | sed 's/^/    /'
    done
    exit 1
  fi

coverage:
  cargo llvm-cov --html

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
  #!/usr/bin/env bash
  set -euo pipefail
  targets=(colour_find_rgb colour_fromstring style_parse key_string_lookup attributes utf8_decode regsub input_parse)
  for target in "${targets[@]}"; do
    echo "fuzzing $target"
    if ! cargo +nightly fuzz run "$target" -- -max_total_time={{duration}}; then
      echo "FAILED: $target"
      exit 1
    fi
    echo
  done
  echo "All ${#targets[@]} fuzz targets passed ({{duration}}s each)"

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
