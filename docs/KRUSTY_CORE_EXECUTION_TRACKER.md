# Krusty Core Execution Tracker

Last updated: 2026-03-09
Program state: Roadmap complete

Reference roadmap: `docs/KRUSTY_CORE_BEST_IN_CLASS_ROADMAP.md`

## Phase Status

| Phase | Name | Status | Entry met | Exit met |
| --- | --- | --- | --- | --- |
| 0 | Program Definition and Baseline | Complete | Yes | Yes |
| 1 | Context Engine and Deterministic Continuation | Complete | Yes | Yes |
| 2 | Canonical AI Execution Pipeline | Complete | Yes | Yes |
| 3 | Tool and Sandbox Maturity | Complete | Yes | Yes |
| 4 | Persistence, Recovery, and Surface Parity | Complete | Yes | Yes |
| 5 | Subagents and Extensibility Unification | Complete | Yes | Yes |
| 6 | Planning and Execution Discipline | Complete | Yes | Yes |
| 7 | Observability, Replay, and Evaluations | Complete | Yes | Yes |
| 8 | Elegance and Deletion Pass | Complete | Yes | Yes |
| 9 | Final Competitive Audit and Closure | Complete | Yes | Yes |

## Current Scorecard (Core Domains)

Legend: `Done`, `Partial`, `Not started`

| Domain | Current | Target | Phase owner |
| --- | --- | --- | --- |
| Orchestration loop | Done | Deterministic state machine with recoverable interruptions | 1, 2 |
| Context ledger and compaction continuity | Done | Full ledger + pinned/replay invariants | 1 |
| Prompt system | Done | Canonical prompt pack pipeline across all AI surfaces | 2 |
| Provider/model normalization | Done | Single canonical policy pipeline + capability registry | 2 |
| Tool policy and sandbox matrix | Done | Full policy matrix with auditable outcomes | 3 |
| Tool evidence contracts | Done | Exhaustive high-impact tool contracts | 3 |
| Persistence and resume | Done | Crash-safe partial-turn and deterministic resume | 4 |
| CLI/TUI/ACP/server parity | Done | Semantic parity across entrypoints | 4 |
| Subagent governance | Done | Unified inheritance, quotas, containment | 5 |
| MCP/skills/plugins/extensions governance | Done | Unified execution and policy model | 5 |
| Plan/task continuity | Done | Durable and consistent lifecycle across resumes | 6 |
| Observability + evals | Done | Replay-backed quality and reliability gating | 7 |
| Complexity cleanup | Done | Reduced abstraction count with no capability loss | 8 |
| Competitive parity audit | Done | Parity/advantage across all core domains | 9 |

## Phase 0 Checklist (Started)

| Task | Status | Proof |
| --- | --- | --- |
| Define master roadmap and phase gates | Done | `docs/KRUSTY_CORE_BEST_IN_CLASS_ROADMAP.md` |
| Define cross-domain scorecard with target states | Done | This file (`Current Scorecard`) |
| Define backcheck template | Done | Roadmap section `Backcheck` |
| Define phase entry/exit criteria | Done | Roadmap section `Phase Plan` |
| Define risk register | Done | This file section `Risk Register (Initial)` |
| Define rollback policy template | Done | This file section `Rollback Policy Template` |

## Phase 0 Backcheck Record

Phase: 0
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: Master roadmap created with canonical phase ownership and no duplicate program tracks.

2. Behavior backcheck:
- Result: Pass
- Evidence: Phase gates require deterministic pass/fail evidence before advancement.

3. Parity backcheck:
- Result: Pass
- Evidence: Scorecard explicitly includes CLI/TUI/ACP/server parity domain.

4. Deletion backcheck:
- Result: Pass
- Evidence: Deletion pass is mandatory phase with explicit exit criteria.

5. Competitive backcheck:
- Result: Pass
- Evidence: Final phase requires explicit cross-product parity/advantage audit.

Advance decision: `Proceed`
Blocking issues: None

## Phase 1 Completion Record

