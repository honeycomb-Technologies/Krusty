# Krusty Post-Roadmap Verification Tracker

Last updated: 2026-03-09
Program state: Complete

Reference plan: `docs/KRUSTY_POST_ROADMAP_VERIFICATION_PLAN.md`

## Phase Status

| Phase | Name | Status | Entry met | Exit met |
| --- | --- | --- | --- | --- |
| 1 | Core Watchdog Audit | Complete | Yes | Yes |
| 2 | Server/API Audit | Complete | Yes | Yes |
| 3 | Surface Parity Audit | Complete | Yes | Yes |
| 4 | Workload and Replay Validation | Complete | Yes | Yes |
| 5 | Security and Operations Audit | Complete | Yes | Yes |
| 6 | Release Confidence Program | Complete | Yes | Yes |

## Current Watchpoints

| Area | Current state | Phase owner |
| --- | --- | --- |
| Core control paths | Backcheck passed | 1 |
| Server contract fidelity | Backcheck passed | 2 |
| Surface parity | Backcheck passed | 3 |
| Workload replay confidence | Backcheck passed | 4 |
| Security and operational confidence | Backcheck passed | 5 |
| Release confidence process | Backcheck passed | 6 |

## Phase 1 Kickoff

Primary scope:
- orchestrator
- stream/recovery
- compaction
- tool failure containment
- planning lifecycle
- subagent containment
- runtime traces

## Phase 1 Resolution

1. Resolved: streamed usage events now preserve the true prompt/completion split in `LoopEvent::Usage`, while `StreamResult.total_tokens` keeps using the canonical total from provider usage when present. Fix and regression coverage live in [stream.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/stream.rs).

2. Resolved: repeated-failure containment now clears only the recovered tool signature instead of wiping the entire failure map, so unrelated persistent failures still trip fail-fast. Fix and regression coverage live in [failure.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/failure.rs).

3. Resolved: limited runtime trace retrieval now selects the most recent events and returns them in chronological order, so the session trace endpoint reflects live diagnostics correctly. Fix and regression coverage live in [runtime_traces.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/runtime_traces.rs).

## Phase 1 Current Assessment

- Architecture: core ownership boundaries still look solid.
- Behavior: previously identified defects are closed and regression-covered.
- Recommendation: advance to Phase 2 and audit the server/API transport layer against the hardened core semantics.

## Phase 1 Backcheck Evidence

- `cargo fmt --all`
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cargo test -p krusty-core usage_event_preserves_prompt_completion_split -- --nocapture`
- `cargo test -p krusty-core success_only_clears_matching_signature -- --nocapture`
- `cargo test -p krusty-core runtime_trace_store_limit_returns_most_recent_events_in_order -- --nocapture`
- `cargo test -p krusty-core runtime_trace_forwarder_persists_and_forwards_events -- --nocapture`
- `cargo test -p krusty-core test_runtime_traces_table_exists -- --nocapture`

Advance decision: `Go`

## Phase 3 Findings And Resolution

1. Resolved in code: the TUI reloaded stored conversations after stream completion without preserving the `tool` role, even though full session load handled it correctly. Stored-role mapping is now shared in [sessions.rs](/home/burgess/Work/krusty/crates/krusty-cli/src/tui/handlers/sessions.rs) and reused in [stream_events.rs](/home/burgess/Work/krusty/crates/krusty-cli/src/tui/handlers/stream_events.rs).

2. Resolved in code: ACP session restore only treated persisted messages as proof that a session existed, even though recovery state now lives separately from conversation history. ACP load now restores sessions that have stored recovery or session metadata in [agent.rs](/home/burgess/Work/krusty/crates/krusty-core/src/acp/agent.rs).

3. Resolved in code: the PWA session store ignored persisted recovery state, undercounted usage by treating `prompt_tokens` as total context usage, and marked stored tool calls as `success` even when no tool result existed. Those parity fixes are in [client.ts](/home/burgess/Work/krusty/apps/pwa/app/src/lib/api/client.ts) and [session.ts](/home/burgess/Work/krusty/apps/pwa/app/src/lib/stores/session.ts).

4. Resolved in code: when the PWA was reopened during `awaiting_input`, approving or denying a tool request did not reattach any observation path to the resumed run. Approval actions now re-enter state polling in [session.ts](/home/burgess/Work/krusty/apps/pwa/app/src/lib/stores/session.ts).

5. Resolved in code: frontend validation also exposed a missing label association in [ChatHeader.svelte](/home/burgess/Work/krusty/apps/pwa/app/src/lib/components/chat/ChatHeader.svelte) and a static diff-renderer import that was better owned as a lazy client-side dependency in [ToolWidget.svelte](/home/burgess/Work/krusty/apps/pwa/app/src/lib/components/chat/ToolWidget.svelte). The label is now correctly associated, and the diff renderer is dynamically imported.

## Phase 3 Current Assessment

- Architecture: the main surface-semantic drift points have been corrected without moving behavior ownership out of core/server.
- Behavior: TUI and ACP parity fixes are regression-covered on the Rust side, and the PWA logic now has successful type/build validation against the corrected recovery/approval/token semantics.
- Tooling: Bun is installed locally at `/home/burgess/.bun/bin/bun`; this shell just does not expose it on `PATH`, so frontend validation must use the absolute binary path or a corrected shell environment.
- Remaining non-blocking notes: none.

## Phase 3 Backcheck Evidence

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- `cargo test -p krusty tui::handlers::sessions::tests::storage_role_mapping_preserves_tool_role -- --nocapture`
- `cargo test -p krusty-core load_session_restores_recovery_only_storage_session -- --nocapture`
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun install --frozen-lockfile`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run check`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run build`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run check` after the `ChatHeader.svelte` and `ToolWidget.svelte` cleanup
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run build` after the `ChatHeader.svelte` and `ToolWidget.svelte` cleanup

