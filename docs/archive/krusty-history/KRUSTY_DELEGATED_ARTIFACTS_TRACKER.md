# Krusty Delegated Artifacts Tracker

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Program
Delegated Artifacts Roadmap

## Objective
Make delegated `explore` and `build` first-class across server and PWA with TUI-grade semantic clarity.

## Current Baseline
- Core has typed delegated progress and structured final result envelopes.
- TUI consumes delegated progress directly.
- Server now forwards delegated progress with parent tool-call association.
- PWA now stores delegated artifacts explicitly and renders dedicated explore/build widgets.

## Remaining Program Status

| Phase | Name | Status | Notes |
| --- | --- | --- | --- |
| 1 | Canonical Delegated Event Contract | Complete | Added typed delegated SSE events with stable fields for parent tool call, kind, agent row state, and build-specific metrics. |
| 2 | Server Transport Bridging | Complete | Server now forwards core delegated progress through SSE while preserving parent tool-call association. |
| 3 | PWA Delegated State Model | Complete | `ToolCall` now carries delegated artifact state with live agent rows, summaries, and file/error metadata. |
| 4 | Dedicated PWA Rendering | Complete | Explore/build now render through a dedicated delegated widget using canonical JSON results instead of guessed markdown sections. |
| 5 | Persistence and Recovery Parity | Complete | Active delegated snapshots survive reconnect/reload through `/sessions/:id/state`, while completed runs reload from persisted final tool results. |
| 6 | Trace and Session API Parity | Complete | Session state now exposes delegated tool snapshots for active runs; operator debugging still uses existing runtime traces plus persisted tool results. |
| 7 | Validation and Closure | Complete | Workspace validation gate passed clean, including targeted delegated snapshot tests. |

## Completed Deliveries
- Added `DelegatedProgressEvent` / `DelegatedToolKind` in core and wired delegated progress through the orchestrator execution seam.
- Added `delegated_progress` SSE events and live delegated snapshot aggregation on the server.
- Extended session state responses with active delegated tool snapshots for reconnect/reload parity.
- Added explicit delegated artifact state in the PWA store and merged it from both SSE and session-state polling.
- Added a dedicated delegated web widget for `explore` / `build` with live agent rows, summaries, files examined, and errors.
- Replaced guessed markdown parsing with canonical parsing of core explore/build JSON result envelopes.
- Added a targeted server regression test for delegated snapshot aggregation.

## Validation Evidence
- `cargo fmt --all`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cd apps/pwa/app && bun run check`
- `cd apps/pwa/app && bun run build`

## Closure
Delegated explore/build are now first-class across core, server, and PWA. The TUI still has its own richer terminal-native presentation, but server/PWA are no longer generic tool-widget approximations.
