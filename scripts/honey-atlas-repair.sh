#!/bin/sh
# Point a live Honey serve unit at the Atlas sidecar shipped in the linux
# archive. 0.9.23 already honors MITSURO_AGENT_BROWSER_PATH, so this does not
# require a new binary. Does not restart mitsuro-hive.socket.
#
# Usage: sh scripts/honey-atlas-repair.sh [v0.9.23]
set -eu

repo="honeycomb-Technologies/Mitsuro"
tag=${1:-v0.9.23}
if ! printf '%s' "$tag" | grep -Eq '^v[0-9]'; then
  echo "Tag must look like v0.9.23, got: $tag" >&2
  exit 2
fi

install_dir=${MITSURO_INSTALL_DIR:-"$HOME/.local/bin"}
current_sidecar="$install_dir/.mitsuro-current/agent-browser"
overlay_dir="$HOME/.local/lib/mitsuro"
overlay_sidecar="$overlay_dir/agent-browser"
dropin_dir="$HOME/.config/systemd/user/mitsuro-serve.service.d"
archive="https://github.com/${repo}/releases/download/${tag}/mitsuro-x86_64-unknown-linux-gnu.tar.gz"

if ! curl -fsSIL "$archive" >/dev/null 2>&1; then
  echo "${tag} has no GitHub Release linux archive yet." >&2
  exit 1
fi

sidecar=""
if [ -f "$current_sidecar" ] && [ ! -L "$current_sidecar" ]; then
  sidecar=$current_sidecar
elif [ -f "$current_sidecar" ]; then
  sidecar=$(readlink -f "$current_sidecar")
fi

if [ -z "$sidecar" ] || [ ! -f "$sidecar" ]; then
  echo "Release pointer is missing agent-browser. Extracting ${tag} sidecar."
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT HUP INT TERM
  curl -fsSL "$archive" -o "$work/mitsuro.tar.gz"
  tar -xzf "$work/mitsuro.tar.gz" -C "$work" ./agent-browser
  if [ ! -f "$work/agent-browser" ]; then
    echo "Archive ${tag} did not contain agent-browser." >&2
    exit 1
  fi
  mkdir -p "$overlay_dir"
  cp "$work/agent-browser" "$overlay_sidecar"
  chmod 0555 "$overlay_sidecar"
  sidecar=$overlay_sidecar
fi

mkdir -p "$dropin_dir"
cat > "$dropin_dir/atlas.conf" <<EOF
[Service]
Environment=MITSURO_AGENT_BROWSER_PATH=$sidecar
EOF

echo "Atlas sidecar: $sidecar"
"$sidecar" --version

if command -v systemctl >/dev/null 2>&1; then
  systemctl --user daemon-reload
  systemctl --user restart mitsuro-serve.service
fi

i=0
while [ "$i" -lt 10 ]; do
  if curl --fail --silent --show-error http://127.0.0.1:3000/health; then
    echo
    break
  fi
  i=$((i + 1))
  if [ "$i" -eq 10 ]; then
    echo " /health failed after Atlas repair. Check journalctl --user -u mitsuro-serve.service" >&2
    exit 1
  fi
  sleep 2
done

echo "Browser uses this sidecar only after serve restarted with MITSURO_AGENT_BROWSER_PATH."
echo "If tabs still fail after this 503 is gone, install Chromium with: $sidecar install"
echo "Do not restart mitsuro-hive.socket by itself."
