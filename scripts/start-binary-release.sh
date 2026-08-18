#!/bin/sh
# Start or no-op the binary Release workflow for a protected v* tag.
# Safe to re-run: exits 0 if the linux archive already exists or a build is
# already in flight.
#
# Usage: sh scripts/start-binary-release.sh v0.9.23
set -eu

repo=${GITHUB_REPOSITORY:-honeycomb-Technologies/Mitsuro}
tag=${1:-}

if [ -z "$tag" ]; then
  echo "Usage: sh scripts/start-binary-release.sh v0.9.23" >&2
  exit 2
fi
if ! printf '%s' "$tag" | grep -Eq '^v[0-9]'; then
  echo "Tag must look like v0.9.23, got: $tag" >&2
  exit 2
fi

archive="https://github.com/${repo}/releases/download/${tag}/mitsuro-x86_64-unknown-linux-gnu.tar.gz"
if curl -fsSIL "$archive" >/dev/null 2>&1; then
  echo "${tag} already has mitsuro-x86_64-unknown-linux-gnu.tar.gz"
  exit 0
fi

if [ -z "${GH_TOKEN:-}${GITHUB_TOKEN:-}" ] && ! gh auth status >/dev/null 2>&1; then
  echo "Need gh auth or GH_TOKEN to dispatch Release binaries for ${tag}" >&2
  exit 1
fi

in_flight=$(gh run list --repo "$repo" --workflow release.yml \
  --status in_progress --limit 10 --json databaseId --jq 'length')
queued=$(gh run list --repo "$repo" --workflow release.yml \
  --status queued --limit 10 --json databaseId --jq 'length')
if [ "$in_flight" != "0" ] || [ "$queued" != "0" ]; then
  echo "Release binaries already in flight for this repository; not dispatching again"
  exit 0
fi

echo "Dispatching Release binaries for ${tag}"
gh workflow run release.yml --repo "$repo" --ref main -f "tag=${tag}"