## Phase 3 Backcheck

1. Architecture backcheck: passed. Surface fixes stayed inside adapters at [sessions.rs](/home/burgess/Work/krusty/crates/krusty-cli/src/tui/handlers/sessions.rs), [stream_events.rs](/home/burgess/Work/krusty/crates/krusty-cli/src/tui/handlers/stream_events.rs), [agent.rs](/home/burgess/Work/krusty/crates/krusty-core/src/acp/agent.rs), [client.ts](/home/burgess/Work/krusty/apps/pwa/app/src/lib/api/client.ts), and [session.ts](/home/burgess/Work/krusty/apps/pwa/app/src/lib/stores/session.ts) without moving semantic ownership out of core/server.

2. Behavior backcheck: passed. TUI reload now preserves `tool` roles, ACP restores recovery-only sessions, and the PWA now restores recovery state, resumes observation after approval, and stops misclassifying orphaned tool calls or usage totals.

3. Validation backcheck: passed. Rust regressions succeeded, `svelte-check` completed with zero errors, and the production build completed successfully once Bun was executed through its installed absolute path with writable temp/install directories.

4. Confidence backcheck: passed. The frontend cleanup pass closed the remaining accessibility, lazy-loading, and noisy bundle-warning issues, leaving no unresolved Phase 3 diagnostic.

Advance decision: `Go`

## Phase 4 Kickoff

Primary scope:
- long coding sessions
- compaction edge cases
- repeated tool loops
- provider interruptions
- delegated builder/explore flows
- recovery after interruption

## Phase 4 Findings And Resolution

1. Resolved in code: Phase 4 did not have a representative workload pack even though runtime traces and replay gates already existed. The replay gate in [runtime_traces.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/runtime_traces.rs) now supports workload-shape assertions for minimum runs, turns, compactions, awaiting-input transitions, and required event types.

2. Resolved in code: resumed sessions could only be evaluated as whole-session trace aggregates, which permanently mixed prior interrupted runs into the recovery verdict. [runtime_traces.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/runtime_traces.rs) now exposes latest-run summarization so replay validation can judge the recovered attempt directly when that is the relevant contract.

3. Resolved in code: representative replay scenarios for long sessions, approval pause/resume, interruption recovery, and loop-guard rejection were missing. Those workload tests now live in [runtime_traces.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/runtime_traces.rs).

4. Resolved in code: the final frontend cleanup before Phase 4 closure removed the last surface-level diagnostics by fixing label wiring in [ChatHeader.svelte](/home/burgess/Work/krusty/apps/pwa/app/src/lib/components/chat/ChatHeader.svelte), lazy-loading the diff renderer in [ToolWidget.svelte](/home/burgess/Work/krusty/apps/pwa/app/src/lib/components/chat/ToolWidget.svelte), and aligning intentional build warning thresholds in [vite.config.ts](/home/burgess/Work/krusty/apps/pwa/app/vite.config.ts).

## Phase 4 Current Assessment

- Architecture: workload validation stayed attached to the existing runtime trace seam instead of inventing a second replay subsystem.
- Behavior: representative long-session, compaction, interruption, approval/resume, and loop-guard scenarios are now concretely gated.
- Recommendation: advance to Phase 5 and audit security/operational boundaries with the replay pack retained as regression evidence.

