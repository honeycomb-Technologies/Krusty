# Grok stack reverse-compaction / context-reconstruction deep dive

**Scope searched:** `/home/burgess/Work/grok-stack/grok-app` and `/home/burgess/Work/grok-stack/grok-server` active source/docs, excluding build artifacts/deps. I also checked the two-commit git history for compaction/reconstruction terms.

## Bottom line

I did **not** find a reverse-compaction or context-reconstruction implementation in `grok-stack`. This repository is a remote-control gateway/UI around an external `grok agent stdio -m grok-build` ACP agent, not the inner agent runtime. The only implemented “context” behavior is live event translation, in-memory session state, bounded UI/event caches, and plan-mode approvals.

If Grok Build has an actual reverse-compaction system, it is likely inside the external `grok` binary / Grok session store, not in this repo. Evidence:

- `grok-server` declares itself an ACP client gateway to real Grok Build agents, not the agent core: `grok-server/src/main.rs:3-6`, `grok-server/README.md:7-13`.
- Prompts are forwarded as just the current text to ACP `PromptRequest`; no history, summaries, snapshots, or reconstructed context are assembled here: `grok-server/src/acp_client.rs:175-198`.
- `get_status` is explicitly “cheap, no history”: `grok-server/src/server.rs:62-64`.
- Server session state is an in-memory `DashMap`, with cancellation history pruned/restarted away: `grok-server/src/session.rs:30-43`, `grok-server/src/session.rs:153-181`.
- Exact active-source search for `compaction|reconstruct|reverse compact|checkpoint|token budget|context window|summarize` found no implementation hits beyond UI “compact” styling and one non-text-content “summarized” comment.

## Files that matter

| Area | File | Evidence / role |
|---|---|---|
| Server bootstrap | `grok-server/src/main.rs` | Long-lived app server that acts as an ACP client to `grok agent stdio -m grok-build` (`:3-6`); default `--grok-cmd grok` and `--model grok-build` (`:41-47`); starts stdio or WS and drains in-memory sessions on shutdown (`:90-126`). |
| API facade | `grok-server/src/server.rs` | JSON-RPC methods are create/list/prompt/approve/cancel/get_status/discovery/stream (`:34-84`); create spawns a session via `SessionManager` (`:105-129`); prompt forwards to session/ACP (`:136-165`); stream broadcasts `UpdateKind` with per-subscription seq (`:278-313`). |
| Session state | `grok-server/src/session.rs` | In-memory `SessionManager.sessions: DashMap` (`:30-43`); `create_session` spawns Grok ACP and event pump (`:46-107`); list preview uses live command/tool counts (`:118-132`); cancel keeps only transient historical entries up to 64 (`:153-181`). |
| ACP gateway | `grok-server/src/acp_client.rs` | Spawns command and appends `--permission-mode plan` (`:54-75`); handles ACP notifications and permission requests (`:88-138`); initializes ACP session (`:140-160`); forwards prompts as one text block (`:175-198`); maps ACP updates to local events (`:243-337`). |
| Data models | `grok-server/src/types.rs` | DTOs are IDs, plan/diff/tool/status/update stream models (`:20-100`, `:113-194`, `:198-308`); no Summary/Checkpoint/ContextSnapshot/reconstruction model. `rules` and `attachments` exist as future-ish fields (`:215-229`, `:242-251`) but are not used by server forwarding. |
| App schema/client | `grok-app/lib/effect/schemas.ts`, `connection-manager.ts`, `json-rpc-client.ts` | TypeScript mirrors wire DTOs (`schemas.ts:1-7`, `:117-145`, `:151-220`); RPC methods map directly to server endpoints (`connection-manager.ts:109-164`); client supports request/response and subscriptions only (`json-rpc-client.ts:1-8`, `:119-184`). |
| App UI | `grok-app/App.tsx` | Renders sessions, 11 update kinds, plan approval buttons, diffs, tools, status (`:1-11`, `:257-355`, `:357-470`); active-session stream is kept in React state capped at 220 entries (`:843-874`); no compaction UX beyond ordinary status/errors (`:1178-1205`). |
| Tests | Rust and TS test modules | Tests cover DTO serde, update parsing, diff extraction, discovery smoke, and connection smoke only (`grok-server/src/types.rs:402-446`, `grok-server/src/acp_client.rs:463-501`, `grok-server/src/session.rs:427-446`, `grok-app/lib/effect/schemas.test.ts:10-58`, `grok-app/lib/effect/connection-manager.test.ts:9-47`). No compaction/reconstruction tests. |

