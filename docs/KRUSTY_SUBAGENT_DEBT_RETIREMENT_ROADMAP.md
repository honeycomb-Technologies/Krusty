# Krusty Subagent Debt Retirement Roadmap

## Purpose

Retire the remaining non-blocking architecture debt after explorer functional recovery, then finish with a deeper live audit of the Krusty repo using the repaired delegated exploration path.

## Remaining Debt

1. Delegated runtime parity with the main agent is still incomplete.
2. `build` and `explore` do not yet share one uniform delegated artifact contract.
3. Surface semantics still carry minor naming drift such as "Files examined" for path-backed evidence.
4. Delegated runs are first-class runtime units, but not full child sessions.

## Phases

### Phase 1: Shared Delegated Artifact Contract
- normalize `build` output to the same artifact vocabulary as `explore`
- preserve common fields across core/server/PWA:
  - `investigation_summary`
  - `confidence`
  - `outcome`
  - `outcome_reason`
  - `usable_agents`
  - `failed_agents`
  - `paths_examined`
  - `paths_examined_count`

Exit gate:
- delegated tools share one coherent contract instead of special-case shapes

### Phase 2: Surface Language Cleanup
- make PWA delegated widget language evidence-accurate
- prefer "Evidence examined" or path-aware language over file-only wording
- keep build/explore parity in the same component

Exit gate:
- delegated web UI matches the runtime contract cleanly

### Phase 3: Verification and Audit
- rerun validation
- restart a fresh server
- execute a broader Krusty repo `explore` audit live
- inspect session state and trace
- review the actual investigation output for quality, coverage, and honesty

Exit gate:
- debt cleanup is complete for this slice
- live delegated audit is available for review