## Phase 4 Backcheck Evidence

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- `cargo test -p krusty-core runtime_trace -- --nocapture`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run check`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run build`

## Phase 4 Backcheck

1. Architecture backcheck: passed. Replay/workload validation is still owned by the canonical runtime trace layer in [runtime_traces.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/runtime_traces.rs), not by duplicated server or surface-specific test logic.

2. Behavior backcheck: passed. The workload pack now proves long-session compaction, approval pause/resume, interruption recovery, and loop-guard rejection on structured loop events instead of ad hoc inspection.

3. Parity backcheck: passed. The replay gate continues to operate on canonical `LoopEvent`s, so the same evidence applies regardless of whether the session was driven by CLI, TUI, ACP, or server.

4. Deletion backcheck: passed. No duplicate replay harness was added; the trace store and gate were extended in place.

5. Confidence backcheck: passed. Phase 4 now has a durable workload pack and a latest-run summary contract, which closes the biggest verification gap left after the roadmap.

Advance decision: `Go`

## Phase 5 Kickoff

Primary scope:
- sandbox boundaries
- destructive tool approval behavior
- multi-tenant session ownership
- credential handling
- rollback and operator visibility

## Phase 5 Findings And Resolution

1. Resolved in code: the server accepted external API requests as if they were local single-tenant traffic, even though it binds `0.0.0.0` and exposes tool/chat/session surfaces. [auth.rs](/home/burgess/Work/krusty/crates/krusty-server/src/auth.rs) now fails closed for non-loopback requests instead of silently accepting them.

2. Resolved in code: the terminal WebSocket sat outside the API auth middleware, leaving an unaudited remote command surface. [lib.rs](/home/burgess/Work/krusty/crates/krusty-server/src/lib.rs) now places `/ws/terminal` behind the same request gate as `/api`.

3. Resolved in code: `X-Workspace-Dir` could widen the effective filesystem root because the auth layer copied it directly into user context. [auth.rs](/home/burgess/Work/krusty/crates/krusty-server/src/auth.rs) now scopes requested workspace paths back to the configured server root before they can reach file, git, or tool routes.

4. Audited: credential persistence already used atomic writes and restrictive Unix permissions in [credentials.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/credentials.rs), so no additional credential-storage change was needed in this phase.

## Phase 5 Current Assessment

- Architecture: the trust boundary is now enforced once at the server edge instead of route-by-route assumptions.
- Behavior: external API/terminal access fails closed, and workspace scoping cannot be widened by request headers.
- Recommendation: advance to Phase 6 and turn the verified gates into an explicit release-confidence checklist and maintenance rule set.

## Phase 5 Backcheck Evidence

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- `cargo test -p krusty-server auth -- --nocapture`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run check`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run build`

## Phase 5 Backcheck

1. Architecture backcheck: passed. The security boundary is now centralized in [auth.rs](/home/burgess/Work/krusty/crates/krusty-server/src/auth.rs) and router composition in [lib.rs](/home/burgess/Work/krusty/crates/krusty-server/src/lib.rs), not duplicated across individual routes.

2. Behavior backcheck: passed. Non-loopback API and terminal access now fail closed, and workspace headers can no longer escape the configured server root.

3. Parity backcheck: passed. The same local-only trust model now applies consistently to both REST and terminal WebSocket surfaces.

4. Deletion backcheck: passed. No second auth mechanism was added; the existing lightweight middleware was hardened in place.

5. Confidence backcheck: passed. The largest remaining operational/security gap from the audit is closed and regression-covered.

Advance decision: `Go`

## Phase 6 Kickoff

Primary scope:
- release checklist
- replay ownership
- regression gating expectations
- operator runbooks
- intentional-delta review

## Phase 6 Findings And Resolution

1. Resolved in documentation/process: the verification work needed a durable ship gate, not just one-time successful runs. The release-confidence rules now live in [KRUSTY_RELEASE_CONFIDENCE_PROGRAM.md](/home/burgess/Work/krusty/docs/KRUSTY_RELEASE_CONFIDENCE_PROGRAM.md).

2. Resolved in documentation/process: replay ownership and go/no-go criteria are now explicit instead of implicit. The release program defines validation commands, replay expectations, operator response steps, and maintenance rules.

3. Resolved in documentation/process: intentional architecture deltas are now part of release review through a direct link back to [COMPARISON.md](/home/burgess/Work/krusty/crates/krusty-core/COMPARISON.md), so future releases do not silently reintroduce drift.

## Phase 6 Current Assessment

