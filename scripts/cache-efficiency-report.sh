#!/usr/bin/env bash
set -euo pipefail
source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/identity-env.sh"

DB_PATH="${1:-${MITSURO_DB_PATH:-$HOME/.mitsuro/mitsuro.db}}"
SESSION_ID="${2:-}"

if [[ ! -f "$DB_PATH" ]]; then
  echo "database not found: $DB_PATH" >&2
  exit 2
fi

if [[ -n "$SESSION_ID" && ! "$SESSION_ID" =~ ^[A-Za-z0-9_-]+$ ]]; then
  echo "session-id contains unsupported characters" >&2
  exit 2
fi

SESSION_PREDICATE=""
if [[ -n "$SESSION_ID" ]]; then
  SESSION_PREDICATE="AND session_id = '$SESSION_ID'"
fi

sqlite3 -header -column "$DB_PATH" <<SQL
WITH calls AS (
  SELECT
    session_id,
    run_id,
    call_kind,
    operation,
    json_extract(payload_json, '$.provider') AS provider,
    json_extract(payload_json, '$.model') AS model,
    COALESCE(json_extract(payload_json, '$.usage_available'), 0) AS usage_available,
    COALESCE(json_extract(payload_json, '$.input_tokens'), 0) AS input_tokens,
    COALESCE(json_extract(payload_json, '$.cache_creation_input_tokens'), 0) AS cache_write,
    COALESCE(json_extract(payload_json, '$.cache_read_input_tokens'), 0) AS cache_read,
    COALESCE(json_extract(payload_json, '$.completion_tokens'), 0) AS output_tokens
  FROM runtime_traces
  WHERE event_type = 'provider_call'
    AND COALESCE(json_extract(payload_json, '$.final_snapshot'), 0) = 1
    $SESSION_PREDICATE
)
SELECT
  provider,
  model,
  call_kind,
  COUNT(*) AS calls,
  SUM(usage_available) AS usage_calls,
  SUM(cache_read > 0) AS cache_hit_calls,
  ROUND(100.0 * SUM(cache_read > 0) / NULLIF(SUM(usage_available), 0), 2) AS request_hit_pct,
  SUM(input_tokens) AS input_tokens,
  SUM(cache_read) AS cache_read_tokens,
  ROUND(100.0 * SUM(cache_read) / NULLIF(SUM(input_tokens), 0), 2) AS token_hit_pct,
  SUM(cache_write) AS cache_write_tokens,
  SUM(output_tokens) AS output_tokens
FROM calls
GROUP BY provider, model, call_kind
ORDER BY provider, model, call_kind;

WITH boundaries AS (
  SELECT
    session_id,
    SUM(event_type = 'provider_request_prepared') AS prepared_requests,
    SUM(event_type = 'microcompaction_applied') AS microcompactions,
    SUM(event_type = 'context_compacted') AS full_compactions
  FROM runtime_traces
  WHERE 1 = 1
    $SESSION_PREDICATE
  GROUP BY session_id
)
SELECT
  session_id,
  prepared_requests,
  microcompactions,
  full_compactions
FROM boundaries
ORDER BY session_id;
SQL