## Front-to-back flow actually implemented

1. **App connect/list.** UI auto-connects to `ws://127.0.0.1:8765` and polls `session/list` on the home screen (`grok-app/App.tsx:814-839`). The connection manager maps calls directly to JSON-RPC methods (`grok-app/lib/effect/connection-manager.ts:109-164`).
2. **Create session.** UI sends `session/create` with `cwd`, `plan_mode: true`, `model: grok-build` (`grok-app/App.tsx:946-964`). Server builds `grok agent stdio`, then calls `SessionManager.create_session` (`grok-server/src/server.rs:105-129`).
3. **Spawn inner Grok.** `SessionManager` allocates an in-memory session, channels, and a broadcast; then `GrokBuildConnection::spawn` starts the external ACP agent (`grok-server/src/session.rs:46-107`). `spawn` appends sanitized `-m <model>` and `--permission-mode plan` (`grok-server/src/acp_client.rs:54-75`), initializes ACP, and creates the real Grok session (`grok-server/src/acp_client.rs:140-160`).
4. **Prompt.** UI sends `session/prompt` (`grok-app/App.tsx:979-1001`). Server rejects cancelled sessions, then calls `AppSession.prompt` (`grok-server/src/server.rs:136-165`). `AppSession.prompt` only forwards `params.message` (`grok-server/src/session.rs:391-398`), and ACP sends a `PromptRequest` with one text `ContentBlock` (`grok-server/src/acp_client.rs:175-198`). There is no local context assembly.
5. **Stream updates.** ACP notifications are translated to `UpdateKind` values: message/thought/plan/tool/diff/commands/status (`grok-server/src/acp_client.rs:243-337`). `AppSession.handle_event` updates session status, stores last commands/tools, caps tools, and broadcasts updates (`grok-server/src/session.rs:270-331`). `session/stream` forwards those updates to clients with seq numbers (`grok-server/src/server.rs:278-313`). UI parses, caps to 220, sorts, and renders (`grok-app/App.tsx:857-874`, `:487-495`).
6. **Approvals.** Inner ACP permission requests create an `ApprovalId` and store a responder (`grok-server/src/acp_client.rs:99-107`), then emit `PermissionRequested` (`:125-132`). Session converts that to a pending `PlanStepUpdate` and tracks `pending_approvals` (`grok-server/src/session.rs:341-376`). UI renders five approval buttons (`grok-app/App.tsx:330-348`) and calls `session/approve_step` (`grok-app/App.tsx:1009-1027`), which replies to the stored ACP responder (`grok-server/src/acp_client.rs:201-229`).
7. **Cancel/shutdown.** `session/cancel` marks a session cancelled and aborts the pump; historical cancelled sessions are in-memory only and pruned above 64 (`grok-server/src/session.rs:153-181`, `:261-267`). Shutdown drains/removes all sessions (`grok-server/src/main.rs:90-126`).

## Triggers found vs. not found

Found:

- **Session create trigger:** JSON-RPC `session/create` → external ACP session spawn (`grok-server/src/server.rs:105-129`, `grok-server/src/session.rs:46-107`).
- **Prompt trigger:** `session/prompt` → ACP `PromptRequest` (`grok-server/src/server.rs:136-165`, `grok-server/src/acp_client.rs:175-198`).
- **Plan/approval trigger:** ACP `RequestPermissionRequest` → pending plan step + UI buttons (`grok-server/src/acp_client.rs:99-138`, `grok-server/src/session.rs:341-376`, `grok-app/App.tsx:330-348`).
- **UI stream/status trigger:** session view subscribes and polls status (`grok-app/App.tsx:843-899`).

