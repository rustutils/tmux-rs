#!/usr/bin/env bash
# Runs the unit and integration tests of every crate, then the integration
# tests against the system tmux in the three mixed combinations. Fails if a
# server panicked during any of them. `just test` runs `just check` first.
#
# The system tmux is found on PATH unless SYSTEM_TMUX names it (it is not
# in /usr/bin on macOS).
set -euo pipefail
cd "$(dirname "$0")/.."

# Clear any stale server panic files before starting.
rm -f server-panic-*.txt

cargo nextest run --workspace

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
