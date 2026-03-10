# Phase 3 Spec: Tool and Sandbox Maturity

Last updated: 2026-03-09
Status: Complete

## Objective

Raise Krusty's tool runtime to a professional-grade policy model with explicit approval, retry, and plan-mode invariants that are shared across all surfaces.

## Scope

In scope:
- `crates/krusty-core/src/tools/registry.rs`
- `crates/krusty-core/src/agent/tool_control.rs`
- `crates/krusty-core/src/agent/executor.rs`
- `crates/krusty-core/src/agent/hooks.rs`
- high-impact tool implementations that need explicit policy metadata

Out of scope:
- full crash-safe persistence and replay semantics (Phase 4)
- extension/subagent governance unification (Phase 5)

## Required Deliverables

1. Canonical tool policy contract
- Explicit policy object for approval, retry, and plan-mode behavior.

2. Policy-driven executor behavior
- Approval and retry pathways consume tool policy instead of ad hoc category checks.

3. Plan-mode consistency
- Plan-mode blocking derives from the same tool policy contract.

4. Validation
- Targeted tests for tool policy classification and executor/policy outcomes.

## Work Breakdown

### Track A: Policy object
- Add shared tool policy definition.
- Remove duplicated behavior encoded by raw category checks.

### Track B: Executor alignment
- Route approval and retry decisions through canonical policy object.

### Track C: Plan-mode alignment
- Ensure plan-mode guardrails reference shared policy contract.

### Track D: Further sandbox maturity
- Expand beyond current approval model into richer policy matrix where needed.

## Entry Criteria

- Phase 2 backcheck completed and recorded.

## Exit Criteria

- Approval, retry, and plan-mode rules derive from one canonical tool policy layer.
- No major tool execution path relies on duplicated implicit category logic.
- Phase 3 backcheck recorded in tracker.

## Rollback Triggers

- Regressions that over-block safe read-only workflows.
- Approval or plan-mode behavior diverges across surfaces.
