#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

if ! command -v codex >/dev/null 2>&1; then
  printf 'Codex binary is required for dual-backend acceptance.\n' >&2
  exit 1
fi

MITSURO_SERVER_URL="${MITSURO_SERVER_URL:-http://127.0.0.1:3000}"
export MITSURO_SERVER_URL
RUN_LIVE_ACCEPTANCE="${MITSURO_RUN_LIVE_ACCEPTANCE:-0}"
unset MITSURO_RUN_LIVE_ACCEPTANCE

printf 'Checking Mitsuro server at %s\n' "$MITSURO_SERVER_URL"
curl --fail --silent --show-error "$MITSURO_SERVER_URL/health" >/dev/null

printf 'Running desktop backend contracts\n'
cargo test -p mitsuro-desktop-backend

printf 'Running live Mitsuro read-only contract\n'
MITSURO_RUN_SERVER_IT=1 \
  cargo test -p mitsuro-desktop-backend live_server_read_only_contract -- --nocapture

printf 'Running real Codex stdio handshake and ephemeral thread contract\n'
MITSURO_RUN_APP_SERVER_IT=1 \
  cargo test -p mitsuro-desktop-backend real_app_server_initialize_and_thread_list -- --nocapture

printf 'Running GPUI desktop tests and native-browser compile check\n'
cargo test -p mitsuro-gpui-desktop --no-default-features
cargo check -p mitsuro-gpui-desktop --features browser-native

if [[ "$RUN_LIVE_ACCEPTANCE" == "1" ]]; then
  printf 'Running strict live Mitsuro SSE turn acceptance\n'
  MITSURO_RUN_LIVE_ACCEPTANCE=1 \
    cargo test -p mitsuro-desktop-backend live_server_streaming_turn -- --nocapture

  printf 'Running strict live Codex stdio turn acceptance\n'
  MITSURO_RUN_LIVE_ACCEPTANCE=1 \
    cargo test -p mitsuro-desktop-backend real_app_server_streaming_turn -- --nocapture
else
  printf 'Live provider turns skipped; set MITSURO_RUN_LIVE_ACCEPTANCE=1 to require both.\n'
fi

printf 'Dual-backend acceptance passed.\n'
