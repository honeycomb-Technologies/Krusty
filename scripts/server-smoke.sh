#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${KRUSTY_BASE_URL:-http://localhost:3000}"
SMOKE_ROOT="${KRUSTY_SMOKE_ROOT:-$(pwd)/target/server-smoke}"

PASS_COUNT=0
FAIL_COUNT=0
RESPONSE_FILE=""
RESPONSE_STATUS=""

cleanup() {
  if [[ -n "${RESPONSE_FILE:-}" && -f "${RESPONSE_FILE:-}" ]]; then
    rm -f "$RESPONSE_FILE"
  fi
}

trap cleanup EXIT

pass() {
  printf 'PASS %s\n' "$1"
  PASS_COUNT=$((PASS_COUNT + 1))
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

  cleanup
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

assert_status() {
  local expected="$1"
  local label="$2"
  if [[ "$RESPONSE_STATUS" == "$expected" ]]; then
    pass "$label"
  else
    fail "$label" "expected HTTP $expected, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
  fi
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

mkdir -p "$SMOKE_ROOT/projects"
FRESH_CODE_DIR="$SMOKE_ROOT/projects/fresh-code-session"
MAKO_DIR="$SMOKE_ROOT/projects/mako-smoke"
mkdir -p "$MAKO_DIR"

CODE_SESSION_ID=""
MAKO_SESSION_ID=""

printf 'Server smoke target: %s\n' "$BASE_URL"

request GET /health
assert_status 200 "health endpoint"
assert_json_eq '.status' 'ok' "health status"

request GET "/api/files/browse?path=$(urlencode "$SMOKE_ROOT")"
assert_status 200 "directory browse endpoint"
assert_json_eq '.current' "$SMOKE_ROOT" "directory browse current path"

request POST /api/sessions "$(jq -nc \
  --arg title 'Smoke Code Session' \
  --arg project_dir "$FRESH_CODE_DIR" \
  '{title: $title, project_dir: $project_dir, workspace_mode: "selected", session_type: "code"}')"
if [[ "$RESPONSE_STATUS" == "201" ]]; then
  pass "create code session in fresh directory"
  CODE_SESSION_ID="$(json '.id')"
else
  fail "create code session in fresh directory" "expected HTTP 201, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
fi

if [[ -z "$CODE_SESSION_ID" ]]; then
  printf '\nSummary: %d passed, %d failed\n' "$PASS_COUNT" "$FAIL_COUNT"
  exit 1
fi

assert_json_eq '.session_type' 'code' "code session type"
assert_json_eq '.project_dir' "$FRESH_CODE_DIR" "code session project directory"
assert_json_eq '.working_dir' "$FRESH_CODE_DIR" "code session working directory"

request GET "/api/sessions/$CODE_SESSION_ID"
assert_status 200 "get created code session"
assert_json_eq '.session.id' "$CODE_SESSION_ID" "get session id"

request GET "/api/sessions?working_dir=$(urlencode "$FRESH_CODE_DIR")"
assert_status 200 "list sessions by working directory"
assert_json_eq '.[0].id' "$CODE_SESSION_ID" "working directory session filter"

request PATCH "/api/sessions/$CODE_SESSION_ID" "$(jq -nc \
  --arg project_dir "$FRESH_CODE_DIR" \
  '{project_dir: $project_dir, workspace_mode: "created"}')"
assert_status 200 "update session workspace mode"
assert_json_eq '.workspace_mode' 'created' "created workspace mode persisted"

request GET /api/reports
assert_status 200 "reports endpoint"

request GET /api/memories
assert_status 200 "memories endpoint"

request GET "/api/memories/snapshot?project_dir=$(urlencode "$FRESH_CODE_DIR")"
assert_status 200 "memory snapshot endpoint"

request GET /api/mako/current
assert_status 200 "mako current endpoint"

request POST /api/mako/dispatch "$(jq -nc \
  --arg task 'Smoke-test current status' \
  --arg project_dir "$MAKO_DIR" \
  '{task: $task, project_dir: $project_dir, priority: "normal"}')"
if [[ "$RESPONSE_STATUS" == "200" || "$RESPONSE_STATUS" == "201" ]]; then
  pass "dispatch Mako run"
  MAKO_SESSION_ID="$(json '.session_id')"
else
  fail "dispatch Mako run" "expected HTTP 200/201, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
fi

if [[ -z "$MAKO_SESSION_ID" ]]; then
  printf '\nSummary: %d passed, %d failed\n' "$PASS_COUNT" "$FAIL_COUNT"
  exit 1
fi

request GET "/api/mako/sessions/$MAKO_SESSION_ID/status"
assert_status 200 "get Mako session status"
assert_json_eq '.session_type' 'mako' "Mako session type"

request DELETE "/api/mako/sessions/$MAKO_SESSION_ID"
if [[ "$RESPONSE_STATUS" == "204" || "$RESPONSE_STATUS" == "200" ]]; then
  pass "cancel Mako smoke session"
else
  fail "cancel Mako smoke session" "expected HTTP 200/204, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
fi

request DELETE "/api/sessions/$CODE_SESSION_ID"
if [[ "$RESPONSE_STATUS" == "204" || "$RESPONSE_STATUS" == "200" ]]; then
  pass "delete code smoke session"
else
  fail "delete code smoke session" "expected HTTP 200/204, got $RESPONSE_STATUS with body $(cat "$RESPONSE_FILE")"
fi

printf '\nSummary: %d passed, %d failed\n' "$PASS_COUNT" "$FAIL_COUNT"

if [[ "$FAIL_COUNT" -ne 0 ]]; then
  exit 1
fi
