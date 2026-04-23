# Krusty Provider Reliability Layer

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose

Phase 6 deliverable for the subagent redesign roadmap.

This phase makes delegated exploration degrade more gracefully on weaker providers, especially MiniMax, by narrowing the delegated workflow instead of forcing every provider through the same broad exploration pattern.

## Problem Before

Provider handling was still too generic:
- concurrency and stagger were provider-aware
- delegated assignment shape was not
- MiniMax often received the same broad child workflow as stronger delegated providers

That increased the odds of thin, incoherent exploration rather than honest scoped coverage.

## Implemented Reliability Layer

### 1. Provider-aware concurrency and stagger

Already in place and retained:
- MiniMax concurrency clamped to `1`
- MiniMax launch stagger increased

Implemented in:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)

### 2. Provider-aware delegated assignment shaping

Explorer assignment generation is now provider-aware.

MiniMax child tasks now receive a narrower workflow:
- start with `list` or one focused `glob`
- prefer fewer high-signal tool calls
- use `grep` only for specific confirmation
- treat successful structure evidence as authoritative
- move toward synthesis earlier

Implemented in:
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)

## What This Achieves

Krusty now treats provider quality differences as a delegated runtime concern:
- stronger providers can still use the broader explore workflow
- weaker providers get a narrower, lower-drift child assignment
- degradation is more likely to become honest partial coverage instead of incoherent delegation

## What Still Remains

This phase still leaves open:
- deeper provider diagnostics in traces and snapshots
- more aggressive provider-specific fanout policies if real runs still show instability
