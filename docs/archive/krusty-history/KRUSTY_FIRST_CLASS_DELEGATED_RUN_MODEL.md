# Krusty First-Class Delegated Run Model

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose

Phase 2 deliverable for the subagent redesign roadmap.

This document defines the first-class delegated run model now implemented in Krusty as the foundation for later evidence-contract and aggregation work.

## Problem Before

Delegated exploration/build existed mostly as:
- a parent tool call id
- child task ids
- live progress rows
- one aggregated tool result

That made delegated work too anonymous. It was difficult to treat it as a first-class runtime unit across:
- core execution
- server snapshots
- history shaping
- surface rendering

## Implemented Model

Each delegated invocation now has a stable delegated run identity that is carried through the runtime.

### Core fields

- `SubAgentTask.delegated_run_id`
- `SubAgentResult.delegated_run_id`
- `AgentProgress.delegated_run_id`
- `DelegatedProgressEvent.delegated_run_id`
- `DelegatedProgressEvent.parent_session_id`

### Tool outputs

Delegated tool results now preserve:
- `delegated_run_id`

Implemented in:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)
- [build.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/build.rs)
- [history_policy.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/history_policy.rs)

### Server snapshots and events

Delegated server state now preserves:
- `delegated_run_id`
- `parent_session_id`

Implemented in:
- [types.rs](/home/burgess/Work/krusty/crates/krusty-server/src/types.rs)
- [chat.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/chat.rs)

### PWA types

The web client contract now understands:
- `delegated_run_id`
- `parent_session_id`

Implemented in:
- [client.ts](/home/burgess/Work/krusty/apps/pwa/app/src/lib/api/client.ts)

## What This Achieves

Krusty still does not have fully sessionized delegated children like OpenCode yet, but it now has a clear runtime unit that can be:
- identified
- traced
- snapshot
- aggregated
- surfaced consistently

This removes an important ambiguity:
- delegated work is no longer just “the current top-level tool call plus some child rows”

## Why This Matters

Later phases need a stable delegated identity to support:
- stronger child evidence artifacts
- parent aggregation from child evidence
- reconnect/reload semantics
- delegated traceability
- better surface grouping and confidence reporting

Without a delegated run id, those later phases would remain brittle.

## What This Does Not Solve Yet

This phase does not by itself fix:
- weak child evidence contracts
- parent over-reliance on child summaries
- directory evidence under-modeling
- provider sensitivity

Those are the next phases.