Delivered artifacts:
- `crates/krusty-core/src/agent/context_ledger.rs` (ledger + continuation contract + tests)
- `crates/krusty-core/src/agent/orchestrator.rs` (ledger integration + persisted continuation state + interruption guidance)
- `crates/krusty-core/src/storage/database.rs` (schema migration 17 for continuation persistence)
- `crates/krusty-core/src/storage/sessions.rs` (context/continuation persistence load/save APIs + tests)
- `crates/krusty-core/src/agent/compaction.rs` (pinned-system-context preservation test)

Validation evidence:
- `cargo check -p krusty-core -p krusty`
- `cargo test -p krusty-core context_ledger -- --nocapture`
- `cargo test -p krusty-core test_context_continuation_state_round_trip -- --nocapture`
- `cargo test -p krusty-core test_database_creation -- --nocapture`
- `cargo test -p krusty-core test_schema_version_increments -- --nocapture`
- `cargo test -p krusty-core test_sessions_table_exists -- --nocapture`
- `cargo test -p krusty-core compaction_preserves_pinned_system_context -- --nocapture`

## Phase 1 Backcheck Record

Phase: 1
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: Context identity and continuation decisions now live in explicit ledger contracts instead of implicit branch logic.

2. Behavior backcheck:
- Result: Pass
- Evidence: Compaction/interruption paths emit deterministic continuation outcomes and persist them to session state.

3. Parity backcheck:
- Result: Pass
- Evidence: Continuation contract is persisted at core session layer and available uniformly to all surfaces via `SessionManager`.

4. Deletion backcheck:
- Result: Pass
- Evidence: Removed ambiguity fallback by replacing implicit resume hints with typed contract payloads.

5. Competitive backcheck:
- Result: Pass
- Evidence: Live compaction now preserves pinned instructions and explicit resume intent, reducing silent context drift.

Advance decision: `Proceed`
Blocking issues: None

## Phase 2 Kickoff

Primary execution spec:
- `docs/phases/PHASE2_CANONICAL_AI_PIPELINE_SPEC.md`

## Phase 2 Completion Record

Delivered artifacts:
- `crates/krusty-core/src/ai/client/core.rs` (shared canonical options + prompt-section helpers)
- `crates/krusty-core/src/ai/client/streaming.rs` (streaming path bound to canonical seam)
- `crates/krusty-core/src/ai/client/simple.rs` (simple + conversation calls bound to canonical seam)
- `crates/krusty-core/src/ai/client/tools.rs` (subagent tool calls canonicalized)
- `crates/krusty-core/src/ai/client/thinking.rs` (thinking helper bound to canonical seam)
- `crates/krusty-core/src/ai/client/config.rs` (provider/model normalization tests)
- `crates/krusty-core/src/ai/models.rs` (exact metadata resolution before heuristic fallback)

Validation evidence:
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo test -p krusty-core canonicalization_aligns_reasoning_controls_with_model_family -- --nocapture`
- `cargo test -p krusty-core canonicalization_preserves_builtin_reasoning_models -- --nocapture`
- `cargo test -p krusty-core resolves_builtin_reasoning_metadata_before_fallback -- --nocapture`

## Phase 2 Backcheck Record

Phase: 2
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: AI request normalization now routes through shared `AiClient` helpers instead of separate surface-specific option shaping.

2. Behavior backcheck:
- Result: Pass
- Evidence: Streaming, simple, conversation, thinking, and subagent tool calls all enforce provider/model capability normalization before dispatch.

3. Parity backcheck:
- Result: Pass
- Evidence: TUI, server, ACP, title generation, summarization, and subagents now converge on the same core normalization seam.

4. Deletion backcheck:
- Result: Pass
- Evidence: Reduced duplicated prompt-section and option-building logic by moving it into the client core helper layer.

5. Competitive backcheck:
- Result: Pass
- Evidence: Canonical provider/model handling closes a major drift gap versus professional agents that keep one request policy path across surfaces.

Advance decision: `Proceed`
Blocking issues: None

## Phase 3 Kickoff

Primary execution spec:
- `docs/phases/PHASE3_TOOL_POLICY_SPEC.md`

## Phase 3 Completion Record

Delivered artifacts:
- `crates/krusty-core/src/tools/registry.rs` (canonical `ToolPolicy` contract)
- `crates/krusty-core/src/agent/tool_control.rs` (approval/retry rules driven by policy contract)
- `crates/krusty-core/src/agent/hooks.rs` (plan-mode write blocking derived from policy contract)
- `crates/krusty-core/src/agent/failure.rs` (read-only loop detection aligned to shared policy layer)

Validation evidence:
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo test -p krusty-core test_tool_policy_contracts -- --nocapture`
- `cargo test -p krusty-core approval_only_required_for_supervised_write_tools -- --nocapture`
- `cargo test -p krusty-core retries_only_read_only_timeouts_once -- --nocapture`
- `cargo test -p krusty-core plan_mode_blocks_write_category_tool -- --nocapture`
- `cargo test -p krusty-core repeated_read_only_sequence_trips_threshold -- --nocapture`
- `cargo test -p krusty-core repeated_read_only_sequence_resets_on_write -- --nocapture`

