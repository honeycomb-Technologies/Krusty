# Phase 6 Spec: Planning and Execution Discipline

Last updated: 2026-03-09
Status: Complete

## Objective

Tighten plan-state continuity so sessions resume with the correct mode, archived plans stop leaking back into active runtime context, and task completion flows only mutate plan state through explicit lifecycle rules.

## Scope

In scope:
- `crates/krusty-core/src/plan/`
- `crates/krusty-core/src/agent/{context,plan_handler,executor}.rs`
- `crates/krusty-core/src/agent/subagent/`
- `crates/krusty-cli/src/tui/{app,handlers,polling}/`
- `crates/krusty-server/src/routes/{chat,sessions}.rs`

Out of scope for this phase:
- benchmark/eval automation
- full ACP migration onto database-backed plan state

## Required Deliverables

1. Durable plan lifecycle rules
- Shared active-vs-archived plan resolution.
- Archived plans excluded from live context injection and active-session resume state.

2. Strong mode transitions
- Session work mode persists across local TUI actions.
- Resume surfaces use canonical lifecycle resolution instead of local heuristics.

3. Completion/actionability checks
- Plan state is not mutated from assistant prose alone.
- Delegated builder completion carries explicit summary data and respects blocked-task state.

4. Cross-surface continuity
- Pinch/resume surfaces preserve active plan state when present.
- Plan task mutations emit explicit lifecycle updates for consumers.

5. Validation
- Focused lifecycle tests.
- compile + lint checks across `krusty-core`, `krusty`, `krusty-server`.

## Completion Notes

- Added shared lifecycle helpers for effective work mode and active-plan filtering in core plan code.
- Core consumers now load only active plans for runtime behavior, while archived plans remain queryable for history.
- TUI mode changes now persist to session storage, and session resume uses canonical lifecycle state instead of reconstructing mode from local heuristics.
- Removed heuristic task completion from streamed assistant prose; plan state now moves through explicit tools or delegated builder completion summaries.
- Server pinch now carries active plan markdown forward into continuation sessions.
