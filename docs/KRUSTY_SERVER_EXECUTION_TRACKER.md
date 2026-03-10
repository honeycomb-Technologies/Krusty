# Krusty Server Execution Tracker

Last updated: 2026-03-10
Program state: Complete

Reference plan: `docs/KRUSTY_SERVER_BEST_IN_CLASS_ROADMAP.md`

## Phase Status

| Phase | Name | Status | Entry met | Exit met |
| --- | --- | --- | --- | --- |
| 0 | Program Definition and Baseline | Completed | Yes | Yes |
| 1 | Canonical Core/Server Contract | Completed | Yes | Yes |
| 2 | Live Session Transport and State Exactness | Completed | Yes | Yes |
| 3 | Recovery, Presence, and Multi-Surface Continuity | Completed | Yes | Yes |
| 4 | Private Remote Access Plane | Completed | Yes | Yes |
| 5 | Identity, Trust, and Multi-Tenant Control Plane | Completed | Yes | Yes |
| 6 | Performance, Backpressure, and Persistence Efficiency | Completed | Yes | Yes |
| 7 | Operator Observability and Remote Lifecycle Control | Completed | Yes | Yes |
| 8 | Productized Remote Experience and Client Integration | Completed | Yes | Yes |
| 9 | Final Competitive Audit and Closure | Completed | Yes | Yes |

## Current Watchpoints

| Area | Current state | Phase owner |
| --- | --- | --- |
| Core/server contract fidelity | Closed: server transports now derive from canonical core events, recovery state, and session truth | 1, 2 |
| Live-state exactness across reconnects | Closed: clients reopen from exact server-authored live partial state and trace sequence, not inferred deltas | 2, 3 |
| Private remote access plane | Closed for target mode: local-first plus Tailnet/VPN private remote with explicit bearer authority | 4, 5 |
| Performance and backpressure | Closed for current target: bounded SSE queue, explicit lag signaling, and delta trace retrieval prevent slow-consumer stalls | 6 |
| Operator visibility and remote lifecycle control | Closed: access/status, presence, trace, and recovery surfaces now expose control-plane state directly | 7 |
| Client-integrated remote experience | Closed: PWA bootstraps remote authority, heartbeats presence, and restores active turns exactly | 8 |

## Completion Record

Phase 0:
- Program target frozen around local-first, private remote access, exact live session truth, and server-as-control-plane rather than second agent brain.

Phase 1:
- Closed server/core drift by exposing canonical `LoopEvent` shapes through server transport and by using core-owned recovery/session state in server snapshots.

Phase 2:
- Added `live_partial_assistant`, `last_event_sequence`, and `after_sequence` trace reads so reconnecting clients can resume from server truth instead of replay heuristics.

Phase 3:
- Added ownership-checked session presence with stale detection and PWA heartbeats so multi-surface viewing/control cannot silently diverge.

Phase 4:
- Added remote access configuration, persisted bearer authority, Tailscale-aware endpoint publication, and remote launch URLs for private-network handoff.

Phase 5:
- Hardened the authority boundary so remote API traffic must present the persisted bearer token and workspace scope cannot be widened by headers alone.

Phase 6:
- Closed the main transport performance defect by replacing blocking SSE forwarding with a bounded queue and explicit `lagged` signaling for slow consumers while preserving delivery of terminal/control events.

Phase 7:
- Added operator control-plane surfaces for access, status, presence, recovery, and trace inspection so remote/live-state failures can be diagnosed without guessing.

Phase 8:
- Productized the client handoff path by teaching the PWA to bootstrap remote authority, restore live partial turns, and keep session presence fresh across reopen/visibility changes.

Phase 9:
- Re-audited the resulting server/control-plane design against local OpenCode, pi-mono, and Codex references and recorded the closure outcome.

## Validation Evidence

- `cargo fmt --all`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run check`
- `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run build`

Targeted proof added during closure:
- `cargo test -p krusty-server forward_loop_event_surfaces_lag_before_terminal_event -- --nocapture`
- `cargo test -p krusty-server is_local_host_accepts_loopback_names -- --nocapture`

## Backcheck Template

Use this at the end of every phase:

1. Architecture backcheck:
   - Did we preserve `krusty-core` as the canonical behavior owner?

2. Transport backcheck:
   - Are stream, reconnect, and session-state contracts exact and deterministic?

3. Security backcheck:
   - Did the local-first and private-remote trust boundary get stronger?

4. Deletion backcheck:
   - What duplicate transport, recovery, or client heuristic logic can now be removed?

5. Product backcheck:
   - Did this materially improve local use, private remote use, or live-state exactness?

## Final Decision

Roadmap complete. Decision: `Go`.