## Phase 3 Backcheck Record

Phase: 3
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: Tool execution behavior now derives from explicit `ToolPolicy` metadata rather than scattered raw category checks.

2. Behavior backcheck:
- Result: Pass
- Evidence: Approval, retry, plan-mode blocking, and read-only loop detection all route through the same policy source.

3. Parity backcheck:
- Result: Pass
- Evidence: Core orchestrator tool flow and plan-mode hooks consume the same policy definitions.

4. Deletion backcheck:
- Result: Pass
- Evidence: Removed duplicated write/read-only decisions from multiple agent modules by centralizing policy contracts.

5. Competitive backcheck:
- Result: Pass
- Evidence: Krusty now has explicit, auditable tool-control semantics closer to professional agent policy engines.

Advance decision: `Proceed`
Blocking issues: None

## Phase 4 Kickoff

Primary execution spec:
- `docs/phases/PHASE4_RECOVERY_PARITY_SPEC.md`

## Phase 4 Completion Record

Delivered artifacts:
- `crates/krusty-core/src/storage/recovery.rs` (typed recovery snapshot contract + notice helpers + tests)
- `crates/krusty-core/src/storage/database.rs` (schema migration 18 for `recovery_json`)
- `crates/krusty-core/src/storage/sessions.rs` (recovery load/save/clear APIs + roundtrip tests)
- `crates/krusty-core/src/agent/stream.rs` (stream checkpoint capture for partial-turn recovery)
- `crates/krusty-core/src/agent/orchestrator.rs` (crash-safe recovery persistence, no partial assistant commits on interrupted turns)
- `crates/krusty-server/src/routes/sessions.rs` and `crates/krusty-server/src/types.rs` (typed recovery returned from session state endpoint)
- `crates/krusty-cli/src/tui/handlers/sessions.rs` and `crates/krusty-cli/src/tui/handlers/stream_events.rs` (recovery notices sourced from persisted recovery state)
- `crates/krusty-core/src/acp/session.rs` and `crates/krusty-core/src/acp/processor.rs` (one-shot ACP recovery notice on resumed prompts)

