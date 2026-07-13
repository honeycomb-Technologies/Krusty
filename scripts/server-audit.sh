#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${KRUSTY_BASE_URL:-http://localhost:3000}"
AUDIT_ROOT="${KRUSTY_AUDIT_ROOT:-$(pwd)/target/server-audit}"
TEMP_PORT="${KRUSTY_AUDIT_PORT:-43111}"

PASS_COUNT=0
WARN_COUNT=0
SKIP_COUNT=0
FAIL_COUNT=0
RESPONSE_FILE=""
RESPONSE_STATUS=""
HTTP_SERVER_PID=""
CODE_SESSION_ID=""
MAKO_SESSION_ID=""
HOOK_ID=""
APNS_DEVICE_TOKEN="audit-device-token"
# Use an inert path on a real browser push origin. The server intentionally
# rejects arbitrary hosts to prevent SSRF, and this subscription is removed
# before the audit exercises push delivery.
PUSH_ENDPOINT="https://fcm.googleapis.com/fcm/send/krusty-audit"
# SEC1-encoded NIST P-256 generator point and a 16-byte base64url auth
# secret. These are public audit fixtures, not credentials.
PUSH_P256DH="BGsX0fLhLEJH-Lzm5WOkQPJ3A32BLeszoPShOUXYmMKWT-NC4v4af5uO5-tKfA-eFivOM1drMV7Oy7ZAaDe_UfU"
PUSH_AUTH="YWJjZGVmZ2hpamtsbW5vcA"
FIRST_MODEL_ID=""
FIRST_MCP_NAME=""

cleanup() {
  if [[ -n "${HTTP_SERVER_PID:-}" ]]; then
    kill "${HTTP_SERVER_PID}" >/dev/null 2>&1 || true
  fi
  if [[ -n "${RESPONSE_FILE:-}" && -f "${RESPONSE_FILE:-}" ]]; then
    rm -f "$RESPONSE_FILE"
  fi
}

trap cleanup EXIT

pass() {
  printf 'PASS %s\n' "$1"
  PASS_COUNT=$((PASS_COUNT + 1))
}

warn() {
  printf 'WARN %s\n' "$1"
  printf '  %s\n' "$2"
  WARN_COUNT=$((WARN_COUNT + 1))
}

skip() {
  printf 'SKIP %s\n' "$1"
  printf '  %s\n' "$2"
  SKIP_COUNT=$((SKIP_COUNT + 1))
}

fail() {
  printf 'FAIL %s\n' "$1"
  printf '  %s\n' "$2"
  FAIL_COUNT=$((FAIL_COUNT + 1))
}

json() {
  jq -r "$1" "$RESPONSE_FILE"
}

urlencode() {
  jq -rn --arg value "$1" '$value|@uri'
}

request() {
  local method="$1"
  local path="$2"
  local body="${3:-}"

  if [[ -n "${RESPONSE_FILE:-}" && -f "${RESPONSE_FILE:-}" ]]; then
    rm -f "$RESPONSE_FILE"
  fi
  RESPONSE_FILE="$(mktemp)"

  if [[ -n "$body" ]]; then
    RESPONSE_STATUS="$(
      curl -sS -o "$RESPONSE_FILE" -w '%{http_code}' \
        -X "$method" \
        -H 'Content-Type: application/json' \
        --data "$body" \
        "$BASE_URL$path"
    )"
  else
    RESPONSE_STATUS="$(
      curl -sS -o "$RESPONSE_FILE" -w '%{http_code}' \
        -X "$method" \
        "$BASE_URL$path"
    )"
  fi
}

request_sse() {
  local path="$1"
  local body="$2"

  if [[ -n "${RESPONSE_FILE:-}" && -f "${RESPONSE_FILE:-}" ]]; then
    rm -f "$RESPONSE_FILE"
  fi
  RESPONSE_FILE="$(mktemp)"

  RESPONSE_STATUS="$(
    curl -sS -o "$RESPONSE_FILE" -w '%{http_code}' \
      --max-time 20 \
      -X POST \
      -H 'Content-Type: application/json' \
      -H 'Accept: text/event-stream' \
      --data "$body" \
      "$BASE_URL$path"
  )"
}

assert_status() {
  local expected="$1"
  local label="$2"
  if [[ "$RESPONSE_STATUS" == "$expected" ]]; then
    pass "$label"
  else
    fail "$label" "expected HTTP $expected, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
  fi
}

