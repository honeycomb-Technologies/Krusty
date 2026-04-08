# Krusty Explorer Child Runtime Upgrade

## Purpose

Phase 4 deliverable for the subagent redesign roadmap.

This phase upgrades explorer children so they operate on stronger target-scoped runtime semantics and produce usable evidence artifacts without over-relying on perfect child prose.

## Problem Before

Explorer children were still too fragile because:
- directory targets were treated as weaker evidence than concrete file reads
- `list` output lost directory markers, flattening structure evidence
- successful architecture exploration still pushed children toward concrete file reads even when directory structure already carried meaningful evidence
- structured report repair still biased too heavily toward `files_examined`

## Implemented Runtime Upgrades

### 1. Directory evidence is now preserved

`list` output now keeps trailing `/` markers when present, so directory structure survives into delegated evidence instead of being flattened into generic path strings.

Implemented in:
- [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs)

### 2. Explorer reports now support path-backed evidence

Explorer child reports now support:
- `paths_examined`
- `files_examined`

This allows a child to succeed from legitimate directory-structure evidence when that is enough to answer an architecture question defensibly.

Implemented in:
- [types.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/types.rs)

### 3. Structured report synthesis now understands directory evidence

When the runtime has to synthesize a missing `<explore_report>`, it now builds that report from all examined paths, not just concrete files.

Implemented in:
- [types.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/types.rs)
- [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs)

### 4. Completion repair no longer requires concrete file reads in all cases

The child loop now asks for real path evidence, not strictly concrete file reads, before allowing completion repair.

This matters for architecture exploration over module layout and directory structure.

Implemented in:
- [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs)

## What This Achieves

Explorer children are now materially closer to the real exploration task:
- directory structure counts as first-class evidence
- path-backed architecture summaries are possible
- the child runtime is less biased toward “read a file or fail”
- delegated evidence is more faithful to what successful `list` and `glob` calls actually returned

## What Still Remains

This phase does not finish:
- parent aggregation rewrite
- provider-specific delegated task shaping
- full surface parity and closure

Those remain the next phases.
