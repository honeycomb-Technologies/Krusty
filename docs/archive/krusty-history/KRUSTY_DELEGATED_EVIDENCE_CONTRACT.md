# Krusty Delegated Evidence Contract

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose

Phase 3 deliverable for the subagent redesign roadmap.

This document defines the evidence-first contract for delegated exploration so parent aggregation can reason from structured artifacts instead of child prose alone.

## Problem Before

Delegated exploration still depended too much on:
- child final prose
- `success` flags
- overloaded `files_examined` semantics

This made directory-level architecture exploration too fragile and made parent aggregation too summary-dependent.

## Implemented Contract

Explorer child results now expose a canonical evidence artifact via:
- [types.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/types.rs)

### Canonical artifact fields

- `agent`
- `delegated_run_id`
- `success`
- `usable_evidence`
- `degraded_success`
- `outcome_reason`
- `summary`
- `paths_examined`
- `files_examined`
- `directories_examined`
- `key_findings`
- `design_patterns`
- `concerns`
- `confidence`
- `turns_used`
- `duration_ms`
- `error`
- `policy_violations`

## Important Semantic Change

Krusty now explicitly separates:
- `paths_examined`
- `files_examined`
- `directories_examined`

That matters because architecture exploration frequently learns important facts from directory structure before it reads a specific source file.

## Parent Tool Result Updates

The explore tool now preserves:
- `delegated_run_id`
- `paths_examined`
- `paths_examined_count`
- `directories_examined_count`
- `concrete_files_examined_count`

Implemented in:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)
- [history_policy.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/history_policy.rs)

## Surface Contract Updates

The PWA delegated artifact parser now prefers:
- `paths_examined`
- `paths_examined_count`

with compatibility fallback to legacy file-based fields.

Implemented in:
- [session.ts](/home/burgess/Work/krusty/apps/pwa/app/src/lib/stores/session.ts)

## What This Enables

This phase gives the parent a much better substrate for later phases:
- better coverage reasoning
- better partial-result honesty
- better target-level aggregation
- less dependence on child final wording

## What Still Remains

This phase does not yet finish:
- parent aggregation rewrite
- fallback/manual continuation reduction
- provider-specific delegated task strategy

Those are the next phases.
