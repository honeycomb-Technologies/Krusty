# Krusty Live Explorer Audit - March 11, 2026

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Scope

Two live verification passes were run against the patched server.

### Pass 1: Narrow two-target validation

Session: `714ba029-612c-4a89-aceb-29d093c1f09d`

Targets:
- `crates/krusty-core/src`
- `crates/krusty-cli/src`

Outcome:
- one top-level `explore`
- two delegated children
- no manual fallback
- no second phantom `explore`
- one-turn completion with deterministic evidence-based summary

Conclusion:
- the repaired explorer path is functionally restored for scoped delegated audits

### Pass 2: Broad architecture audit

Session: `d75b8086-111d-41d3-8f72-3f7cb7cd0a90`

Targets:
- `crates/krusty-core/src/agent`
- `crates/krusty-core/src/ai`
- `crates/krusty-core/src/tools`
- `crates/krusty-core/src/storage`
- `crates/krusty-server/src`
- `crates/krusty-cli/src/tui`

Observed behavior:
- delegated execution remained serialized for MiniMax reliability
- `src/tools`, `src/storage`, `krusty-server/src`, and `src/tui` produced materially better path-backed summaries
- `src/agent` and `src/ai` still produced weak or degraded summaries
- after the sixth target completed, a new delegated run id appeared under the same top-level tool call, indicating that broad delegated audit behavior is still unstable at larger fanout

## Findings

1. Scoped delegated exploration is now functional.
The narrow two-target validation is acceptable and materially better than the broken state.

2. Broad multi-target architecture audits are still not acceptable on MiniMax.
The six-target run remained uneven and showed renewed instability after apparent completion.

3. Quality is still target-sensitive.
Modules with clearer directory/file structure (`tools`, `storage`, `server`, `tui`) fare better than semantically denser modules (`agent`, `ai`).

4. There is still a larger-fanout coherence bug.
The appearance of a fresh `delegated_run_id` under the same top-level tool call after all six targets had been processed suggests a still-open runtime/tool coordination issue for broader delegated audits.

## Bottom Line

Krusty explorer is restored for scoped delegated audits and no longer in the previously broken state.

Krusty explorer is not yet at a fully satisfactory level for broad multi-target architectural audits on the current MiniMax path. That remains an open limitation.
