# Krusty Parent Aggregation Rewrite

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose

Phase 5 deliverable for the subagent redesign roadmap.

This phase rewrites the parent `explore` path so it reasons from delegated child artifacts instead of depending too much on child polish or vague aggregate success signals.

## Problem Before

Parent `explore` was still too summary-dependent:
- it could misclassify delegated success or partial success
- it did not carry enough deterministic coverage information forward
- it could drift into broad manual probing after a usable partial delegated run
- history shaping preserved too little of the delegated evidence contract

## Implemented Aggregation Changes

### 1. Coverage map is explicit

Parent `explore` now records:
- `coverage.status`
- `coverage.usable_targets`
- `coverage.degraded_targets`
- `coverage.failed_targets`

Implemented in:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)
- [history_policy.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/history_policy.rs)

### 2. Parent-facing investigation summary is deterministic

Parent `explore` now emits:
- `investigation_summary`
- `confidence`
- `coverage_gap_notice`

These are derived from delegated child artifacts instead of relying only on a generic completion message.

Implemented in:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)
- [history_policy.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/history_policy.rs)

### 3. Manual fallback is now guarded

When a prior delegated exploration already returned usable coverage plus an explicit `next_action_hint`, the parent is prevented from immediately drifting into broad read-only manual probing.

Implemented in:
- [failure.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/failure.rs)
- [orchestrator.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/orchestrator.rs)

## What This Achieves

The parent now has a stronger substrate for delegated reasoning:
- coverage gaps are explicit
- delegated success is harder to understate or overstate
- delegated partials are more legible
- the parent is less likely to abandon useful delegated work and start messy manual probing

## What Still Remains

This phase still leaves open:
- provider-specific delegated reliability shaping
- surface parity closure across TUI/server/PWA
- full closure validation on real multi-target runs