assert_status_any() {
  local label="$1"
  shift
  local expected
  for expected in "$@"; do
    if [[ "$RESPONSE_STATUS" == "$expected" ]]; then
      pass "$label"
      return
    fi
  done
  fail "$label" "expected one of [$*], got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
}

assert_json_eq() {
  local jq_filter="$1"
  local expected="$2"
  local label="$3"
  local actual
  actual="$(json "$jq_filter")"
  if [[ "$actual" == "$expected" ]]; then
    pass "$label"
  else
    fail "$label" "expected '$expected', got '$actual'"
  fi
}

assert_json_nonempty() {
  local jq_filter="$1"
  local label="$2"
  local actual
  actual="$(json "$jq_filter")"
  if [[ -n "$actual" && "$actual" != "null" ]]; then
    pass "$label"
  else
    fail "$label" "expected non-empty value, got '$actual'"
  fi
}

mkdir -p "$AUDIT_ROOT/projects" "$AUDIT_ROOT/files" "$AUDIT_ROOT/http"
printf 'Server audit target: %s\n' "$BASE_URL"

request GET /health
assert_status 200 "health endpoint"
assert_json_eq '.status' 'ok' "health status"

request GET /api/server/access
assert_status 200 "server access endpoint"
CURRENT_REMOTE_ACCESS_ENABLED="$(json '.remote_access_enabled')"
request PATCH /api/server/access "$(jq -nc --argjson enabled "${CURRENT_REMOTE_ACCESS_ENABLED}" '{enabled: $enabled}')"
assert_status 200 "server access patch endpoint"
assert_json_eq '.remote_access_enabled' "${CURRENT_REMOTE_ACCESS_ENABLED}" "server access patch preserved enabled state"

request GET /api/server/status
assert_status 200 "server status endpoint"
assert_json_nonempty '.memory' "server status returns memory payload"

request GET /api/models
assert_status 200 "models endpoint"
FIRST_MODEL_ID="$(json '.models[0].id')"
if [[ -n "$FIRST_MODEL_ID" && "$FIRST_MODEL_ID" != "null" ]]; then
  request GET "/api/models/$(urlencode "$FIRST_MODEL_ID")"
  assert_status 200 "model detail endpoint"
  assert_json_eq '.id' "$FIRST_MODEL_ID" "model detail id"
else
  warn "model detail endpoint" "no models were returned to verify a model detail lookup"
fi

request GET /api/tools
assert_status 200 "tools list endpoint"
if jq -e '.[] | select(.name == "glob")' "$RESPONSE_FILE" >/dev/null; then
  request POST /api/tools/execute "$(jq -nc \
    --arg wd "$(pwd)" \
    '{tool_name: "glob", working_dir: $wd, params: {pattern: "AGENTS.md"}}')"
  assert_status 200 "tool execute endpoint"
  assert_json_eq '.is_error' 'false' "tool execute returned success"
else
  warn "tool execute endpoint" "glob tool not present in tool registry response"
fi

FRESH_CODE_DIR="$AUDIT_ROOT/projects/fresh-code-session"
request POST /api/sessions "$(jq -nc \
  --arg title 'Audit Code Session' \
  --arg project_dir "$FRESH_CODE_DIR" \
  '{title: $title, project_dir: $project_dir, workspace_mode: "selected", session_type: "code"}')"
if [[ "$RESPONSE_STATUS" == "201" ]]; then
  pass "create code session endpoint"
  CODE_SESSION_ID="$(json '.id')"
else
  fail "create code session endpoint" "expected HTTP 201, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
fi