Validation evidence:
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cargo test -p krusty-core recovery -- --nocapture`
- `cargo test -p krusty-core test_recovery_state_round_trip -- --nocapture`
- `cargo test -p krusty-core test_session_load_from_storage -- --nocapture`
- `cargo test -p krusty-core test_database_creation -- --nocapture`
- `cargo test -p krusty-core test_schema_version_increments -- --nocapture`
- `cargo test -p krusty-core test_sessions_table_exists -- --nocapture`

## Phase 4 Backcheck Record

Phase: 4
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: Interrupted-turn state now has a dedicated typed storage contract instead of being inferred from canonical messages or surface-local heuristics.

2. Behavior backcheck:
- Result: Pass
- Evidence: Mid-stream failures persist partial assistant recovery state while preventing partial assistant output from being written into canonical conversation history.

3. Parity backcheck:
- Result: Pass
- Evidence: Server state responses, TUI recovery banners/session loads, and ACP resumed prompts all consume the same persisted `SessionRecoveryState`.

4. Deletion backcheck:
- Result: Pass
- Evidence: TUI fallback recovery messaging is now secondary to the shared persisted recovery contract, removing surface-only recovery logic as the primary source of truth.

5. Competitive backcheck:
- Result: Pass
- Evidence: Krusty now has explicit safe-resume vs non-resumable semantics closer to professional coding agents that separate interrupted work state from durable thread history.

Advance decision: `Proceed`
Blocking issues: None

## Phase 5 Kickoff

Primary execution spec:
- `docs/phases/PHASE5_DELEGATION_GOVERNANCE_SPEC.md`

## Phase 5 Progress Record (Slice 1)

Delivered artifacts:
- `crates/krusty-core/src/tools/registry.rs` (typed `DelegationPolicy` contract + `ToolContext` inheritance fields)
- `crates/krusty-core/src/agent/executor.rs` (parent permission mode + subagent turn budget inheritance into tool context)
- `crates/krusty-core/src/agent/subagent/types.rs` (delegation policy + max-turn override on tasks; policy violation evidence on results)
- `crates/krusty-core/src/agent/subagent/execution.rs` (delegated policy enforcement and containment on repeated blocked calls)
- `crates/krusty-core/src/tools/implementations/explore.rs` (inherited delegated policy on spawned explore agents + audit metadata)
- `crates/krusty-core/src/tools/implementations/build.rs` (inherited delegated policy on spawned builders + audit metadata)
- `crates/krusty-core/src/mcp/tool.rs` (delegated governance metadata normalization for remote MCP execution results)

Validation evidence:
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cargo test -p krusty-core test_tool_policy_contracts -- --nocapture`
- `cargo test -p krusty-core delegated_explore_policy_blocks_write_tools -- --nocapture`
- `cargo test -p krusty-core delegated_build_policy_blocks_supervised_write_without_approval_path -- --nocapture`
- `cargo test -p krusty-core delegated_build_policy_allows_autonomous_write -- --nocapture`

## Phase 5 Completion Record

Delivered artifacts:
- `crates/krusty-core/src/agent/subagent/execution.rs` (shared delegated turn-budget resolution, runtime tool-context inheritance, and targeted tests)
- `crates/krusty-core/src/tools/implementations/skill.rs` (delegated governance metadata normalization for skill execution)
- `crates/krusty-core/src/acp/processor.rs` (autonomous direct tool execution surface now inherits delegated turn budget)
- `crates/krusty-server/src/routes/tools.rs` (server direct tool execution now inherits autonomous policy, delegated turn budget, and extensibility managers)
- `crates/krusty-core/src/AGENTS.md` and `crates/krusty-server/src/routes/AGENTS.md` (guardrails for delegated-governance resolution at runtime surfaces)

