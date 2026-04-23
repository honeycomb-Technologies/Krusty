## Goal

Match OpenCode's external delegated-review behavior while keeping Krusty's Rust runtime and delegated-run persistence:

- child/subagent results stay private implementation detail
- parent owns the final user-facing audit/review
- delegated UI stays as one contained mutating run, not fragmented chatter
- runtime telemetry supports the run, but does not dominate the final answer

## Target Contract

1. `explore` runs as one delegated investigation.
2. Child artifacts are structured evidence, not direct user chat output.
3. Parent finalization emits the natural audit/review text.
4. PWA shows one contained delegated card during execution and a normal review answer on completion.
5. Counts/confidence/coverage remain available in details, traces, and diagnostics, not as the main answer body.

## Phase 1: Parent-Owned Review Output

- Make `human_review` the canonical final user-facing answer for successful `explore` turns.
- Remove fallback finalization text that reintroduces telemetry-heavy wording when `human_review` exists.
- Rewrite the generated human review to read like an audit:
  - executive summary
  - per-target review
  - cross-cutting strengths
  - cross-cutting weaknesses/gaps
  - compact caveat only when coverage is partial

## Phase 2: Delegated Artifact Privacy

- Keep delegated machine artifacts rich for traces/state/debugging.
- Preserve delegated fields in history/state for runtime continuity.
- Do not promote machine artifact fields into top-level assistant prose unless explicitly summarized by the parent.

## Phase 3: PWA Containment and Identity

- Ensure delegated runs keep stable identity across streaming and reload.
- Deduplicate cards by delegated run id wherever the run is represented.
- Keep delegated thinking inside the delegated card whenever a delegated run exists.

## Phase 4: Completed-Run Presentation

- Completed delegated cards should become compact progress records, not second competing summaries.
- De-emphasize runtime telemetry in completed headers.
- Hide prompt/runtime summary duplication when the parent message already contains the real review.

## Phase 5: Validation

- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cargo fmt --all --check`
- `cd apps/pwa/app && bun run check`
- `cd apps/pwa/app && bun run build`

## Exit Criteria

- Scoped and broad `explore` runs surface one coherent delegated card in PWA.
- Final user-facing answer reads like a natural audit/review.
- Delegated telemetry remains available, but secondary.
- Reloaded sessions preserve delegated run identity and do not splinter into duplicate cards.
