#!/usr/bin/env bash
# Creates the same release on the GitHub mirror (the same path as the
# GitLab project) with gh, uploading the files from target/package. Runs
# in the release-github job of .gitlab-ci.yml. gh reads GITHUB_TOKEN, a
# token with write access to the repository's contents. --print shows the
# command instead of running it.
#
# Dry run: CI_COMMIT_TAG=v0.2.0 CI_PROJECT_PATH=rustutils/tmux-rs \
#   scripts/release-github.sh --print
set -euo pipefail
cd "$(dirname "$0")/.."

tag="$CI_COMMIT_TAG"
notes_file=$(mktemp)
scripts/release-notes.sh "$tag" > "$notes_file"

cmd=(gh release create "$tag" --repo "$CI_PROJECT_PATH" --title "tmux-rs ${tag#v}"
  --notes-file "$notes_file" --verify-tag)
if [[ "$tag" == *-* ]]; then
  cmd+=(--prerelease)
fi
cmd+=(target/package/*)

if [ "${1:-}" = --print ]; then
  printf '%q ' "${cmd[@]}"
  echo
else
  "${cmd[@]}"
fi
