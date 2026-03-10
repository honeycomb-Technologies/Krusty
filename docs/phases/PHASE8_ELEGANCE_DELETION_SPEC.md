# Phase 8 Spec: Elegance and Deletion Pass

Last updated: 2026-03-09
Status: Complete

## Objective

Reduce architectural noise without reducing capability by collapsing repeated helper paths, deleting dead overlap, and tightening module ownership around the execution/storage seams already stabilized in earlier phases.

## Scope

In scope:
- `crates/krusty-core/src/agent/orchestrator.rs`
- `crates/krusty-server/src/routes/sessions.rs`
- `crates/krusty-core/src/tools/{mod,registry}.rs`

Out of scope for this phase:
- new feature work
- new telemetry/eval systems
- broad behavioral rewrites across unrelated subsystems

## Required Deliverables

1. Duplicate execution path removal
- Collapse repeated `Database`/`SessionManager` setup in core persistence helpers.
- Collapse repeated session route open/load/ownership paths into shared helpers.

2. Overlap deletion
- Remove dead or redundant helper modules where another canonical owner already exists.

3. Ownership tightening
- Keep one owner for tool path policy.
- Keep one owner for session access in the session routes.

4. Validation
- compile + lint checks across `krusty-core`, `krusty`, and `krusty-server`
- focused regression tests on touched trace/session paths

## Completion Notes

- Replaced the orchestrator’s repeated database/session-open blocks with shared helper functions so persistence side effects no longer each rebuild the same boilerplate.
- Replaced repeated session route loader/ownership logic with shared helper functions in the route module.
- Deleted the unused `tools::path_utils` module so filesystem path policy stays owned by `ToolContext`/registry logic instead of split across overlapping helpers.
- Updated local AGENTS files to keep these ownership boundaries explicit going forward.
