#!/usr/bin/env bash
# Runs the fuzz targets that are known to be clean (no crashes) for a short
# soak test. Needs nightly and cargo-fuzz.
#
# Usage: scripts/fuzz.sh [SECONDS]   (per target, default 10)
set -euo pipefail
cd "$(dirname "$0")/.."

if ! cargo +nightly fuzz --version >/dev/null 2>&1; then
  echo "cargo-fuzz is not installed (cargo install cargo-fuzz)" >&2
  exit 1
fi

duration="${1:-10}"
targets=(colour_find_rgb colour_fromstring style_parse key_string_lookup attributes utf8_decode regsub input_parse)

for target in "${targets[@]}"; do
  echo "fuzzing $target"
  if ! cargo +nightly fuzz run "$target" -- -max_total_time="$duration"; then
    echo "FAILED: $target"
    exit 1
  fi
  echo
done
echo "All ${#targets[@]} fuzz targets passed (${duration}s each)"
