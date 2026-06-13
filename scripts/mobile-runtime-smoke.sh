#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${KRUSTY_MOBILE_SERVER:-http://127.0.0.1:3000}"
RUN_CHAT="${MOBILE_SMOKE_CHAT:-0}"
RUN_LAUNCH="${MOBILE_SMOKE_LAUNCH:-0}"
LAUNCH_TIMEOUT="${MOBILE_SMOKE_LAUNCH_TIMEOUT:-8s}"

for arg in "$@"; do
	case "$arg" in
	--chat) RUN_CHAT=1 ;;
	--launch) RUN_LAUNCH=1 ;;
	*)
		printf 'unknown argument: %s\n' "$arg" >&2
		exit 2
		;;
	esac
done

TMPDIR="${TMPDIR:-/tmp}"
HEALTH_JSON="$TMPDIR/krusty-mobile-health.json"
MODELS_JSON="$TMPDIR/krusty-mobile-models.json"
SESSIONS_JSON="$TMPDIR/krusty-mobile-sessions.json"
CREATE_JSON="$TMPDIR/krusty-mobile-created-session.json"
STATE_JSON="$TMPDIR/krusty-mobile-session-state.json"
SERVER_ACCESS_JSON="$TMPDIR/krusty-mobile-server-access.json"
SERVER_STATUS_JSON="$TMPDIR/krusty-mobile-server-status.json"
CREDENTIALS_JSON="$TMPDIR/krusty-mobile-credentials.json"
CHAT_SSE="$TMPDIR/krusty-mobile-chat.sse"

api() {
	local path="$1"
	printf '%s%s' "$BASE_URL" "$path"
}

curl_json() {
	curl -fsS -H 'Accept: application/json' "$@"
}

printf '== Krusty mobile runtime smoke ==\n'
printf 'server: %s\n' "$BASE_URL"

curl_json "$(api /health)" >"$HEALTH_JSON"
python3 - "$HEALTH_JSON" <<'PY'
import json, sys
health=json.load(open(sys.argv[1]))
assert health.get('status') == 'ok', health
assert health.get('features', {}).get('chat') is True, health
print('health ok:', health.get('version'))
PY

curl_json "$(api /api/models)" >"$MODELS_JSON"
python3 - "$MODELS_JSON" <<'PY'
import json, sys
models=json.load(open(sys.argv[1]))
count=len(models.get('models', []))
assert count > 0, models
print('models ok:', count, 'default=', models.get('default_model'))
PY

curl_json "$(api /api/credentials)" >"$CREDENTIALS_JSON"
python3 - "$CREDENTIALS_JSON" <<'PY'
import json, sys
providers=json.load(open(sys.argv[1]))
assert isinstance(providers, list), providers
configured=sum(1 for provider in providers if provider.get('configured'))
print('credentials ok:', len(providers), 'providers, configured=', configured)
PY

curl_json "$(api /api/server/access)" >"$SERVER_ACCESS_JSON"
python3 - "$SERVER_ACCESS_JSON" <<'PY'
import json, sys
access=json.load(open(sys.argv[1]))
assert access.get('local_url'), access
assert isinstance(access.get('remote_access_enabled'), bool), access
print('server access ok:', access.get('local_url'), 'remote=', access.get('remote_launch_url'), 'tailscale=', access.get('tailscale', {}).get('status'))
PY

curl_json "$(api /api/server/status)" >"$SERVER_STATUS_JSON"
python3 - "$SERVER_STATUS_JSON" <<'PY'
import json, sys
status=json.load(open(sys.argv[1]))
assert isinstance(status.get('active_agent_streams'), int), status
print('server status ok:', status.get('active_agent_streams'), 'streams, active_sessions=', len(status.get('active_sessions', [])))
PY

curl_json "$(api /api/sessions)" >"$SESSIONS_JSON"
python3 - "$SESSIONS_JSON" <<'PY'
import json, sys
sessions=json.load(open(sys.argv[1]))
assert isinstance(sessions, list), sessions
print('sessions ok:', len(sessions))
PY

curl -fsS -X POST "$(api /api/sessions)" \
	-H 'Content-Type: application/json' \
	-H 'Accept: application/json' \
	--data '{"title":"Mobile Runtime Smoke","session_type":"chat","permission_mode":"supervised"}' \
	>"$CREATE_JSON"
SESSION_ID=$(
	python3 - "$CREATE_JSON" <<'PY'
import json, sys
session=json.load(open(sys.argv[1]))
assert session.get('id'), session
assert session.get('session_type') == 'chat', session
print(session['id'])
PY
)
printf 'create session ok: %s\n' "$SESSION_ID"

curl_json "$(api /api/sessions/$SESSION_ID)" >/dev/null
curl_json "$(api /api/sessions/$SESSION_ID/state)" >"$STATE_JSON"
python3 - "$STATE_JSON" <<'PY'
import json, sys
state=json.load(open(sys.argv[1]))
assert state.get('agent_state') in {'idle','streaming','tool_executing','awaiting_input','error'}, state
print('session state ok:', state.get('agent_state'))
PY

if [[ "$RUN_CHAT" == "1" ]]; then
	curl -fsS -N -X POST "$(api /api/chat)" \
		-H 'Content-Type: application/json' \
		-H 'Accept: text/event-stream' \
		--data '{"message":"Reply exactly: mobile-smoke-ok","session_type":"chat","permission_mode":"supervised","thinking_enabled":"off"}' \
		>"$CHAT_SSE"
	python3 - "$CHAT_SSE" <<'PY'
import json, sys
text=''
finish=None
for line in open(sys.argv[1], errors='replace'):
    line=line.strip()
    if not line.startswith('data:'):
        continue
    data=line[5:].strip()
    if not data or data == '[DONE]':
        continue
    event=json.loads(data)
    if event.get('type') in ('text_delta', 'text_delta_with_citations'):
        text += event.get('delta', '')
    if event.get('type') == 'finish':
        finish=event.get('session_id')
assert 'mobile-smoke-ok' in text, text
assert finish, 'missing finish event'
print('chat stream ok:', finish)
PY
else
	printf 'chat stream skipped (pass --chat to run a real model request)\n'
fi

if [[ "$RUN_LAUNCH" == "1" ]]; then
	printf 'launch smoke: cargo run -p krusty-mobile (timeout %s)\n' "$LAUNCH_TIMEOUT"
	set +e
	KRUSTY_MOBILE_SERVER="$BASE_URL" timeout "$LAUNCH_TIMEOUT" cargo run -p krusty-mobile
	status=$?
	set -e
	case "$status" in
	0) printf 'mobile launch smoke: exited_cleanly\n' ;;
	124) printf 'mobile launch smoke: running_window_timed_out\n' ;;
	*)
		printf 'mobile launch smoke: failed with status %s\n' "$status" >&2
		exit "$status"
		;;
	esac
else
	printf 'mobile launch skipped (pass --launch to compile/run the GPUI preview briefly)\n'
fi

printf 'mobile runtime smoke passed\n'