Not found:

- Token-window threshold, budget estimator, or context-overflow trigger.
- Summary/reconstruction prompts.
- Checkpoint/snapshot persistence trigger.
- Replay/backfill trigger after reconnect.
- Context-pack builder that chooses source artifacts and reconstructs conversation state.

## Algorithms found vs. not found

Found algorithms are transport/UX algorithms, not compaction:

- ACP update mapping: `SessionUpdate::*` → local `UpdateKind` (`grok-server/src/acp_client.rs:243-337`).
- Plan mapping uses `id: format!("plan-{}", e.content.len())`, explicitly only “stable-ish” (`grok-server/src/acp_client.rs:352-383`).
- Diff extraction from ACP tool content (`grok-server/src/acp_client.rs:445-456`).
- Tool cache cap: keep most recent ~40 after len > 50 (`grok-server/src/session.rs:315-331`).
- UI feed cap: keep last 220 notifications (`grok-app/App.tsx:871`).
- Cancelled-session cap: keep up to 64 cancelled IDs, pruned by sorted UUID, not created time (`grok-server/src/session.rs:160-181`).

Not found:

- Reverse compaction algorithm.
- Ranking/selection of old turns, files, diffs, tool calls, plans, or memories for reconstruction.
- Durable snapshot graph or provenance model.
- Validation that reconstructed context matches original conversation/tool state.

## Data model assessment

Implemented model is a live remote-control stream:

- Typed IDs/outcomes/status: `SessionId`, `ApprovalId`, `ApprovalOutcome`, `SessionStatus` (`grok-server/src/types.rs:20-56`, `:198-213`).
- Plan/diff/tool/message/status stream via `UpdateKind` (`grok-server/src/types.rs:63-100`, `:163-194`).
- Request/response DTOs for session lifecycle and streaming (`grok-server/src/types.rs:215-308`).
- Internal `SessionEvent` bridge for translated ACP events and permission requests (`grok-server/src/types.rs:352-368`).

Missing for compaction replacement:

- No `Compaction`, `ContextSnapshot`, `Reconstruction`, `Summary`, `Checkpoint`, `TranscriptSegment`, `TokenBudget`, `ContextPack`, or provenance/hash model.
- `CreateSessionParams.rules` is documented as “system prompt additions” (`grok-server/src/types.rs:224-225`) but `server.create` only uses `cwd`, `model`, `plan_mode`, and `mcp_servers` (`grok-server/src/server.rs:106-115`).
- `PromptParams.attachments` exists (`grok-server/src/types.rs:249-251`) but `send_prompt` ignores attachments and sends only text (`grok-server/src/acp_client.rs:175-198`).

## Prompts

No compaction/reconstruction prompts were found. The only prompt construction is:

- UI sends user-entered `message` (`grok-app/App.tsx:979-1001`).
- Server forwards `PromptParams.message` (`grok-server/src/session.rs:391-398`).
- ACP client wraps that text in `ContentBlock::Text(TextContent::new(text))` and sends `PromptRequest::new(sid, content)` (`grok-server/src/acp_client.rs:175-198`).

There is no prompt template, summarizer prompt, reverse-compaction instruction, or context reconstruction system prompt in either `grok-app` or `grok-server`.

## Session persistence

`grok-stack` persistence is transient:

- Server sessions are held in an in-memory `DashMap` (`grok-server/src/session.rs:30-43`).
- Session entries are removed on shutdown (`grok-server/src/session.rs:194-203`, called from `grok-server/src/main.rs:90-126`).
- Cancelled sessions are retained only as in-memory “historical” entries until prune cap or restart (`grok-server/src/session.rs:153-181`; documented in `grok-server/README.md:17-24`).
- App sessions and feed updates live in React state (`grok-app/App.tsx:803-811`); homescreen list is polled from server (`:827-839`); active feed is capped in memory (`:871`).
- App `recentEndpoints` is an Effect `Ref`, not durable storage (`grok-app/lib/effect/connection-manager.ts:70-78`).
- A schema comment says tests should use real JSON from `~/.grok/sessions/.../updates.jsonl` (`grok-app/lib/effect/schemas.ts:1-7`), and README mentions a “plan file in the Grok session directory” (`grok-server/README.md:77`), but this repo does not read or write those files.