- Architecture: release confidence is now anchored to the same canonical runtime seams that were hardened in Phases 1-5.
- Behavior: future releases have explicit gates instead of relying on ad hoc human memory.
- Recommendation: verification program is complete; future work should use the release-confidence program as the standing operating rule set.

## Phase 6 Backcheck Evidence

- Created [KRUSTY_RELEASE_CONFIDENCE_PROGRAM.md](/home/burgess/Work/krusty/docs/KRUSTY_RELEASE_CONFIDENCE_PROGRAM.md)
- Verified the document points back to the active tracker and canonical comparison baseline
- Phase 5 validation set remained green immediately before Phase 6 closure

## Phase 6 Backcheck

1. Architecture backcheck: passed. The release process points back to canonical core/server/runtime-trace seams instead of introducing a parallel governance layer.

2. Behavior backcheck: passed. The checklist and runbook now preserve the exact no-warning, replay-backed discipline used during this verification program.

3. Parity backcheck: passed. The release rules apply across CLI, TUI, ACP, server, and PWA because they depend on shared validation and replay gates.

4. Deletion backcheck: passed. No extra release bureaucracy was added beyond a single focused program document and the tracker.

5. Confidence backcheck: passed. Future go/no-go decisions now have explicit, repeatable criteria.

Advance decision: `Complete`

## Phase 2 Findings And Resolution

1. Resolved: session creation in the REST session route was not binding authenticated ownership, so multi-tenant sessions created outside `/chat` could become unowned transport artifacts. The route now persists `user_id` through the canonical storage path in [sessions.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/sessions.rs).

2. Resolved: session read and pinch routes were loading by existence only, which allowed cross-user reads or pinch operations when a session ID was known. Those handlers now use the shared ownership guard path in [sessions.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/sessions.rs).

3. Resolved: linked session creation did not preserve parent ownership, so pinch continuations could fall out of multi-tenant isolation. Child sessions now inherit parent `user_id` in [sessions.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/sessions.rs).

4. Resolved: the chat approval endpoint trusted only an active `session_id`, which let foreign clients approve another user’s pending tool request if they knew the ID. Ownership is now enforced before forwarding approval input in [chat.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/chat.rs).

5. Resolved: the direct tool execution route accepted arbitrary working-directory overrides without scoping them back to the same allowed root used by the rest of the server path surfaces. Working-directory resolution is now root-validated in [tools.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/tools.rs).

## Phase 2 Current Assessment

- Architecture: `krusty-server` is now acting more cleanly as a transport layer over core ownership and tool-governance semantics instead of silently inventing exceptions.
- Behavior: the major multi-tenant drift points in session CRUD, pinch continuation, approval forwarding, and direct tool path resolution are closed and regression-covered.
- Recommendation: advance to Phase 3 and compare CLI/TUI/ACP/PWA/Desktop semantics against the corrected server/core contract.

## Phase 2 Backcheck Evidence

- `cargo fmt --all`
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cargo test -p krusty-core test_create_linked_session_preserves_parent_user_id -- --nocapture`
- `cargo test -p krusty-server create_session_persists_user_ownership -- --nocapture`
- `cargo test -p krusty-server get_session_rejects_foreign_owner -- --nocapture`
- `cargo test -p krusty-server tool_approval_rejects_foreign_owner -- --nocapture`
- `cargo test -p krusty-server resolve_tool_working_dir_rejects_paths_outside_user_root -- --nocapture`
- `cargo test -p krusty-server resolve_tool_working_dir_allows_relative_paths_within_user_root -- --nocapture`

## Phase 2 Backcheck

1. Architecture backcheck: passed. The server remains a thin transport layer. Ownership enforcement stays at route boundaries, while linked-session ownership persistence is correctly owned by storage in [sessions.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/sessions.rs), not route-local shadow state.

2. Behavior backcheck: passed. Session CRUD, pinch continuation, tool approval, and direct tool working-directory resolution now fail closed for foreign or out-of-scope access in [sessions.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/sessions.rs), [chat.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/chat.rs), and [tools.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/tools.rs).

3. Parity backcheck: passed. Server session routes now obey the same ownership rules already assumed by chat setup and session state/trace endpoints, so the API surface is more internally consistent instead of having weaker side doors.

4. Deletion backcheck: passed with no new deletion required. The fixes tightened existing helpers and storage ownership flow without adding a second policy abstraction or duplicate transport path.

5. Confidence backcheck: passed. Phase 2 now has direct regression coverage for the exact ownership and path-scope seams that were previously drifting.

Advance decision: `Go`
