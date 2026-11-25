#!/usr/bin/env bash
# Builds .deb and .rpm packages of a static binary in target/package.
# Needs cargo-deb and cargo-generate-rpm. The package metadata is in
# Cargo.toml.
#
# Usage: scripts/package.sh [TARGET]   (default x86_64-unknown-linux-musl)
set -euo pipefail
cd "$(dirname "$0")/.."

target="${1:-x86_64-unknown-linux-musl}"

rm -rf target/package
mkdir -p target/package
CARGO_PROFILE_RELEASE_STRIP=true cargo build --release --locked --target "$target" -p tmux-rs
cargo deb --no-build --no-strip --target "$target" -p tmux-rs -o target/package/
cargo generate-rpm --target "$target" -o target/package/
ls -l target/package
