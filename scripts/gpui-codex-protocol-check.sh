#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fixture_dir="$repo_root/crates/mitsuro-desktop-backend/fixtures"
codex_bin=${CODEX_BIN:-codex}

for command in "$codex_bin" jq sort diff; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "missing required command: $command" >&2
    exit 2
  fi
done

protocol_tmp=$(mktemp -d "${TMPDIR:-/tmp}/mitsuro-codex-protocol.XXXXXX")
cleanup() {
  case "$protocol_tmp" in
    "${TMPDIR:-/tmp}"/mitsuro-codex-protocol.*) rm -rf -- "$protocol_tmp" ;;
    *) echo "refusing to remove unexpected temporary path: $protocol_tmp" >&2 ;;
  esac
}
trap cleanup EXIT

mkdir -p "$protocol_tmp/stable" "$protocol_tmp/experimental"
"$codex_bin" app-server generate-json-schema --out "$protocol_tmp/stable"
"$codex_bin" app-server generate-json-schema --experimental --out "$protocol_tmp/experimental"

extract_methods() {
  local schema=$1
  local output=$2
  jq -r '
    .. | objects |
    select(.properties?.method?.enum?) |
    .properties.method.enum[]
  ' "$schema" | sort -u > "$output"
}

extract_methods \
  "$protocol_tmp/stable/ClientRequest.json" \
  "$protocol_tmp/stable-client-methods.txt"
extract_methods \
  "$protocol_tmp/experimental/ClientRequest.json" \
  "$protocol_tmp/client-methods.txt"
extract_methods \
  "$protocol_tmp/experimental/ServerNotification.json" \
  "$protocol_tmp/server-notifications.txt"
extract_methods \
  "$protocol_tmp/stable/ServerRequest.json" \
  "$protocol_tmp/stable-server-requests.txt"
extract_methods \
  "$protocol_tmp/experimental/ServerRequest.json" \
  "$protocol_tmp/server-requests.txt"

jq -r '
  .definitions.ThreadItem.oneOf[] |
  .properties.type.enum[0] // empty
' "$protocol_tmp/stable/v2/ThreadListResponse.json" \
  | sort -u > "$protocol_tmp/thread-item-types.txt"

"$codex_bin" --version > "$protocol_tmp/codex-protocol-version.txt"

fixtures=(
  codex-protocol-version.txt
  client-methods.txt
  stable-client-methods.txt
  server-notifications.txt
  server-requests.txt
  stable-server-requests.txt
  thread-item-types.txt
)

if [[ ${1:-} == "--update" ]]; then
  for fixture in "${fixtures[@]}"; do
    cp "$protocol_tmp/$fixture" "$fixture_dir/$fixture"
  done
  echo "updated Codex protocol inventories from $($codex_bin --version)"
  exit 0
fi

failed=0
for fixture in "${fixtures[@]}"; do
  if ! diff -u "$fixture_dir/$fixture" "$protocol_tmp/$fixture"; then
    failed=1
  fi
done

if (( failed != 0 )); then
  echo "Codex protocol drift detected; review it, then run $0 --update" >&2
  exit 1
fi

echo "Codex protocol inventories match $($codex_bin --version)"
