# mitsuro-desktop-backend

Protocol types and agent backends for the Mitsuro desktop shell.

## Backends

| Backend | Status | Role |
|---------|--------|------|
| `CodexAppServerBackend` | wired | Spawns local `codex app-server` over stdio |
| `MitsuroServerBackend` | wired for core conversations | Uses canonical `mitsuro-client` over HTTP/SSE |
| `FixtureBackend` | explicit typed subset | Deterministic development and tests |

All implement [`AgentBackend`](src/backend.rs). [`DesktopBackend`](src/desktop.rs)
is the desktop selection/capability boundary and implements the transport-neutral
[`ProductBackend`](src/product.rs) session/model/turn, file/catalog, process, Hive,
and schedule contracts used by GPUI.
`AgentBackend` still contains legacy Codex-shaped methods for backend-specific
surfaces; unsupported methods must return `NotImplemented` and must not be
represented as working product features.

## Mitsuro transport

`MitsuroServerBackend` supports health/connection, session list/create/read/rename/delete,
model list, SSE turn streaming, approvals, cancellation, text-file browsing/reading/fuzzy
search, skills, MCP status, and installed agent-extension discovery. Its base URL defaults
to `http://127.0.0.1:3000` and can be changed with `MITSURO_SERVER_URL`. Remote servers use
`MITSURO_SERVER_TOKEN`; the shared client installs it as a bearer header.

The Mitsuro process API can inspect and control processes already tracked by the server,
but it cannot spawn an interactive PTY. The GPUI terminal therefore disables live spawn
for Mitsuro instead of substituting fixture output. Codex stdio uses the current
`command/exec`, `command/exec/write`, `command/exec/resize`, and
`command/exec/terminate` contract for standalone interactive commands. The initial
request intentionally has no generic client timeout because app-server resolves it only
after process exit; the request itself carries explicit server timeout/output policy.
Hive current state, global schedules, and background processes are
read-only product surfaces in GPUI; their mutation routes are intentionally not exposed.

## Transport assumptions (Codex app-server)

Verified against the committed `codex-cli 0.147.0` protocol baseline on Linux
(`~/.local/bin/codex app-server --stdio`):

1. **Spawn:** `codex app-server --stdio` (or `CODEX_BIN` env override, default `~/.local/bin/codex` then `codex` on `PATH`).
2. **Framing:** newline-delimited JSON (JSONL). One JSON object per line on stdin/stdout. **Not** LSP `Content-Length` framing.
3. **Request shape:**
   ```json
   {"id":1,"method":"initialize","params":{...}}
   ```
   The JSON-RPC `"jsonrpc":"2.0"` field is optional on this wire; this client includes it for clarity.
4. **Response shape:** `{"id":1,"result":{...}}` or `{"id":1,"error":{"code":...,"message":"..."}}`.
5. **Notifications:** server→client messages with `method` + `params` and **no** `id` (often include `emittedAtMs`).
6. **Server requests:** client must answer approvals (`execCommandApproval`, `applyPatchApproval`, `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`). Incoming requests are classified, mapped to `TurnStreamEvent::ApprovalRequested`, and answered via `CodexAppServerBackend::respond_approval` / `approve` / `deny`.
7. **Handshake:** first client call must be `initialize` with `clientInfo` (`name`, `version`; optional `title`) and optional `capabilities`. The desktop explicitly enables `experimentalApi` because its process, environment, realtime, and background-terminal surfaces use experimental methods.
8. **Offline-safe methods:** `initialize`, `thread/list`, and `thread/start` (use `ephemeral: true` when probing) do not require paid model calls.
9. **Correlation:** request `id` may be `u64` or `string`; this client uses monotonic integer ids.

## Wired methods

- `initialize`
- `thread/list`
- `thread/start`
- `thread/read`
- `turn/start` (live; fixture replay requires explicit fixture mode)
- `account/read` · `account/login/start` · `account/login/cancel` · `account/logout`
- `account/usage/read` · `account/rateLimits/read` (fixture demo offline; no paid models)
- `remoteControl/status/read` · `remoteControl/enable` · `remoteControl/disable`
- `remoteControl/pairing/start` · `remoteControl/pairing/status`
- `remoteControl/client/list` · `remoteControl/client/revoke`
- `externalAgentConfig/detect` · `externalAgentConfig/import`
- `externalAgentConfig/import/readHistories` · `externalAgentConfig/import/recordHistory`

