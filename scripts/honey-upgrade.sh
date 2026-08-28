#!/bin/sh
# Upgrade a Honey host to a published GitHub Release.
# Refuses to run until the linux archive exists. install.sh owns service
# restart and activation health.
#
# Usage: sh scripts/honey-upgrade.sh v0.9.23
set -eu

valid_release_tag() {
  candidate=$1
  case "$candidate" in
    ''|*[!0-9A-Za-z.+-]*) return 1 ;;
  esac
  printf '%s\n' "$candidate" | \
    grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'
}

repo="honeycomb-Technologies/Mitsuro"
tag=${1:-}
if [ -z "$tag" ]; then
  echo "Usage: sh scripts/honey-upgrade.sh v0.9.23" >&2
  exit 2
fi
if ! valid_release_tag "$tag"; then
  echo "Tag must look like v0.9.23, got: $tag" >&2
  exit 2
fi

archive="https://github.com/${repo}/releases/download/${tag}/mitsuro-x86_64-unknown-linux-gnu.tar.gz"
if ! curl -fsSIL "$archive" >/dev/null 2>&1; then
  echo "${tag} has no GitHub Release linux archive yet." >&2
  echo "A tag is not enough. Wait for Actions → Release binaries, then retry." >&2
  echo "Status: sh scripts/release-status.sh" >&2
  exit 1
fi

echo "Installing Mitsuro ${tag} from the GitHub Release"
curl -fsSL "https://raw.githubusercontent.com/${repo}/main/install.sh" | VERSION="$tag" sh

echo "Checking /health"
i=0
while [ "$i" -lt 10 ]; do
  if curl --fail --silent --show-error http://127.0.0.1:3000/health; then
    echo
    break
  fi
  i=$((i + 1))
  if [ "$i" -eq 10 ]; then
    echo " /health failed after install. Check journalctl --user -u mitsuro-serve.service" >&2
    exit 1
  fi
  sleep 2
done
if [ -x "$HOME/.local/bin/.mitsuro-current/mitsuro" ]; then
  echo -n "Installed CLI: "
  "$HOME/.local/bin/.mitsuro-current/mitsuro" --version
  echo "Executable: $(readlink -f "$HOME/.local/bin/.mitsuro-current/mitsuro")"
fi
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ -f "$script_dir/honey-atlas-repair.sh" ]; then
  echo "Pointing serve at the Atlas sidecar"
  sh "$script_dir/honey-atlas-repair.sh" "$tag"
fi
echo "Honey ${tag} is live only if /health passed and the executable is this release."