## UI/UX surfacing

Surfaced:

- Homescreen session cards show cwd basename, status, preview, and creation time (`grok-app/App.tsx:257-288`).
- Activity feed renders thoughts, plans, plan-step updates, diffs, tool progress, subagents, status/commands/file changes/messages/errors (`grok-app/App.tsx:292-485`).
- Plan cards expose five approval outcomes (`grok-app/App.tsx:330-348`).
- Header/status bar and pending approval bar show current status and approval count (`grok-app/App.tsx:1150-1186`).
- Error banner for stream/prompt/approve/status failures (`grok-app/App.tsx:1188-1200`).

Not surfaced:

- Compaction start/end/progress.
- Token budget/context-window pressure.
- Reconstruction source list/provenance.
- Persisted checkpoint selection or restore UI.
- Audit view for reconstructed vs. original context.

## Tests

Tests are shallow and do not cover compaction:

- Rust DTO serde and update variants (`grok-server/src/types.rs:402-446`).
- Rust diff extraction helpers and minimal mapping checks (`grok-server/src/acp_client.rs:463-501`).
- Rust session discovery empty-list smoke; comments admit full simulation needs a connection (`grok-server/src/session.rs:427-446`).
- Server discovery/status DTO smoke (`grok-server/src/server.rs:395-421`).
- TS schema parsing all 11 `UpdateKind` variants and malformed thought (`grok-app/lib/effect/schemas.test.ts:10-58`).
- TS connection manager method-existence smoke (`grok-app/lib/effect/connection-manager.test.ts:9-47`).

No tests assert token thresholds, compaction prompts, context reconstruction content, durable checkpoint restore, reconnect backfill, or history persistence.

## Strengths worth copying into Mitsuro

- **Typed event stream boundary.** `UpdateKind`/`SessionEvent` cleanly separates provider/ACP wire shape from UI/server DTOs (`grok-server/src/types.rs:163-194`, `:352-368`). Mitsuro’s replacement should similarly expose typed compaction/reconstruction lifecycle events.
- **Gateway separation.** ACP translation is isolated in `acp_client.rs`, session state in `session.rs`, JSON-RPC in `server.rs`, and UI schemas in `grok-app/lib/effect/schemas.ts`.
- **Live UX patterns.** The app renders a broad typed feed with explicit status/error surfaces and bounded memory (`grok-app/App.tsx:292-495`, `:843-899`, `:1188-1205`). Useful for showing compaction/reconstruction progress and artifacts.
- **Explicit approval flow.** Permission requests become typed pending plan steps with approve/reject outcomes (`grok-server/src/acp_client.rs:99-138`, `grok-server/src/session.rs:341-376`, `grok-app/App.tsx:330-348`). A compaction replacement could reuse this pattern for user-approved context restores or destructive summarization.
- **Bounded in-memory fanout.** Broadcast channels and caps avoid unbounded UI/tool growth (`grok-server/src/session.rs:59-60`, `:315-331`; `grok-app/App.tsx:871`). Mitsuro should copy the bounded/fanout mindset, but add durable persistence.

## Weaknesses / risks

