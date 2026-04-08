# Krusty Broad Explorer Limitations Plan

## Goal

Move Krusty from:
- functional scoped delegated exploration

to:
- trustworthy broad multi-target architectural exploration

without regressing the scoped recovery that is already working.

## Audit-Driven Conclusions

The live audits show a split state:
- narrow two-target `explore` is now good enough
- broad six-target architecture `explore` on MiniMax is not

That means the next work should not reopen all explorer work. It should target only the still-open broad-run weaknesses.

## Focus Areas

### 1. Broad-run batching

Current problem:
- one large six-target delegated run is too expensive and too uneven

Plan:
- partition broad audits into smaller delegated batches
- aggregate batch outputs at the parent level
- avoid one huge serialized delegated sweep

### 2. Target-specific exploration strength

Current problem:
- `agent` and `ai` are harder targets than `tools`, `storage`, `server`, and `tui`

Plan:
- give `agent` and `ai` stronger architecture-audit assignments
- emphasize representative files, entrypoints, and module boundaries
- reduce weak “just list directories” completions on dense modules

### 3. Placeholder summary elimination

Current problem:
- “Let me try…” style completion text still leaks through broad runs

Plan:
- tighten broad-run completion filtering
- ensure repair-phase language cannot survive as final usable evidence
- prefer deterministic synthesis from evidence over model prose when broad runs degrade

### 4. Delegated lifecycle audit

Current problem:
- prior broad audit showed possible delegated rerun/restart behavior under one tool call

Plan:
- inspect delegated run lifecycle end to end on broad runs
- verify whether extra delegated runs are real reruns, retries, or state/progress duplication
- fix at the runtime level rather than masking it in UI

### 5. Provider policy refinement

Current problem:
- MiniMax is acceptable for scoped exploration, but still weak for broad multi-target sweeps

Plan:
- refine provider strategy specifically for broad architecture audits
- likely combine batching + stricter assignment shaping
- only escalate beyond that if provider limitation remains dominant

## Exit Criteria

This plan is complete when:
- broad architecture audits no longer collapse in quality relative to scoped runs
- `agent` and `ai` no longer lag materially behind other core modules
- placeholder/degraded summaries do not survive as usable broad-run evidence
- delegated lifecycle is stable under broad multi-target runs
- live Krusty broad audit results are strong enough to review as architecture output rather than runtime debugging artifacts