Validation evidence:
- `cargo fmt --all`
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cargo test -p krusty-core delegated_turn_budget_prefers_task_override_then_policy_then_runtime_default -- --nocapture`
- `cargo test -p krusty-core build_subagent_tool_context_inherits_delegated_policy_contract -- --nocapture`
- `cargo test -p krusty-core delegated_explore_policy_blocks_write_tools -- --nocapture`
- `cargo test -p krusty-core delegated_build_policy_blocks_supervised_write_without_approval_path -- --nocapture`
- `cargo test -p krusty-core delegated_build_policy_allows_autonomous_write -- --nocapture`
- `cargo test -p krusty-core skill_tool_returns_governance_metadata -- --nocapture`

## Phase 5 Backcheck Record

Phase: 5
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: Delegated permission mode and turn budget now resolve from one shared contract across orchestrated tools, subagent runtime, ACP direct tool execution, and server tool execution.

2. Behavior backcheck:
- Result: Pass
- Evidence: Subagents enforce delegated policy with containment on repeated violations, and direct execution surfaces no longer default to an implicit supervised context that can self-block delegated builders.

3. Parity backcheck:
- Result: Pass
- Evidence: TUI/orchestrator, ACP, and server direct tool execution now share the same delegated-governance defaults for permission mode and subagent turn budget; MCP and skill results expose normalized governance metadata.

4. Deletion backcheck:
- Result: Pass
- Evidence: Removed drift-prone duplicated turn-budget resolution by centralizing subagent budget inheritance in the runtime helper path.

5. Competitive backcheck:
- Result: Pass
- Evidence: Krusty now has explicit delegated-governance inheritance and auditability closer to professional agents that prevent silent subagent/tool-surface policy divergence.

Advance decision: `Proceed`
Blocking issues: None

## Phase 6 Kickoff

Primary execution spec:
- `docs/phases/PHASE6_PLAN_DISCIPLINE_SPEC.md`

## Phase 6 Completion Record

Delivered artifacts:
- `crates/krusty-core/src/plan/lifecycle.rs` (canonical active-plan filtering + effective work-mode resolution + tests)
- `crates/krusty-core/src/plan/manager.rs` (active plan and lifecycle helpers)
- `crates/krusty-core/src/agent/context.rs` and `crates/krusty-core/src/agent/plan_handler.rs` (runtime/context consumers now use active plans only)
- `crates/krusty-core/src/agent/executor.rs` (plan task mutations emit explicit `PlanUpdate` events)
- `crates/krusty-core/src/agent/subagent/types.rs` and `crates/krusty-core/src/agent/subagent/execution.rs` (delegated completion summaries carried in progress events)
- `crates/krusty-cli/src/tui/app.rs`, `crates/krusty-cli/src/tui/handlers/sessions.rs`, `crates/krusty-cli/src/tui/handlers/keyboard.rs`, `crates/krusty-cli/src/tui/handlers/stream_events.rs`, `crates/krusty-cli/src/tui/handlers/streaming/mod.rs`, and `crates/krusty-cli/src/tui/polling/blocks.rs` (persisted TUI mode transitions, canonical resume, explicit plan updates, no prose-driven completion)
- `crates/krusty-server/src/routes/chat.rs` and `crates/krusty-server/src/routes/sessions.rs` (canonical work-mode recovery and active-plan pinch carry-forward)

Validation evidence:
- `cargo fmt --all`
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cargo test -p krusty-core in_progress_plan_with_started_work_repairs_stale_plan_mode -- --nocapture`
- `cargo test -p krusty-core lifecycle_state_filters_archived_plans_from_active_runtime_state -- --nocapture`
- `cargo test -p krusty-core test_get_active_plan_filters_completed_plan -- --nocapture`
- `cargo test -p krusty-core delegated_turn_budget_prefers_task_override_then_policy_then_runtime_default -- --nocapture`

## Phase 6 Backcheck Record

Phase: 6
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: Plan lifecycle and effective work mode now resolve from shared core helpers instead of separate UI/server heuristics.

2. Behavior backcheck:
- Result: Pass
- Evidence: Archived plans no longer re-enter live context, TUI mode changes persist, and assistant prose alone can no longer mark plan tasks complete.

3. Parity backcheck:
- Result: Pass
- Evidence: Core context injection, TUI resume, server chat setup, and server pinch all consume the same active-plan lifecycle rules.

4. Deletion backcheck:
- Result: Pass
- Evidence: Removed heuristic streamed-text completion logic and replaced ad hoc resume-mode reconstruction with one canonical lifecycle path.

5. Competitive backcheck:
- Result: Pass
- Evidence: Krusty now behaves more like professional agents that keep plan state explicit, durable, and separate from non-authoritative assistant narration.

Advance decision: `Proceed`
Blocking issues: None

## Phase 7 Kickoff

Primary execution spec:
- `docs/phases/PHASE7_OBSERVABILITY_REPLAY_SPEC.md`

## Phase 7 Completion Record

Delivered artifacts:
- `crates/krusty-core/src/storage/runtime_traces.rs` (compact runtime trace store, failure taxonomy, replay summaries, and replay gate contract + tests)
- `crates/krusty-core/src/agent/observability.rs` (canonical loop-event trace forwarder + integration test)
- `crates/krusty-core/src/agent/orchestrator.rs` (trace forwarder inserted at orchestrator event boundary)
- `crates/krusty-core/src/storage/database.rs`, `crates/krusty-core/src/storage/database_tests.rs`, `crates/krusty-core/src/storage/sessions.rs`, and `crates/krusty-core/src/storage/mod.rs` (migration 19 + session/runtime trace accessors)
- `crates/krusty-server/src/routes/sessions.rs` and `crates/krusty-server/src/types.rs` (session trace retrieval surface)

