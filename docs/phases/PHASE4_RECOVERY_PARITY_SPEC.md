# Phase 4 Spec: Persistence, Recovery, and Surface Parity

Last updated: 2026-03-09
Status: Complete

## Objective

Make interrupted turns deterministic across storage, orchestrator, TUI, server, and ACP without polluting canonical conversation history with partial assistant output.

## Scope

In scope:
- `crates/krusty-core/src/agent/orchestrator.rs`
- `crates/krusty-core/src/agent/stream.rs`
- `crates/krusty-core/src/storage/`
- `crates/krusty-cli/src/tui/handlers/`
- `crates/krusty-core/src/acp/`
- `crates/krusty-server/src/routes/sessions.rs`
- `crates/krusty-server/src/types.rs`

Out of scope:
- full ACP execution-engine unification with the canonical orchestrator (later phase)
- observability/replay suites and benchmark harnesses (Phase 7)
- subagent and extension inheritance rules (Phase 5)

## Required Deliverables

1. Typed recovery snapshot
- Persist an explicit interrupted-turn contract in storage, separate from canonical session messages.

2. Crash-safe partial-turn handling
- Persist streaming checkpoints and tool-execution risk state without committing partial assistant output to durable history.

3. Explicit resume contract
- Mark resumable vs non-resumable recovery states with typed reasons rather than heuristic banners.

4. Surface parity
- TUI, server, and ACP consume the same persisted recovery semantics.

5. Validation
- Migration, storage roundtrip, recovery notice, and ACP restore tests.
- Compile and lint checks across `krusty-core`, `krusty`, and `krusty-server`.

## Work Breakdown

### Track A: Storage contract
- Add `recovery_json` migration and public recovery types.
- Add load/save/clear APIs on `SessionManager`.

### Track B: Orchestrator recovery wiring
- Persist streaming checkpoints.
- Persist tool-execution non-resumable state.
- Keep partial assistant content out of canonical session history on interrupted turns.

### Track C: Surface consumers
- Server session state exposes typed recovery state.
- TUI banners/load flow render persisted recovery notices.
- ACP injects a one-shot recovery notice into the next resumed prompt.

### Track D: Validation and backcheck
- Add focused tests and record phase completion evidence.

## Entry Criteria

- Phase 3 backcheck completed and recorded.

## Exit Criteria

- Interrupted work is represented by a typed persisted recovery contract.
- Partial assistant output is not saved into canonical history when the turn fails mid-stream.
- TUI, server, and ACP consume the same recovery semantics.
- Phase 4 backcheck recorded in tracker.

## Rollback Triggers

- Any regression where interrupted turns silently mutate canonical conversation history.
- Surface-specific recovery behavior diverges from persisted storage state.
- Recovery persistence introduces hidden replay of unsafe tool actions.
