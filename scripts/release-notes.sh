#!/usr/bin/env bash
# Prints the release notes for a tag: the CHANGELOG.md section headed
# "## VERSION" (with or without a date after it). Fails if the tag does
# not match the version in Cargo.toml, or if the section is missing. A
# prerelease tag (v0.2.0-rc.1) falls back to the Unreleased section.
#
# Usage: scripts/release-notes.sh TAG
set -euo pipefail
cd "$(dirname "$0")/.."

tag="${1:?usage: scripts/release-notes.sh TAG}"
version="${tag#v}"

crate_version=$(sed -n '/^\[workspace\.package\]/,/^\[/s/^version *= *"\(.*\)"/\1/p' Cargo.toml)
if [ "$crate_version" != "$version" ]; then
  echo "tag $tag does not match the version $crate_version in Cargo.toml" >&2
  exit 1
fi

# The body of the section whose heading is "## $1" or "## $1 (...)".
section() {
  awk -v want="$1" '
    /^## / {
      if (found) exit
      heading = substr($0, 4)
      sub(/ \(.*\)$/, "", heading)
      found = (heading == want)
      next
    }
    found { print }
  ' CHANGELOG.md | sed -e '/./,$!d' | tac | sed -e '/./,$!d' | tac
}

notes=$(section "$version")
if [ -z "$notes" ] && [[ "$version" == *-* ]]; then
  notes=$(section Unreleased)
fi
if [ -z "$notes" ]; then
  echo "no CHANGELOG.md section for $version" >&2
  exit 1
fi
printf '%s\n' "$notes"
