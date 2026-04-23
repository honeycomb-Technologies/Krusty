# Krusty Subagent Redesign Closure Report

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Outcome

The explorer redesign restored functional delegated exploration on the live server.

On March 11, 2026, live verification against session `714ba029-612c-4a89-aceb-29d093c1f09d` confirmed:
- a single top-level `explore` invocation
- target-scoped delegated children for `crates/krusty-core/src` and `crates/krusty-cli/src`
- usable evidence returned from both children
- no parent fallback into broad manual reads
- no second phantom `explore` invocation after a successful first run
- deterministic parent completion from delegated evidence in one turn

## What Was Fixed

1. Child target binding was made authoritative.
Explorer children now run from the actual assigned target and are told not to re-locate that path from inside themselves.

2. Delegated evidence became first-class.
Explorer children now preserve `paths_examined`, `files_examined`, and `directories_examined`, and successful runs can synthesize usable reports from path-backed evidence.

3. Placeholder child summaries stopped counting as success.
Weak or non-substantive summaries are downgraded, repaired, or deterministically synthesized from real evidence instead of being trusted as-is.

4. Parent aggregation became deterministic.
`explore` now emits explicit coverage, investigation summary, confidence, and gap notices.

5. Successful single-`explore` turns now finalize directly.
The orchestrator now ends successful single-`explore` turns with an evidence-based assistant response instead of handing control back to the model for another potentially incoherent turn.

## Remaining Architectural Debt

The explorer is functional again, but two longer-term redesign items remain:
- main-agent/subagent runtime parity is still incomplete
- delegated runs are first-class runtime units, but not yet full child sessions with independent persistence semantics

These are no longer blocking functional explorer behavior. They are follow-on architecture work.
