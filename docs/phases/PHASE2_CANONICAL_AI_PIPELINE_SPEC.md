# Phase 2 Spec: Canonical AI Execution Pipeline

Last updated: 2026-03-09
Status: Complete

## Objective

Unify all AI request paths behind one canonical policy pipeline so provider/model/prompt/tool behavior is consistent across streaming, non-streaming, subagent, ACP, TUI, and server surfaces.

## Scope

In scope:
- `crates/krusty-core/src/ai/client/`
- `crates/krusty-core/src/ai/transform.rs`
- `crates/krusty-core/src/ai/model_profile.rs`
- `crates/krusty-core/src/ai/providers.rs`
- `crates/krusty-core/src/agent/orchestrator.rs`
- `crates/krusty-core/src/acp/` request pathways
- server/TUI call setup touchpoints that currently bypass canonical options normalization

Out of scope:
- Tool sandbox policy matrix (Phase 3)
- Full surface parity/backfill persistence work (Phase 4)
- Eval framework and replay suites (Phase 7)

## Required Deliverables

1. Canonical request profile
- A single typed profile assembled once per call path (model capability, reasoning mode, context management, tools policy, streaming policy).

2. Shared options normalizer
- One function that normalizes `CallOptions` and disallows surface-specific drift.

3. Prompt-pack assembly seam
- A shared prompt assembly pipeline for orchestrator/server/ACP/subagents.

4. Provider transform consistency
- Ensure all providers use the same policy gates before request transformation.

5. Validation
- Targeted tests proving parity between streaming and non-streaming options build paths.
- Compile checks across `krusty-core`, `krusty`, and `krusty-server`.

## Work Breakdown

### Track A: Request profile + normalization
- Add canonical request profile types.
- Route all outbound requests through normalizer.

### Track B: Prompt-pack unification
- Remove duplicated prompt assembly branches.
- Ensure project/plan/skills context behavior is consistent.

### Track C: Provider capability enforcement
- Enforce profile-driven capability guards centrally (not scattered per surface).

### Track D: Validation and drift checks
- Add focused tests for option parity across pathways.

## Entry Criteria

- Phase 1 backcheck completed and recorded.

## Exit Criteria

- No primary AI call path bypasses canonical request normalization.
- Prompt assembly is consistent across major surfaces.
- Provider capability rules are enforced by shared policy logic.
- Phase 2 backcheck recorded in tracker.

## Rollback Triggers

- Any regression where one surface silently differs in model/tool/prompt policy.
- Increased duplicated policy logic across `ai/client` modules.
