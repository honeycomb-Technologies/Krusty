#!/bin/sh
# Point a live Honey serve unit at the Atlas sidecar shipped in the linux
# archive. 0.9.23 already honors MITSURO_AGENT_BROWSER_PATH, so this does not
# require a new binary. Does not restart mitsuro-hive.socket.
#
# Usage: sh scripts/honey-atlas-repair.sh [v0.9.23]
set -eu

valid_release_tag() {
  candidate=$1
  case "$candidate" in
    ''|*[!0-9A-Za-z.+-]*) return 1 ;;
  esac
  printf '%s\n' "$candidate" | \
    grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'
}

sha256_file() {
  checksum_path=$1
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$checksum_path" | awk '{print tolower($1)}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$checksum_path" | awk '{print tolower($1)}'
  else
    echo "sha256sum or shasum is required to verify downloads." >&2
    return 1
  fi
}

published_sha256() {
  checksum_file=$1
  checksum_archive=$2
  awk -v expected_name="$checksum_archive" '
    NR == 1 {
      name = $2
      sub(/^\*/, "", name)
      if (length($1) == 64 && $1 !~ /[^0-9A-Fa-f]/ && name == expected_name) {
        digest = tolower($1)
        valid = 1
        next
      }
    }
    { valid = 0; exit 1 }
    END {
      if (NR != 1 || valid != 1) exit 1
      print digest
    }
  ' "$checksum_file"
}

verify_download() {
  verify_archive=$1
  verify_checksum=$2
  verify_name=$3

  if ! expected_sha256=$(published_sha256 "$verify_checksum" "$verify_name"); then
    echo "Release checksum must contain exactly one sha256sum record for $verify_name." >&2
    return 1
  fi
  if ! actual_sha256=$(sha256_file "$verify_archive"); then
    return 1
  fi
  if [ "$actual_sha256" != "$expected_sha256" ]; then
    echo "Checksum verification failed; the downloaded archive may be corrupted." >&2
    return 1
  fi
}

run_self_test() (
  set -eu
  test_root=$(mktemp -d)
  trap 'rm -rf "$test_root"' EXIT HUP INT TERM
  test_name=mitsuro-x86_64-unknown-linux-gnu.tar.gz
  test_archive="$test_root/$test_name"
  test_checksum="$test_archive.sha256"

  printf '%s\n' 'verified Atlas archive fixture' > "$test_archive"
  test_digest=$(sha256_file "$test_archive")
  printf '%s  %s\n' "$test_digest" "$test_name" > "$test_checksum"
  verify_download "$test_archive" "$test_checksum" "$test_name"
  printf '%s *%s\n' "$test_digest" "$test_name" > "$test_checksum"
  verify_download "$test_archive" "$test_checksum" "$test_name"

  printf '%s\n' 'tampered' >> "$test_archive"
  if verify_download "$test_archive" "$test_checksum" "$test_name" >/dev/null 2>&1; then
    echo "Self-test accepted a tampered release archive." >&2
    exit 1
  fi
  printf '%s\n' 'verified Atlas archive fixture' > "$test_archive"

  printf '%s  %s\n' "$test_digest" wrong-name.tar.gz > "$test_checksum"
  if verify_download "$test_archive" "$test_checksum" "$test_name" >/dev/null 2>&1; then
    echo "Self-test accepted a checksum for the wrong archive." >&2
    exit 1
  fi
  {
    printf '%s  %s\n' "$test_digest" "$test_name"
    printf '%s  %s\n' "$test_digest" extra.tar.gz
  } > "$test_checksum"
  if verify_download "$test_archive" "$test_checksum" "$test_name" >/dev/null 2>&1; then
    echo "Self-test accepted multiple checksum records." >&2
    exit 1
  fi
  printf '%s  %s\n' not-a-digest "$test_name" > "$test_checksum"
  if verify_download "$test_archive" "$test_checksum" "$test_name" >/dev/null 2>&1; then
    echo "Self-test accepted a malformed checksum." >&2
    exit 1
  fi
  if verify_download "$test_archive" "$test_root/missing.sha256" "$test_name" >/dev/null 2>&1; then
    echo "Self-test accepted a missing checksum." >&2
    exit 1
  fi

  for valid_tag in v0.9.23 v1.2.3-rc.1 v1.2.3+build.4; do
    valid_release_tag "$valid_tag"
  done
  for invalid_tag in v0.9 v1.2.3/../../attacker v1.2.3_bad; do
    if valid_release_tag "$invalid_tag"; then
      echo "Self-test accepted invalid release tag: $invalid_tag" >&2
      exit 1
    fi
  done
  multiline_tag=$(printf 'v1.2.3\nattacker')
  if valid_release_tag "$multiline_tag"; then
    echo "Self-test accepted a multiline release tag." >&2
    exit 1
  fi

  echo "honey-atlas-repair.sh self-test passed"
)

if [ "${1:-}" = "--self-test" ]; then
  run_self_test
  exit 0
fi

repo="honeycomb-Technologies/Mitsuro"
tag=${1:-v0.9.23}
if ! valid_release_tag "$tag"; then
  echo "Tag must look like v0.9.23, got: $tag" >&2
  exit 2
fi

install_dir=${MITSURO_INSTALL_DIR:-"$HOME/.local/bin"}
current_sidecar="$install_dir/.mitsuro-current/agent-browser"
overlay_dir="$HOME/.local/lib/mitsuro"
overlay_sidecar="$overlay_dir/agent-browser"
dropin_dir="$HOME/.config/systemd/user/mitsuro-serve.service.d"
archive_name=mitsuro-x86_64-unknown-linux-gnu.tar.gz
archive="https://github.com/${repo}/releases/download/${tag}/${archive_name}"
checksum="$archive.sha256"

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
  curl -fsSL "$archive" -o "$work/$archive_name"
  if ! curl -fsSL "$checksum" -o "$work/$archive_name.sha256"; then
    echo "Release checksum is required but could not be downloaded." >&2
    exit 1
  fi
  verify_download "$work/$archive_name" "$work/$archive_name.sha256" "$archive_name"
  echo "Release checksum verified."
  tar -xzf "$work/$archive_name" -C "$work" ./agent-browser
  if [ ! -f "$work/agent-browser" ] || [ -L "$work/agent-browser" ]; then
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