- **No actual compaction implementation.** No triggers, prompts, algorithms, persisted snapshots, or reconstruction models exist in this repo.
- **No durable server persistence.** In-memory `DashMap` and shutdown removal mean reconnect/restart cannot restore session history (`grok-server/src/session.rs:30-43`, `:194-203`).
- **No backfill for stream subscribers.** New `session/stream` subscribers only receive future broadcast events (`grok-server/src/server.rs:278-313`); active UI clears updates when opening a session (`grok-app/App.tsx:973-976`).
- **No auth/ownership.** README explicitly warns of no auth and full IDOR (`grok-server/README.md:58-64`).
- **Unused/ignored context fields.** `rules` and `attachments` are modeled but not forwarded (`grok-server/src/types.rs:224-225`, `:249-251`; `grok-server/src/server.rs:106-115`; `grok-server/src/acp_client.rs:175-198`).
- **Approval ID correlation bug risk.** Permission responders are keyed by generated `ApprovalId` (`grok-server/src/acp_client.rs:102-107`), but the session emits `PlanStep.id` as `tool_call_id` when available (`grok-server/src/session.rs:351-356`), and UI sends `s.id` as `approval_id` (`grok-app/App.tsx:341`, `:1021-1024`). If `tool_call_id` is not the generated approval UUID, `approve_step` will fail to find the responder (`grok-server/src/acp_client.rs:207-229`).
- **Wire schema mismatch risk.** Rust uses `#[serde(rename_all = "camelCase")]` for `CommandInfo`/`ToolInfo` (`grok-server/src/types.rs:119-125`, `:142-151`), while TS schemas/tests expect `input_hint`/`has_diff` snake_case (`grok-app/lib/effect/schemas.ts:89-102`, `grok-app/lib/effect/schemas.test.ts:23-24`). The app’s parse fallback can mask this (`grok-app/App.tsx:864-870`).
- **Per-subscription seq resets.** `session/stream` seq is local to each subscription (`grok-server/src/server.rs:292-301`), not a canonical session event sequence for replay/reconstruction.
- **Cancellation is not a real ACP cancel.** ACP cancel is a stub/drop comment (`grok-server/src/acp_client.rs:232-238`).
- **Tests are mostly smoke tests.** Several tests explicitly avoid full integration (`grok-server/src/acp_client.rs:467-490`, `grok-server/src/session.rs:433-446`).
- **No reconnect/backoff.** Client leaves `TODO: auto-reconnect logic` (`grok-app/lib/effect/json-rpc-client.ts:81-84`).

## Transferable to Mitsuro’s compaction replacement

Transferable scaffolding, not algorithm:

1. **Typed lifecycle events.** Add Mitsuro events analogous to `UpdateKind`, e.g. `CompactionTriggered`, `ContextSnapshotPersisted`, `ReconstructionStarted`, `ReconstructionSourceSelected`, `ReconstructionApplied`, `ReconstructionValidationFailed`.
2. **Separated layers.** Keep provider/tool-history normalization separate from session state and UI/API exposure, as `acp_client.rs`/`session.rs`/`server.rs` do.
3. **Live observability.** Surface compaction/reconstruction as feed/status events with bounded UI retention and clear error banners, copying the app’s typed ActivityFeed approach.
4. **Explicit durable source of truth.** Do the opposite of `grok-stack` persistence: store transcript segments, tool results, summaries, context packs, hashes, and reconstruction decisions durably in Mitsuro core storage.
5. **Canonical event sequence.** Use a session-level durable event sequence, not per-subscription seq, so reconnect/backfill and reconstruction audits are possible.
6. **Approval/governance hook.** The plan approval pattern is useful for asking users to approve a risky reconstruction or summary discard, but fix ID correlation and store policy decisions durably.
7. **Model/data contract first.** Define explicit `ContextSnapshot`/`ReconstructionPlan`/`ReconstructionResult` DTOs and tests; no equivalent exists here.

Not transferable:

- Compaction trigger logic.
- Reverse-compaction prompts.
- Context reconstruction algorithms.
- Persisted session restore/backfill design.
- Token accounting/budget management.

## Recommendation for Mitsuro

Use `grok-stack` as a cautionary gateway/UI example, not as a compaction reference. For Mitsuro’s compaction replacement, design the missing pieces explicitly: durable session-event storage, token-budget triggers, prompt templates, context-pack/reconstruction data models, provenance/audit hashes, replay/backfill APIs, and tests that verify reconstruction quality and safety across restart/reconnect.