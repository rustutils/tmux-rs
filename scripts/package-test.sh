#!/usr/bin/env bash
# Installs the packages from target/package (built by scripts/package.sh)
# in a container of each distribution and starts a server with them, with
# scripts/package-smoke.sh. Needs podman. Only the packages of this
# machine's architecture are tested.
set -euo pipefail
cd "$(dirname "$0")/.."

shopt -s nullglob
debs=(target/package/*.deb)
rpms=(target/package/*.rpm)
if [ ${#debs[@]} -eq 0 ] || [ ${#rpms[@]} -eq 0 ]; then
  echo "no packages in target/package, run scripts/package.sh first" >&2
  exit 1
fi

for image in debian:stable ubuntu:latest fedora:latest; do
  echo "== $image"
  podman run --rm -v "$PWD:/src:ro,z" -w /src "$image" sh scripts/package-smoke.sh
done
