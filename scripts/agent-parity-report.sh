#!/usr/bin/env bash
set -euo pipefail

if [[ $# -eq 1 ]]; then
  DB_PATH="${KRUSTY_DB_PATH:-$HOME/.krusty/krusty.db}"
  SESSION_ID="$1"
else
  DB_PATH="${1:-${KRUSTY_DB_PATH:-$HOME/.krusty/krusty.db}}"
  SESSION_ID="${2:-}"
fi

if [[ -z "$SESSION_ID" ]]; then
  echo "usage: $0 [database-path] <session-id>" >&2
  exit 2
fi

if [[ ! "$SESSION_ID" =~ ^[A-Za-z0-9_-]+$ ]]; then
  echo "session-id contains unsupported characters" >&2
  exit 2
fi

if [[ ! -f "$DB_PATH" ]]; then
  echo "database not found: $DB_PATH" >&2
  exit 2
fi

LEGACY_USAGE_ROWS=$(sqlite3 "$DB_PATH" "SELECT COUNT(*) FROM runtime_traces WHERE session_id = '$SESSION_ID' AND event_type = 'usage';")
if [[ "$LEGACY_USAGE_ROWS" -gt 0 ]]; then
  echo "note: legacy usage rows are excluded because they lack exact provider-call boundaries; run a new session for comparable totals" >&2
fi

sqlite3 -header -column "$DB_PATH" <<SQL
WITH run_events AS (
  SELECT
    run_id,
    MIN(sequence) AS first_sequence,
    COUNT(*) FILTER (WHERE event_type = 'tool_call_complete') AS tool_calls,
    COUNT(*) FILTER (
      WHERE event_type = 'tool_result'
        AND COALESCE(json_extract(payload_json, '$.is_error'), 0) = 1
    ) AS tool_failures,
    COUNT(*) FILTER (WHERE event_type IN ('error', 'server_tool_error')) AS run_errors,
    ROUND((julianday(MAX(created_at)) - julianday(MIN(created_at))) * 86400.0, 3) AS seconds
  FROM runtime_traces
  WHERE session_id = '$SESSION_ID'
  GROUP BY run_id
), provider_usage AS (
  SELECT
    run_id,
    COUNT(*) AS provider_calls,
    COUNT(*) FILTER (WHERE call_kind = 'agent_loop') AS agent_loop_calls,
    COUNT(*) FILTER (WHERE call_kind = 'auxiliary') AS auxiliary_calls,
    GROUP_CONCAT(DISTINCT operation) AS operations,
    COUNT(*) FILTER (
      WHERE COALESCE(json_extract(payload_json, '$.usage_available'), 0) = 0
    ) AS calls_without_usage,
    COALESCE(SUM(json_extract(payload_json, '$.prompt_tokens')), 0) AS uncached_input,
    COALESCE(SUM(json_extract(payload_json, '$.cache_creation_input_tokens')), 0) AS cache_write,
    COALESCE(SUM(json_extract(payload_json, '$.cache_read_input_tokens')), 0) AS cache_read,
    COALESCE(SUM(json_extract(payload_json, '$.completion_tokens')), 0) AS output_tokens,
    COALESCE(SUM(json_extract(payload_json, '$.reasoning_tokens')), 0) AS reasoning_tokens,
    COALESCE(SUM(json_extract(payload_json, '$.total_tokens')), 0) AS logical_total
  FROM runtime_traces
  WHERE session_id = '$SESSION_ID'
    AND event_type = 'provider_call'
    AND COALESCE(json_extract(payload_json, '$.final_snapshot'), 0) = 1
  GROUP BY run_id
)
SELECT
  run_events.run_id,
  COALESCE(provider_usage.provider_calls, 0) AS provider_calls,
  COALESCE(provider_usage.agent_loop_calls, 0) AS agent_loop_calls,
  COALESCE(provider_usage.auxiliary_calls, 0) AS auxiliary_calls,
  COALESCE(provider_usage.operations, '') AS operations,
  COALESCE(provider_usage.calls_without_usage, 0) AS calls_without_usage,
  run_events.tool_calls,
  run_events.tool_failures,
  run_events.run_errors,
  run_events.seconds,
  COALESCE(provider_usage.uncached_input, 0) AS uncached_input,
  COALESCE(provider_usage.cache_write, 0) AS cache_write,
  COALESCE(provider_usage.cache_read, 0) AS cache_read,
  COALESCE(provider_usage.output_tokens, 0) AS output_tokens,
  COALESCE(provider_usage.reasoning_tokens, 0) AS reasoning_tokens,
  COALESCE(provider_usage.logical_total, 0) AS logical_total
FROM run_events
LEFT JOIN provider_usage USING (run_id)
ORDER BY run_events.first_sequence;

SELECT
  delegated_run_id,
  role,
  stage,
  provider,
  model,
  resumable,
  updated_at
FROM delegated_runs
WHERE parent_session_id = '$SESSION_ID'
ORDER BY updated_at;
SQL