Validation evidence:
- `cargo fmt --all`
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cargo test -p krusty-core runtime_trace_store_round_trip -- --nocapture`
- `cargo test -p krusty-core runtime_trace_summary_classifies_failures_and_compaction -- --nocapture`
- `cargo test -p krusty-core replay_gate_rejects_provider_failures -- --nocapture`
- `cargo test -p krusty-core runtime_trace_forwarder_persists_and_forwards_events -- --nocapture`
- `cargo test -p krusty-core test_runtime_traces_table_exists -- --nocapture`
- `cargo test -p krusty-core test_database_creation -- --nocapture`
- `cargo test -p krusty-core test_schema_version_increments -- --nocapture`

## Phase 7 Backcheck Record

Phase: 7
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: Runtime traces are captured once at the canonical `LoopEvent` boundary instead of scattering telemetry hooks across providers, tools, and UI surfaces.

2. Behavior backcheck:
- Result: Pass
- Evidence: Persisted traces now carry deterministic sequence ordering, run segmentation, terminal stop reason, and normalized failure categories for replay and diagnostics.

3. Parity backcheck:
- Result: Pass
- Evidence: TUI, server, ACP, and any future surface that consumes the orchestrator inherit the same persisted trace stream because capture happens in core before transport fan-out.

4. Deletion backcheck:
- Result: Pass
- Evidence: Reused the existing `LoopEvent` protocol and compact summary payloads instead of introducing duplicate provider-specific telemetry schemas.

5. Competitive backcheck:
- Result: Pass
- Evidence: Krusty now has durable replay-backed acceptance signals and structured failure taxonomy, closing a major professionalism gap versus top coding agents with trace/eval-driven core iteration.

Advance decision: `Proceed`
Blocking issues: None

## Phase 8 Kickoff

Primary execution spec:
- `docs/phases/PHASE8_ELEGANCE_DELETION_SPEC.md`

## Phase 8 Completion Record

Delivered artifacts:
- `crates/krusty-core/src/agent/orchestrator.rs` (shared session-manager opener + unified recovery persistence helper for repeated DB/session side effects)
- `crates/krusty-server/src/routes/sessions.rs` (shared session-manager/session-loading/ownership helpers replacing repeated route-local boilerplate)
- `crates/krusty-core/src/tools/mod.rs` and removal of `crates/krusty-core/src/tools/path_utils.rs` (deleted dead overlapping path-validation abstraction so registry/tool-context logic is the single owner)

Validation evidence:
- `cargo fmt --all`
- `cargo check -p krusty-core -p krusty -p krusty-server`
- `cargo clippy -p krusty-core -p krusty -p krusty-server -- -D warnings`
- `cargo test -p krusty-core runtime_trace_forwarder_persists_and_forwards_events -- --nocapture`
- `cargo test -p krusty-core test_runtime_traces_table_exists -- --nocapture`

## Phase 8 Backcheck Record

Phase: 8
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: Core/session route helper seams now own session access boilerplate instead of duplicating the same open/load logic in many tiny paths.

2. Behavior backcheck:
- Result: Pass
- Evidence: The refactor preserved the existing session, recovery, and trace behavior while removing only repeated plumbing and one dead helper module.

3. Parity backcheck:
- Result: Pass
- Evidence: Session-route behavior stays uniform because existence and ownership checks now come from shared local helpers rather than ad hoc per-handler variations.

4. Deletion backcheck:
- Result: Pass
- Evidence: Removed `tools::path_utils` entirely and collapsed duplicate Database/SessionManager setup paths instead of layering new abstractions on top.

5. Competitive backcheck:
- Result: Pass
- Evidence: Krusty’s core is now leaner in hot-path orchestration and session access code, moving closer to professional agents that keep fewer overlapping seams and clearer module ownership.

Advance decision: `Proceed`
Blocking issues: None

## Phase 9 Kickoff

Primary execution spec:
- `docs/phases/PHASE9_FINAL_COMPETITIVE_AUDIT_SPEC.md`

Primary closure artifacts:
- `docs/KRUSTY_CORE_FINAL_CLOSURE_REPORT.md`
- `crates/krusty-core/COMPARISON.md`

## Phase 9 Completion Record

Delivered artifacts:
- `crates/krusty-core/COMPARISON.md` (final cross-core comparison against local OpenCode, pi-mono, and Codex snapshots)
- `docs/phases/PHASE9_FINAL_COMPETITIVE_AUDIT_SPEC.md` (final audit/closure phase record)
- `docs/KRUSTY_CORE_FINAL_CLOSURE_REPORT.md` (roadmap closure report)
- this tracker updated to final scorecard state and full roadmap completion

Validation evidence:
- Local reference snapshots verified:
  - `git -C /home/burgess/Work/krusty rev-parse --short=12 HEAD`
  - `git -C /home/burgess/Work/opencode rev-parse --short=12 HEAD`
  - `git -C /home/burgess/Work/pi-mono rev-parse --short=12 HEAD`
  - `git -C /tmp/codex rev-parse --short=12 HEAD`
- Direct source anchors re-checked from:
  - `/home/burgess/Work/opencode/packages/opencode/src/session/processor.ts`
  - `/home/burgess/Work/opencode/packages/opencode/src/session/compaction.ts`
  - `/home/burgess/Work/pi-mono/packages/coding-agent/src/core/agent-session.ts`
  - `/home/burgess/Work/pi-mono/packages/coding-agent/src/core/system-prompt.ts`
  - `/tmp/codex/codex-rs/core/src/tools/orchestrator.rs`
  - `/tmp/codex/codex-rs/core/src/context_manager/history.rs`
  - `/tmp/codex/codex-rs/core/src/models_manager/manager.rs`

## Phase 9 Backcheck Record

Phase: 9
Date: 2026-03-09
Reviewer: Codex

1. Architecture backcheck:
- Result: Pass
- Evidence: The audit concludes Krusty reaches parity/advantage mostly through the typed seams created in earlier phases, not by adding heavyweight competitor-style subsystems blindly.

2. Behavior backcheck:
- Result: Pass
- Evidence: Former gaps called out in the earlier comparison are now closed by concrete runtime contracts for continuation, recovery, governance, planning, and replay.

3. Parity backcheck:
- Result: Pass
- Evidence: The final audit covers orchestrator, context, prompts, models, tools, planning, persistence, and observability across the sampled reference cores and current Krusty code.

4. Deletion backcheck:
- Result: Pass
- Evidence: Remaining differences versus OpenCode and Codex that were not copied are documented as intentional shape choices rather than leaving duplicate or overlapping machinery inside Krusty.

5. Competitive backcheck:
- Result: Pass
- Evidence: Krusty now stands at parity or advantage by design across the audited core domains, with intentional simplicity retained where heavier competitor subsystems were not necessary to reach the same runtime outcome.

Advance decision: `Close roadmap`
Blocking issues: None

## Risk Register (Initial)

| Risk | Impact | Mitigation |
| --- | --- | --- |
| Scope creep across phases | High | Strict phase gates and no cross-phase drift without change note |
| Policy duplication regression | High | Canonical owner modules and deletion backcheck each phase |
| Surface parity drift (CLI/TUI/ACP/server) | High | Mandatory parity backcheck and shared policy pathways |
| Provider drift from external APIs | Medium | Capability registry + transform seam tests + replay packs |
| Over-engineering harms elegance | High | Elegance pass (Phase 8) and per-phase deletion accounting |

## Rollback Policy Template

Every phase must include:
- rollback trigger conditions
- rollback scope (files/modules/surfaces)
- rollback verification steps
- retained artifacts that must survive rollback (tests/docs/telemetry)

## Backcheck Record Template (Use Per Phase)

Phase:
Date:
Reviewer:

1. Architecture backcheck:
- Result:
- Evidence:

2. Behavior backcheck:
- Result:
- Evidence:

3. Parity backcheck:
- Result:
- Evidence:

4. Deletion backcheck:
- Result:
- Evidence:

5. Competitive backcheck:
- Result:
- Evidence:

Advance decision: `Proceed` / `Hold`
Blocking issues:
