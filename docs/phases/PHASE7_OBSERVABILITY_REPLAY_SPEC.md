# Phase 7 Spec: Observability, Replay, and Evaluations

Last updated: 2026-03-09
Status: Complete

## Objective

Move Krusty’s core from intuition-driven debugging to replay-backed measurement by capturing a compact, structured trace of the canonical loop stream and turning that trace into deterministic summaries and regression gates.

## Scope

In scope:
- `crates/krusty-core/src/agent/{orchestrator,observability}.rs`
- `crates/krusty-core/src/storage/{runtime_traces,database,sessions,mod}.rs`
- `crates/krusty-server/src/routes/sessions.rs`
- `crates/krusty-server/src/types.rs`

Out of scope for this phase:
- benchmark harnesses across providers
- UI dashboards for trace inspection
- large curated eval corpora beyond focused replay/gating tests

## Required Deliverables

1. Structured runtime traces
- Persist the canonical `LoopEvent` stream from one boundary.
- Keep stored payloads compact and diagnostic instead of dumping raw streamed text.

2. Failure taxonomy
- Normalize terminal and tool/server failure classes into explicit categories.
- Preserve terminal stop reason separately from free-form error text.

3. Replay summaries and gates
- Build replay-friendly session summaries from persisted traces.
- Add an explicit replay gate contract that can pass/fail regressions from trace outcomes.

4. Surface access
- Make runtime trace summaries and recent events retrievable without reimplementing trace logic in the server layer.

5. Validation
- Storage migration coverage.
- Focused runtime trace store/forwarder/gating tests.
- compile + lint checks across `krusty-core`, `krusty`, and `krusty-server`.

## Completion Notes

- Added a compact `runtime_traces` persistence model and replay summary/gate contracts in core storage.
- Instrumented the orchestrator once, at the loop-event boundary, so all surfaces inherit the same trace stream without provider/tool-specific telemetry duplication.
- Stored payloads summarize event shape, sizes, and governance/failure data instead of persisting full raw deltas, preserving Krusty’s small-footprint design.
- Added a session trace API surface so recent trace events and replay summaries can be inspected externally.
- Added focused tests for trace roundtrip, failure classification, replay gating, forwarder persistence, and migration coverage.
