#!/usr/bin/env bash
# Builds the release files in target/package: for each target a tarball,
# a .deb and an .rpm with a static binary. Needs cargo-deb and
# cargo-generate-rpm. The package metadata is in Cargo.toml.
#
# Usage: scripts/package.sh [TARGET...]
#   (default x86_64-unknown-linux-musl aarch64-unknown-linux-musl)
set -euo pipefail
cd "$(dirname "$0")/.."

if [ $# -eq 0 ]; then
  set -- x86_64-unknown-linux-musl aarch64-unknown-linux-musl
fi

version=$(cargo pkgid --locked -p tmux-rs | sed 's/.*[#@]//')

# Packages must not depend on the terminfo entries of the build machine.
# tmux-256color is in ncurses-base on Debian, Ubuntu and Fedora.
export TMUX_TERM=tmux-256color
export CARGO_PROFILE_RELEASE_STRIP=true

rm -rf target/package
mkdir -p target/package

for target in "$@"; do
  rustup target add "$target"
  # The musl targets bring their own C runtime, so rust-lld links them
  # for any architecture without a cross toolchain.
  linker_var="CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_LINKER"
  env "$linker_var=rust-lld" cargo build --release --locked --target "$target" -p tmux-rs

  # A tarball that unpacks into ~/.local or /usr/local.
  arch="${target%%-*}"
  name="tmux-rs-$version-$arch-linux"
  stage="target/package/.stage/$name"
  mkdir -p "$stage/bin" "$stage/share/doc/tmux-rs"
  cp "target/$target/release/tmux-rs" "$stage/bin/"
  cp README.md CHANGELOG.md LICENSE "$stage/share/doc/tmux-rs/"
  tar -C target/package/.stage -czf "target/package/$name.tar.gz" "$name"

  # The binary is stripped already, and the host strip cannot read
  # another architecture.
  cargo deb --no-build --no-strip --target "$target" -p tmux-rs -o target/package/
  cargo generate-rpm --target "$target" -o target/package/
done

rm -rf target/package/.stage
(cd target/package && sha256sum -- * > SHA256SUMS)
ls -l target/package
