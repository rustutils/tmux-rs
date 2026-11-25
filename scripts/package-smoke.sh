#!/bin/sh
# Installs the package for this machine's distribution and architecture
# from target/package (or the directory given) and starts a server with
# it. Runs as root inside a container: scripts/package-test.sh runs it
# with podman, CI in a job per distribution.
#
# Usage: scripts/package-smoke.sh [DIR]
set -eu

dir=$(cd "${1:-target/package}" && pwd)
export TERM=xterm

if command -v apt-get >/dev/null; then
  arch=$(dpkg --print-architecture)
  DEBIAN_FRONTEND=noninteractive apt-get install -y -qq "$dir"/tmux-rs_*_"$arch".deb >/dev/null
else
  # The packages are not signed.
  rpm -i --nosignature "$dir"/tmux-rs-*."$(uname -m)".rpm
fi

tmux-rs -V
# Start a session and check that it is listed.
tmux-rs -f/dev/null new -d -s smoke "sleep 100"
tmux-rs ls | grep -q "^smoke:"
# The default terminal must have a terminfo entry here.
term=$(tmux-rs show -gv default-terminal)
tmux-rs kill-server
if ! find /usr/share/terminfo /lib/terminfo /etc/terminfo -name "$term" 2>/dev/null | grep -q .; then
  echo "no terminfo entry for $term" >&2
  exit 1
fi
echo "ok, default-terminal $term"
