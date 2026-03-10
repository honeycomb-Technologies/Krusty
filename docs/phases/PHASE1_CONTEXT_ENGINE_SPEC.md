# Phase 1 Spec: Context Engine and Deterministic Continuation

Last updated: 2026-03-09
Status: Complete

## Objective

Eliminate context identity drift across long turns, compaction, interruption, and resume by introducing a first-class context ledger and deterministic continuation contract.

## Scope

In scope:
- `crates/krusty-core/src/agent/compaction.rs`
- `crates/krusty-core/src/agent/orchestrator.rs`
- `crates/krusty-core/src/agent/context.rs`
- `crates/krusty-core/src/agent/history_policy.rs`
- New context ledger module under `crates/krusty-core/src/agent/`
- Session persistence touchpoints needed for resume invariants

Out of scope:
- Full tool sandbox matrix (Phase 3)
- ACP/server parity completion (Phase 4)
- Replay/evals infrastructure (Phase 7)

## Required Deliverables

1. Context ledger model
- Explicit segments: canonical, summarized, dropped, pinned, replayed.
- Deterministic serialization contract for ledger state used by orchestrator.

2. Continuation contract
- Deterministic `resume intent` payload after compaction/interruption.
- Explicit unsafe-to-resume reasons.

3. Compaction invariants
- Preserve latest actionable user objective.
- Preserve required tool evidence contracts for continuation.
- Never silently drop pinned context.

4. Resume semantics
- Resume path consumes ledger + continuation payload, not inferred heuristics alone.

5. Focused tests
- Long-thread continuation tests.
- Compaction edge cases with aggressive tool-history pruning.
- Resume-safe vs non-resumable decision tests.

## Work Breakdown

### Track A: Ledger model and integration
- Add `context_ledger.rs` module and typed structures.
- Integrate ledger updates at key orchestrator boundaries.
- Add persistence-safe ledger serialization strategy.

### Track B: Compaction behavior hardening
- Refactor compaction entry/exit around ledger contract.
- Ensure replacement summaries carry continuation-critical state only.

### Track C: Resume contract
- Add typed continuation decision enum (`resumable` / `non_resumable(reason)`).
- Surface reasons to caller pathways.

### Track D: Validation
- Add targeted unit/integration tests.
- Re-run `krusty-core` and `krusty` test suites before phase backcheck.

## Entry Criteria

- Phase 0 complete with documented backcheck.
- Current orchestrator/compaction behavior validated and baseline understood.

## Exit Criteria

- Ledger model is the source of truth for continuation decisions.
- Compaction and interruption paths are deterministic and test-backed.
- No pinned context is dropped implicitly.
- Resume behavior has explicit success/failure reasons.
- Backcheck for Phase 1 completed and recorded in tracker.

## Rollback Triggers

- Context regressions causing objective drift after compaction.
- Resume path introducing duplicate tool actions.
- Inability to preserve pinned evidence under pressure.