if [[ -n "$CODE_SESSION_ID" ]]; then
  request GET "/api/sessions/$CODE_SESSION_ID"
  assert_status 200 "get session endpoint"
  assert_json_eq '.session.id' "$CODE_SESSION_ID" "get session id"

  request GET "/api/sessions?working_dir=$(urlencode "$FRESH_CODE_DIR")"
  assert_status 200 "list sessions filter endpoint"

  request GET /api/sessions/directories
  assert_status 200 "session directories endpoint"

  request PATCH "/api/sessions/$CODE_SESSION_ID" "$(jq -nc \
    --arg title 'Audit Renamed Session' \
    --arg project_dir "$FRESH_CODE_DIR" \
    '{title: $title, project_dir: $project_dir, workspace_mode: "created"}')"
  assert_status 200 "update session endpoint"
  assert_json_eq '.workspace_mode' 'created' "update session workspace mode"

  request GET "/api/sessions/$CODE_SESSION_ID/state"
  assert_status 200 "session state endpoint"

  request GET "/api/sessions/$CODE_SESSION_ID/trace"
  assert_status 200 "session trace endpoint"

  request GET "/api/sessions/$CODE_SESSION_ID/presence"
  assert_status 200 "session presence get endpoint"

  request PUT "/api/sessions/$CODE_SESSION_ID/presence" "$(jq -nc \
    '{client_id: "audit-client", surface: "mobile", capability: "controller"}')"
  assert_status 200 "session presence heartbeat endpoint"

  request DELETE "/api/sessions/$CODE_SESSION_ID/presence/audit-client"
  assert_status 200 "session presence removal endpoint"

  skip "session pinch endpoint" "not exercised in audit because it materially rewrites session state"
  skip "session tool approval endpoint" "requires a live pending approval to exercise meaningfully"
fi

request_sse /api/chat "$(jq -nc \
  --arg message 'Audit chat ping' \
  --arg project_dir "$AUDIT_ROOT/projects/chat" \
  '{message: $message, project_dir: $project_dir, workspace_mode: "selected", session_type: "chat"}')"
if [[ "$RESPONSE_STATUS" == "200" ]]; then
  if grep -q '^data:' "$RESPONSE_FILE"; then
    if grep -q '"type":"error"' "$RESPONSE_FILE"; then
      warn "chat stream endpoint" "route responded over SSE but emitted an error event: $(tr '\n' ' ' < "$RESPONSE_FILE")"
    else
      pass "chat stream endpoint"
    fi
  else
    warn "chat stream endpoint" "HTTP 200 without SSE data frames: $(cat "$RESPONSE_FILE")"
  fi
elif [[ "$RESPONSE_STATUS" == "400" ]]; then
  warn "chat stream endpoint" "route rejected request due current AI environment: $(cat "$RESPONSE_FILE")"
else
  fail "chat stream endpoint" "unexpected HTTP $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
fi

request GET "/api/git/status?path=$(urlencode "$(pwd)")"
assert_status 200 "git status endpoint"
request GET "/api/git/branches?path=$(urlencode "$(pwd)")"
assert_status 200 "git branches endpoint"
request GET "/api/git/worktrees?path=$(urlencode "$(pwd)")"
assert_status 200 "git worktrees endpoint"
skip "git checkout endpoint" "skipped to avoid mutating repository branch state during audit"

AUDIT_FILE="$AUDIT_ROOT/files/probe.txt"
request PUT "/api/files?path=$(urlencode "$AUDIT_FILE")" "$(jq -nc '{content: "krusty-audit"}')"
assert_status 200 "file write endpoint"
request GET "/api/files?path=$(urlencode "$AUDIT_FILE")"
assert_status 200 "file read endpoint"
assert_json_eq '.content' 'krusty-audit' "file read content"
request GET "/api/files/tree?root=$(urlencode "$AUDIT_ROOT")&depth=2"
assert_status 200 "file tree endpoint"
request GET "/api/files/browse?path=$(urlencode "$AUDIT_ROOT")"
assert_status 200 "file browse endpoint"

request GET /api/credentials
assert_status 200 "credentials list endpoint"
request GET /api/credentials/openai
assert_status 200 "credential provider endpoint"
skip "credential set endpoint" "skipped to avoid mutating configured provider secrets"
skip "credential delete endpoint" "skipped to avoid mutating configured provider secrets"

request GET /api/mako/current
assert_status 200 "mako current endpoint"
request POST /api/mako/dispatch "$(jq -nc \
  --arg task 'Audit Mako route health' \
  --arg project_dir "$AUDIT_ROOT/projects/mako" \
  '{task: $task, project_dir: $project_dir, priority: "normal"}')"
if [[ "$RESPONSE_STATUS" == "200" || "$RESPONSE_STATUS" == "201" ]]; then
  pass "mako dispatch endpoint"
  MAKO_SESSION_ID="$(json '.session_id')"
else
  fail "mako dispatch endpoint" "expected HTTP 200/201, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
fi

