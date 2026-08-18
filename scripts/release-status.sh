#!/bin/sh
# Print the cut state: versioned, tagged, published. A cut is not released
# until the linux archive exists on the GitHub Release.
#
# Usage: sh scripts/release-status.sh [v0.9.23]
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
repo="honeycomb-Technologies/Mitsuro"
cli_toml="$root/crates/mitsuro-cli/Cargo.toml"
cargo_version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$cli_toml" | head -n 1)
tag=${1:-}
if [ -z "$tag" ]; then
  if [ -n "$cargo_version" ]; then
    tag="v${cargo_version}"
  else
    echo "Could not read cargo/mitsuro version" >&2
    exit 1
  fi
fi

echo "cargo/mitsuro: ${cargo_version:-unknown}"
echo "target tag:    $tag"

if git -C "$root" rev-parse --verify "refs/tags/${tag}" >/dev/null 2>&1 \
  || git -C "$root" ls-remote --exit-code origin "refs/tags/${tag}" >/dev/null 2>&1; then
  echo "git tag:       present"
else
  echo "git tag:       missing"
  echo "next: merge the Version (Sampo) PR, or wait for Sampo publish"
  exit 0
fi

archive="https://github.com/${repo}/releases/download/${tag}/mitsuro-x86_64-unknown-linux-gnu.tar.gz"
if curl -fsSIL "$archive" >/dev/null 2>&1; then
  echo "linux archive: present"
  echo "released:      yes"
  echo "honey:         sh scripts/honey-upgrade.sh $tag"
  exit 0
fi

echo "linux archive: missing"
echo "released:      no (tag without assets is not a release)"
echo "next:          Actions → Release binaries → Run workflow → tag $tag"
echo "               or merge the pipeline PR so main dispatches that workflow"
exit 0
