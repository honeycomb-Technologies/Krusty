# Phase 5 Spec: Subagents and Extensibility Unification

Last updated: 2026-03-09
Status: Complete

## Objective

Unify delegated execution paths (subagents, MCP, skills, extensions/plugins) under explicit inherited governance contracts so delegated work cannot silently bypass parent policy.

## Scope

In scope:
- `crates/krusty-core/src/agent/subagent/`
- `crates/krusty-core/src/tools/implementations/{explore,build,skill}.rs`
- `crates/krusty-core/src/mcp/tool.rs`
- `crates/krusty-core/src/tools/registry.rs`
- `crates/krusty-core/src/agent/executor.rs`

Out of scope for initial slice:
- full extension host policy mediation for every WASM host callsite
- plugin runtime policy integration beyond install/trust metadata

## Required Deliverables

1. Inherited delegated policy contract
- Delegated surfaces receive explicit inherited permission mode and turn budget.

2. Subagent governance enforcement
- Subagent tool loops enforce delegated policy (approval-sensitive writes, read-only constraints) with containment on repeated violations.

3. Delegated audit normalization
- Delegated tool outputs include normalized governance metadata for downstream consumers.

4. Cross-surface coverage
- Active delegated paths (subagent + MCP) consume the shared policy contract.

5. Validation
- Focused tests for delegated policy behavior.
- compile + lint checks across `krusty-core`, `krusty`, `krusty-server`.

## Work Breakdown

### Track A: Core contract
- Add typed delegated governance policy in the tool/runtime boundary.

### Track B: Subagent inheritance and containment
- Propagate inherited policy into `explore`/`build` spawned tasks.
- Enforce policy and containment in subagent execution loop.

### Track C: Delegated result normalization
- Include delegated governance metadata in subagent and MCP result envelopes.

### Track D: Remaining delegated surfaces
- Extend the same contract to skills/extensions/plugin execution seams.

## Completion Notes

- Active delegated execution paths now inherit explicit permission mode and turn budget through the shared `DelegationPolicy` contract.
- Subagent runtime execution resolves policy and budget from the shared contract at tool-call time, removing drift between task metadata and actual tool context.
- Direct tool execution surfaces now align with the same governance defaults and extensibility managers used by the main orchestrator.
- Skills now emit normalized governance metadata as a delegated read-only surface.
- Plugins and the WASM extension host were reviewed against this phase's exit gate. They are not active agent-dispatch execution paths today, so this phase retains their existing trust/ABI contracts rather than introducing speculative runtime mediation.

## Entry Criteria

- Phase 4 backcheck completed and recorded.

## Exit Criteria

- No delegated execution path can bypass parent permission constraints silently.
- Delegated turn budgets are explicit and inherited.
- Delegated outputs expose normalized governance metadata.
- Phase 5 backcheck recorded in tracker.

## Rollback Triggers

- Any regression where subagents execute approval-sensitive writes under supervised parent mode.
- Any delegated path loses explicit policy metadata/auditability.