request GET /api/mako/sessions
assert_status 200 "mako sessions list endpoint"
if [[ -n "$MAKO_SESSION_ID" ]]; then
  request GET "/api/mako/sessions/$MAKO_SESSION_ID/status"
  assert_status 200 "mako session status endpoint"
  request POST "/api/mako/sessions/$MAKO_SESSION_ID/message" "$(jq -nc '{message: "Audit follow-up"}')"
  assert_status 200 "mako send message endpoint"
  request POST "/api/mako/sessions/$MAKO_SESSION_ID/priority" "$(jq -nc '{priority: "high"}')"
  assert_status 200 "mako priority endpoint"
  request POST "/api/mako/sessions/$MAKO_SESSION_ID/schedule" "$(jq -nc --arg start_at "$(date -u -d '+10 minutes' '+%Y-%m-%dT%H:%M:%SZ')" '{start_at: $start_at}')"
  assert_status 200 "mako schedule endpoint"
  request POST "/api/mako/sessions/$MAKO_SESSION_ID/pause" '{}'
  assert_status 200 "mako pause endpoint"
  request POST "/api/mako/sessions/$MAKO_SESSION_ID/resume" '{}'
  assert_status 200 "mako resume endpoint"
  skip "mako events endpoint" "skipped because SSE observation is long-lived and already covered by status/control endpoints in this audit"
fi

request GET /api/mcp
assert_status 200 "mcp list endpoint"
request POST /api/mcp/reload '{}'
assert_status 200 "mcp reload endpoint"
FIRST_MCP_NAME="$(json '.[0].name')"
if [[ -n "$FIRST_MCP_NAME" && "$FIRST_MCP_NAME" != "null" ]]; then
  request GET "/api/mcp/$(urlencode "$FIRST_MCP_NAME")/tools"
  assert_status 200 "mcp tools endpoint"
  skip "mcp connect endpoint" "skipped to avoid perturbing live MCP connection state during audit"
  skip "mcp disconnect endpoint" "skipped to avoid perturbing live MCP connection state during audit"
else
  warn "mcp tools endpoint" "no MCP servers were configured to probe by name"
fi

request GET /api/memories
assert_status 200 "memories list endpoint"
request GET "/api/memories/snapshot?project_dir=$(urlencode "$FRESH_CODE_DIR")"
assert_status 200 "memory snapshot endpoint"

request GET /api/processes
assert_status 200 "process list endpoint"
FIRST_PROCESS_ID="$(json '.[0].id')"
if [[ -n "$FIRST_PROCESS_ID" && "$FIRST_PROCESS_ID" != "null" ]]; then
  request GET "/api/processes/$FIRST_PROCESS_ID"
  assert_status 200 "process detail endpoint"
else
  warn "process detail endpoint" "no background processes were registered to inspect"
fi
skip "process kill/suspend/resume endpoints" "skipped to avoid mutating live process state during audit"

python3 -m http.server "$TEMP_PORT" --bind 127.0.0.1 --directory "$AUDIT_ROOT/http" >/dev/null 2>&1 &
HTTP_SERVER_PID="$!"
sleep 1

request GET /api/ports
assert_status 200 "ports list endpoint"
request GET "/api/ports/$TEMP_PORT/proxy"
assert_status 200 "ports proxy endpoint"

request GET /api/settings/preview
assert_status 200 "preview settings get endpoint"
CURRENT_PREVIEW_SETTINGS="$(cat "$RESPONSE_FILE")"
request PATCH /api/settings/preview "$(jq -c '.' "$RESPONSE_FILE")"
assert_status 200 "preview settings patch endpoint"
request POST /api/settings/preview/pins "$(jq -nc --argjson port "$TEMP_PORT" '{port: $port}')"
assert_status 200 "preview pin add endpoint"
request DELETE "/api/settings/preview/pins/$TEMP_PORT"
assert_status 200 "preview pin remove endpoint"
request POST /api/settings/preview/hidden "$(jq -nc --argjson port "$TEMP_PORT" '{port: $port}')"
assert_status 200 "preview hidden add endpoint"
request DELETE "/api/settings/preview/hidden/$TEMP_PORT"
assert_status 200 "preview hidden remove endpoint"
request PATCH /api/settings/preview "$CURRENT_PREVIEW_SETTINGS"
assert_status 200 "preview settings restore endpoint"

