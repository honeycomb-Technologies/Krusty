# Krusty Broad Explorer Limitations Tracker

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose

Track the remaining issues exposed by the live broad architecture audit after scoped delegated exploration was restored.

This tracker intentionally excludes already-closed scoped explorer recovery work.

## Current State

- Scoped delegated exploration: Functional
- Broad multi-target architecture exploration on MiniMax: Not yet acceptable

Primary audit source:
- [KRUSTY_LIVE_EXPLORER_AUDIT_2026_03_11.md](./KRUSTY_LIVE_EXPLORER_AUDIT_2026_03_11.md)
- [KRUSTY_BROAD_EXPLORER_AUDIT_FINDINGS.md](./KRUSTY_BROAD_EXPLORER_AUDIT_FINDINGS.md)

## Open Issues

### BEL-001: Broad fanout quality collapse on MiniMax

Large delegated audit batches degrade significantly compared to scoped two-target runs.

Evidence:
- six-target Krusty audit was materially weaker than the two-target validation
- execution remained serialized and expensive
- mixed child quality weakened the overall audit

Status: Open

### BEL-002: `agent` and `ai` targets underperform

The `crates/krusty-core/src/agent` and `crates/krusty-core/src/ai` targets still tend to produce weak or degraded summaries compared to other modules.

Evidence:
- `tools`, `storage`, `server`, and `tui` performed better in the live audit
- `agent` and `ai` still showed placeholder or degraded behavior

Status: Open

### BEL-003: Placeholder/forced-summary leakage in broad runs

Even after scoped recovery, broad runs still allow low-value completion text such as “Let me try...” or similar repair-phase language to survive too far into delegated completion.

Evidence:
- live audit showed placeholder-style summaries on `agent`, `ai`, and parts of the wider run

Status: Open

### BEL-004: Large-run delegated lifecycle instability

Broad runs may still exhibit unexpected delegated rerun/restart behavior under one top-level tool call.

Evidence:
- prior broad audit showed a fresh `delegated_run_id` appearing under the same top-level `explore` call after apparent completion

Status: Open

### BEL-005: Provider strategy mismatch for broad architecture audits

MiniMax can be made acceptable for scoped delegated exploration, but current behavior suggests it is still a weak fit for broader multi-target architecture sweeps without additional provider shaping or batching strategy.

Status: Open

## Next Program Shape

The next fix program should focus on:
1. batched broad exploration instead of large single fanout
2. stronger target-specific assignments for `agent` and `ai`
3. stricter placeholder-summary elimination in broad runs
4. delegated lifecycle audit for rerun/restart behavior
5. provider policy refinement for broad architectural exploration
