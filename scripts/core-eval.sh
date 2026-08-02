#!/usr/bin/env bash
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/identity-env.sh"

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-deterministic}"

cd "$repo_root"

case "$mode" in
  deterministic)
    cargo test -p mitsuro-core --test scripted_core_scenarios
    python3 scripts/test_harness_e2e_loop.py
    python3 scripts/test_grok_core_behavior.py
    python3 scripts/test_context_saturation_e2e.py
    ;;
  grok-provider-smoke)
    MITSURO_GROK_LIVE=1 \
      MITSURO_GROK_LIVE_MODEL="${MITSURO_GROK_LIVE_MODEL:-grok-4.5}" \
      cargo run -p mitsuro-core --example grok_live_smoke
    ;;
  grok-live-e2e)
    : "${MITSURO_EVAL_ROOT:?set MITSURO_EVAL_ROOT to a new disposable directory}"
    : "${MITSURO_BASE_URL:?set MITSURO_BASE_URL to the isolated loopback candidate URL}"
    python3 scripts/grok-core-behavior.py \
      --base-url "$MITSURO_BASE_URL" \
      --root "$MITSURO_EVAL_ROOT/core-behavior" \
      --model "${MITSURO_GROK_LIVE_MODEL:-grok-4.5}" \
      --timeout "${MITSURO_EVAL_TIMEOUT:-900}"
    python3 scripts/harness-e2e-loop.py \
      --base-url "$MITSURO_BASE_URL" \
      --root "$MITSURO_EVAL_ROOT" \
      --model "${MITSURO_GROK_LIVE_MODEL:-grok-4.5}" \
      --cycles "${MITSURO_EVAL_CYCLES:-3}" \
      --timeout "${MITSURO_EVAL_TIMEOUT:-900}" \
      --external-retries "${MITSURO_EVAL_EXTERNAL_RETRIES:-3}"
    ;;
  context-saturation-live)
    : "${MITSURO_EVAL_ROOT:?set MITSURO_EVAL_ROOT to a new disposable directory}"
    : "${MITSURO_BASE_URL:?set MITSURO_BASE_URL to the isolated loopback candidate URL}"
    python3 scripts/context-saturation-e2e.py \
      --base-url "$MITSURO_BASE_URL" \
      --root "$MITSURO_EVAL_ROOT" \
      --grok-model "${MITSURO_GROK_LIVE_MODEL:-grok-4.5}" \
      --terra-model "${MITSURO_TERRA_LIVE_MODEL:-gpt-5.6-terra}" \
      --batch-chars "${MITSURO_SATURATION_BATCH_CHARS:-280000}" \
      --max-batches "${MITSURO_SATURATION_MAX_BATCHES:-8}" \
      --timeout "${MITSURO_EVAL_TIMEOUT:-1200}"
    ;;
  *)
    echo "usage: $0 {deterministic|grok-provider-smoke|grok-live-e2e|context-saturation-live}" >&2
    exit 2
    ;;
esac
