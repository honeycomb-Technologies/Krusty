# Krusty Delegated Artifacts Closure Report

## Result
Completed the delegated-artifact program for `explore` and `build` across server and PWA.

## What Changed
- Core now exposes delegated progress in a transportable, parent-tool aware shape through `DelegatedProgressEvent`.
- Server SSE now emits typed `delegated_progress` events instead of forcing web/mobile surfaces to infer swarm state from generic tool lifecycle.
- Server keeps active delegated snapshots per session so reconnect/reload can restore live swarm state through `/sessions/:id/state`.
- PWA session state now stores delegated artifacts explicitly on tool calls.
- Explore/build rendering moved off guessed markdown parsing and onto canonical structured progress/result data.

## User-Facing Outcome
- One top-level `explore` or `build` call now behaves like one coherent delegated artifact on the web.
- Child agents render inside that artifact with live status, actions, counts, and summaries.
- Completed runs show clear final evidence: totals, files examined, errors, and summaries.
- Reloading during a live delegated run preserves agent rows through session state polling.

## Design Notes
- TUI and PWA now share semantic parity, not identical visuals.
- Runtime traces remain the canonical loop-level audit surface; delegated per-agent chronology is surfaced live through SSE and active session state, while completed delegated evidence remains persisted in tool results.
- Repeated top-level delegated calls remain distinct by tool-call id and render as separate top-level artifacts rather than being confused with child agents.

## Validation
- `cargo fmt --all`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cd apps/pwa/app && bun run check`
- `cd apps/pwa/app && bun run build`
