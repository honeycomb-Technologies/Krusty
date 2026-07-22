#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mode="${1:-deterministic}"

cd "$repo_root"

case "$mode" in
  deterministic)
    cargo test -p krusty-core --test scripted_core_scenarios
    python3 scripts/test_harness_e2e_loop.py
    python3 scripts/test_grok_core_behavior.py
    python3 scripts/test_context_saturation_e2e.py
    ;;
  grok-provider-smoke)
    KRUSTY_GROK_LIVE=1 \
      KRUSTY_GROK_LIVE_MODEL="${KRUSTY_GROK_LIVE_MODEL:-grok-4.5}" \
      cargo run -p krusty-core --example grok_live_smoke
    ;;
  grok-live-e2e)
    : "${KRUSTY_EVAL_ROOT:?set KRUSTY_EVAL_ROOT to a new disposable directory}"
    : "${KRUSTY_BASE_URL:?set KRUSTY_BASE_URL to the isolated loopback candidate URL}"
    python3 scripts/grok-core-behavior.py \
      --base-url "$KRUSTY_BASE_URL" \
      --root "$KRUSTY_EVAL_ROOT/core-behavior" \
      --model "${KRUSTY_GROK_LIVE_MODEL:-grok-4.5}" \
      --timeout "${KRUSTY_EVAL_TIMEOUT:-900}"
    python3 scripts/harness-e2e-loop.py \
      --base-url "$KRUSTY_BASE_URL" \
      --root "$KRUSTY_EVAL_ROOT" \
      --model "${KRUSTY_GROK_LIVE_MODEL:-grok-4.5}" \
      --cycles "${KRUSTY_EVAL_CYCLES:-3}" \
      --timeout "${KRUSTY_EVAL_TIMEOUT:-900}" \
      --external-retries "${KRUSTY_EVAL_EXTERNAL_RETRIES:-3}"
    ;;
  context-saturation-live)
    : "${KRUSTY_EVAL_ROOT:?set KRUSTY_EVAL_ROOT to a new disposable directory}"
    : "${KRUSTY_BASE_URL:?set KRUSTY_BASE_URL to the isolated loopback candidate URL}"
    python3 scripts/context-saturation-e2e.py \
      --base-url "$KRUSTY_BASE_URL" \
      --root "$KRUSTY_EVAL_ROOT" \
      --grok-model "${KRUSTY_GROK_LIVE_MODEL:-grok-4.5}" \
      --terra-model "${KRUSTY_TERRA_LIVE_MODEL:-gpt-5.6-terra}" \
      --batch-chars "${KRUSTY_SATURATION_BATCH_CHARS:-280000}" \
      --max-batches "${KRUSTY_SATURATION_MAX_BATCHES:-8}" \
      --timeout "${KRUSTY_EVAL_TIMEOUT:-1200}"
    ;;
  *)
    echo "usage: $0 {deterministic|grok-provider-smoke|grok-live-e2e|context-saturation-live}" >&2
    exit 2
    ;;
esac
