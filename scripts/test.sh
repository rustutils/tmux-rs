#!/usr/bin/env bash
# Runs the unit and integration tests of every crate, then the integration
# tests against the system tmux in the three mixed combinations. Fails if a
# server panicked during any of them. `just test` runs `just check` first.
#
# The system tmux must be 3.8. It is found on PATH unless SYSTEM_TMUX names
# it (it is not in /usr/bin on macOS).
set -euo pipefail
cd "$(dirname "$0")/.."

# Clear any stale server panic files before starting.
rm -f server-panic-*.txt

tmux="${SYSTEM_TMUX:-$(command -v tmux || true)}"
if [ "$("${tmux:-false}" -V 2>/dev/null)" != "tmux 3.8" ]; then
  echo "the tests need tmux 3.8 on PATH (or set SYSTEM_TMUX)" >&2
  exit 1
fi

# The tests compare with C tmux 3.8 (TMUX_REFERENCE_BIN), here the same
# tmux as below.
export TMUX_REFERENCE_BIN="${TMUX_REFERENCE_BIN:-$tmux}"

cargo nextest run --workspace

TMUX_SERVER_BIN="$tmux" TMUX_CLIENT_BIN="$tmux" cargo nextest run --tests
TMUX_CLIENT_BIN="$tmux" cargo nextest run --tests
TMUX_SERVER_BIN="$tmux" cargo nextest run --tests

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