Remote Control is a Codex-only capability in the current transport matrix. Its typed
status, pairing, client-list, revocation, and lifecycle contracts never fall back to
fixture records; Mitsuro HTTP returns `NotImplemented` through the desktop boundary.

External-agent discovery, import, progress/completion, and history use generated typed
contracts. Detection is read-only; import preserves each detected item's lossless
details payload and remains Codex-only at the desktop capability boundary.

The generated protocol inventories are committed in `fixtures/`: 95 stable client
methods, 133 methods with experimental APIs enabled, 70 server notifications, 10 stable
server requests (11 experimental), and 18 thread item variants for `codex-cli 0.147.0`.
The experimental-only `currentTime/read` request is answered directly from the
client-owned system clock so it cannot stall a turn. Run
`scripts/gpui-codex-protocol-check.sh` after changing the Codex CLI; use `--update` only
when intentionally accepting a reviewed protocol baseline. Inventory and generic Codex
forwarding are not evidence that a fixture or UI feature is implemented.

## Turn streaming

Server notifications are mapped to [`TurnStreamEvent`](src/types.rs):

| Notification | Event |
|--------------|--------|
| `turn/started` | `TurnStarted` |
| `turn/completed` | `TurnCompleted` |
| `item/started` | `ItemStarted` |
| `item/completed` | `ItemCompleted` |
| `item/agentMessage/delta` | `AgentMessageDelta` |
| `item/reasoning/textDelta` | `ReasoningTextDelta` |
| `item/reasoning/summaryTextDelta` | `ReasoningSummaryDelta` |
| `item/plan/delta` | `PlanDelta` |
| `execCommandApproval` / `applyPatchApproval` / `item/*/requestApproval` | `ApprovalRequested` |

Hydrated and live transcripts preserve all 18 current thread item variants. The six
conversation-native variants keep their specialized renderers; tool, search, image,
collaboration, review-mode, compaction, hook, and sleep variants render as restrained
activity rows with their real title, status, and bounded protocol summary. Unknown
future variants remain visible as forward-compatible activity instead of disappearing.

Offline path: [`FixtureBackend`](src/fixture.rs) + `fixtures/sample-turn.jsonl` (embedded as `SAMPLE_TURN_JSONL`). The sample stream injects a mid-turn `item/commandExecution/requestApproval` server request.

Approvals: [`approvals`](src/approvals.rs) — params/response types, `build_approval_result`, `parse_approval_request`.

### Progressive live turns (mid-stream approvals)

Collect-all-then-apply hangs when the server waits on an approval RPC. Use [`live_turn`](src/live_turn.rs):

| API | Role |
|-----|------|
| `run_live_turn_progressive` | Deliver each `TurnStreamEvent` as it arrives; `on_approval` → `respond_approval` before further recv |
| `run_live_turn_with_policy` | Non-interactive `AutoApprove` / `AutoReject` (tests) |
| `run_live_turn_with_bridge` / `_blocking` | UI: block on `LiveApprovalBridge` until Approve/Reject |
| `LiveApprovalBridge` | `wait` / `submit` handoff between turn loop and ApprovalBar |

Real network turns remain gated by `MITSURO_ALLOW_LIVE_TURN=1` in the desktop shell.

## Tests

```bash
# Unit tests always (mock stdio + fixture parse + progressive mid-stream approval)
cargo test -p mitsuro-desktop-backend

# Read-only Mitsuro server contract (sessions/models/files/skills/MCP/extensions)
MITSURO_RUN_SERVER_IT=1 cargo test -p mitsuro-desktop-backend \
  live_server_read_only_contract -- --nocapture

# Live turn/start only with explicit opt-in (may use paid models):
MITSURO_ALLOW_LIVE_TURN=1 cargo test -p mitsuro-desktop-backend real_app_server_turn_start

# Reproducible GPUI acceptance across both real transports (safe contracts by default):
scripts/gpui-dual-backend-acceptance.sh

# Require completed provider-backed streaming turns from Mitsuro and Codex (may use credits):
MITSURO_RUN_LIVE_ACCEPTANCE=1 scripts/gpui-dual-backend-acceptance.sh
```

## Contract sources

- Mitsuro: `../mitsuro-client` plus the server route and type contracts.
- Codex: typed protocol models in this crate plus the maintained method inventory.