request GET /api/hooks
assert_status 200 "hooks list endpoint"
request POST /api/hooks "$(jq -nc \
  '{hook_type: "PreToolUse", tool_pattern: "^glob$", command: "echo audit-hook"}')"
if [[ "$RESPONSE_STATUS" == "201" ]]; then
  pass "hook create endpoint"
  HOOK_ID="$(json '.id')"
else
  fail "hook create endpoint" "expected HTTP 201, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
fi
if [[ -n "$HOOK_ID" ]]; then
  request PATCH "/api/hooks/$HOOK_ID/toggle" '{}'
  assert_status 200 "hook toggle endpoint"
  request DELETE "/api/hooks/$HOOK_ID"
  assert_status 204 "hook delete endpoint"
fi

request GET /api/push/status
assert_status 200 "push status endpoint"
request POST /api/push/subscribe "$(jq -nc \
  --arg endpoint "$PUSH_ENDPOINT" \
  --arg p256dh "$PUSH_P256DH" \
  --arg auth "$PUSH_AUTH" \
  '{endpoint: $endpoint, p256dh: $p256dh, auth: $auth}')"
assert_status 200 "push subscribe endpoint"
request DELETE /api/push/subscribe "$(jq -nc --arg endpoint "$PUSH_ENDPOINT" '{endpoint: $endpoint}')"
assert_status 200 "push unsubscribe endpoint"
request GET /api/push/vapid-public-key
if [[ "$RESPONSE_STATUS" == "200" ]]; then
  pass "push vapid public key endpoint"
else
  warn "push vapid public key endpoint" "push service unavailable in current environment: $(cat "$RESPONSE_FILE")"
fi
request POST /api/push/test '{}'
if [[ "$RESPONSE_STATUS" == "200" ]]; then
  pass "push test endpoint"
else
  warn "push test endpoint" "push service unavailable in current environment: $(cat "$RESPONSE_FILE")"
fi

request GET /api/apns/status
assert_status 200 "apns status endpoint"
request POST /api/apns/register "$(jq -nc --arg token "$APNS_DEVICE_TOKEN" '{device_token: $token}')"
assert_status 200 "apns register endpoint"
request DELETE /api/apns/register "$(jq -nc --arg token "$APNS_DEVICE_TOKEN" '{device_token: $token}')"
assert_status 200 "apns unregister endpoint"
request POST /api/apns/test '{}'
if [[ "$RESPONSE_STATUS" == "200" ]]; then
  pass "apns test endpoint"
else
  warn "apns test endpoint" "APNs service unavailable in current environment: $(cat "$RESPONSE_FILE")"
fi

request GET /api/reports
assert_status 200 "reports list endpoint"
REPORT_ID="$(json '.reports[0].id')"
if [[ -n "$REPORT_ID" && "$REPORT_ID" != "null" ]]; then
  request GET "/api/reports/$REPORT_ID"
  assert_status 200 "report detail endpoint"
else
  warn "report detail endpoint" "no reports existed to fetch by id"
fi

request GET /api/skills
assert_status 200 "skills list endpoint"
request GET /api/skills?scope=global
assert_status 200 "skills global list endpoint"

request GET /api/auth/oauth/status/openai
assert_status 200 "oauth status openai endpoint"
request GET /api/auth/oauth/status/anthropic
assert_status 200 "oauth status anthropic endpoint"
skip "oauth start endpoint" "skipped to avoid initiating an external OAuth flow during audit"
skip "oauth exchange endpoint" "skipped because it requires a real authorization code"
skip "oauth revoke endpoint" "skipped to avoid deleting any live OAuth token"
skip "oauth callback endpoint" "skipped because it is part of a real browser OAuth flow"

if [[ -n "$MAKO_SESSION_ID" ]]; then
  request DELETE "/api/mako/sessions/$MAKO_SESSION_ID"
  assert_status_any "mako cancel endpoint" 200 204
fi

if [[ -n "$CODE_SESSION_ID" ]]; then
  request DELETE "/api/sessions/$CODE_SESSION_ID"
  assert_status_any "session delete endpoint" 200 204
fi

printf '\nSummary: %d passed, %d warned, %d skipped, %d failed\n' \
  "$PASS_COUNT" "$WARN_COUNT" "$SKIP_COUNT" "$FAIL_COUNT"

if [[ "$FAIL_COUNT" -ne 0 ]]; then
  exit 1
fi
