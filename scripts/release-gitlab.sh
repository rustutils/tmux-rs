#!/usr/bin/env bash
# Creates the GitLab release for a tag pipeline, with the notes from
# CHANGELOG.md and a link to every file the package job left in
# target/package. Runs in the release job of .gitlab-ci.yml, which has
# release-cli. --print shows the command instead of running it.
#
# Dry run: CI_COMMIT_TAG=v0.2.0 CI_PROJECT_URL=https://gitlab.com/rustutils/tmux-rs \
#   scripts/release-gitlab.sh --print
set -euo pipefail
cd "$(dirname "$0")/.."

tag="$CI_COMMIT_TAG"
notes=$(scripts/release-notes.sh "$tag")

# The links point at the package job's artifacts for this tag, which
# never expire.
cmd=(release-cli create --name "tmux-rs ${tag#v}" --tag-name "$tag" --description "$notes")
for path in target/package/*; do
  name=$(basename "$path")
  url="$CI_PROJECT_URL/-/jobs/artifacts/$tag/raw/$path?job=package"
  cmd+=(--assets-link "{\"name\":\"$name\",\"url\":\"$url\"}")
done

if [ "${1:-}" = --print ]; then
  printf '%q ' "${cmd[@]}"
  echo
else
  "${cmd[@]}"
fi
