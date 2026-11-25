#!/usr/bin/env bash
# Installs the packages from target/package (built by scripts/package.sh)
# in a container of each distribution and starts a server with them.
# Needs podman.
set -euo pipefail
cd "$(dirname "$0")/.."

shopt -s nullglob
debs=(target/package/*.deb)
rpms=(target/package/*.rpm)
if [ ${#debs[@]} -eq 0 ] || [ ${#rpms[@]} -eq 0 ]; then
  echo "no packages in target/package, run scripts/package.sh first" >&2
  exit 1
fi

# Start a session, check that it is listed, and stop the server.
smoke='tmux-rs -V && tmux-rs -f/dev/null new -d -s smoke "sleep 100" &&
  tmux-rs ls | grep -q "^smoke:" && tmux-rs kill-server && echo ok'

# Runs the install command, then the smoke test, in a container of image.
run() {
  echo "== $1"
  podman run --rm -v "$PWD/target/package:/pkg:ro,z" -e TERM=xterm \
    -e DEBIAN_FRONTEND=noninteractive "$1" sh -ec "$2 && $smoke"
}

run debian:stable 'apt-get install -y -qq /pkg/*.deb >/dev/null'
run ubuntu:latest 'apt-get install -y -qq /pkg/*.deb >/dev/null'
# The packages are not signed.
run fedora:latest 'rpm -i --nosignature /pkg/*.rpm'
