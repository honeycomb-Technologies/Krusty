# Krusty Server Best-In-Class Roadmap

Last updated: 2026-03-10
Owner: Server/runtime platform program
Status: Complete

## Goal

Make `krusty-server` best in class as a local-first and Tailnet/VPN-first control plane for Krusty, while preserving the existing core contract:
- `krusty-core` remains the canonical behavior owner
- `krusty-server` becomes a mature transport, state, recovery, and remote-control surface
- PWA/Desktop/mobile clients can reconnect to exact live session state without semantic drift

This roadmap is phase-gated. No phase advances until its backcheck is complete and recorded.

## Design Position

Krusty server is not a generic public SaaS backend.

Its primary operating modes are:
- local-first self-host on the user machine
- private remote access over Tailnet/VPN-style transport
- future WireGuard-class private networking without reopening unsafe public defaults

That means the roadmap optimizes for:
- exactness of session state
- trusted-device remote access
- low-friction private connectivity
- mature control-plane behavior
- strong recovery and observability

It does not optimize for:
- anonymous public internet exposure by default
- route-local business logic that bypasses core
- duplicating orchestration or policy in the server layer

## Non-Negotiables

- `krusty-core` stays the one canonical agent behavior owner.
- Server transport contracts must be deterministic across SSE, HTTP, WebSocket, and session reload.
- Local-first trust must fail closed.
- Tailnet/VPN remote access must feel seamless, but never weaken default security boundaries.
- Mobile/PWA recovery must resume to the exact known server session state, not an inferred approximation.
- No best-in-class claim without backpressure, replay, and reconnection evidence.

## Coverage Matrix

| Server area | Covered in phase(s) |
| --- | --- |
| Core/server contract shape | 1, 2 |
| Stream transport, backpressure, reconnect semantics | 2, 6 |
| Exact session/live-state continuity for PWA/Desktop/mobile | 2, 3 |
| Recovery, reconnect, and presence model | 3 |
| Local-first trust boundary and remote-access model | 4, 5 |
| Tailnet/VPN discovery and published remote endpoints | 4 |
| Multi-device and control-plane identity | 5 |
| Throughput, persistence, and event-write performance | 6 |
| Operator observability and remote lifecycle control | 7 |
| Productized remote-control workflows | 7, 8 |
| Release confidence and competitive closure | 8, 9 |

## Phase Plan

## Phase 0: Program Definition and Baseline

Purpose: freeze target server architecture and quality bar before implementation.

Deliverables:
- Master scorecard for server subsystems.
- Entry/exit criteria per phase.
- Server-specific backcheck template.
- Explicit target operating modes: local-only, Tailnet/VPN remote, future WireGuard-class remote.

Exit gate:
- Every server subsystem has `current state`, `target state`, `owner`, and `phase mapping`.
- Remote-access philosophy is explicit and future changes cannot accidentally weaken it.

## Phase 1: Canonical Core/Server Contract

Purpose: make the server a perfectly clean control-plane layer over `krusty-core`.

Deliverables:
- One explicit mapping between `LoopEvent`/`LoopInput` and every server transport.
- No route reimplements orchestration, tool policy, or recovery logic.
- Session, trace, recovery, and approval APIs all derive from canonical core state.

Exit gate:
- No major server route invents behavior that core does not own.

## Phase 2: Live Session Transport and State Exactness

Purpose: make live state transport exact under streaming, reconnects, and device switches.

Deliverables:
- Canonical event sequencing and replay contract for SSE/HTTP/WebSocket consumers.
- Reconnect/resubscribe semantics for live sessions.
- Exact session snapshot contract for PWA/Desktop/mobile surfaces.
- No client must infer session state from partial deltas alone.

Exit gate:
- Closing/reopening the app on another device restores the exact server-known session state.

## Phase 3: Recovery, Presence, and Multi-Surface Continuity

Purpose: make interrupted sessions and multi-device viewing feel continuous instead of fragile.

Deliverables:
- Presence model for active viewers/controllers.
- Resume/recovery semantics for mobile app reopen, browser refresh, and desktop handoff.
- Exact handling for awaiting-input, tool approval, and interrupted-turn recovery across devices.
- Explicit stale-client detection and refresh rules.

Exit gate:
- A session observed from multiple surfaces cannot silently diverge in status or recovery interpretation.

## Phase 4: Private Remote Access Plane

Purpose: productize remote access without weakening the local-first trust model.

Deliverables:
- Tailnet-first endpoint publication model.
- Automatic remote URL discovery/publication for trusted private networking.
- Connection metadata model: local endpoint, tailnet endpoint, future VPN endpoint.
- Clear separation between private remote access and unsupported public exposure.

Exit gate:
- Remote access feels seamless on trusted private networks and still fails closed by default.

## Phase 5: Identity, Trust, and Multi-Tenant Control Plane

Purpose: mature the server into a real control plane instead of a collection of trusted local routes.

Deliverables:
- Strong identity model for device/user/session ownership.
- Workspace/root scoping rules that cannot be widened by transport headers alone.
- Approval/control actions bound to session ownership and active authority.
- Explicit policy for single-user local mode vs future multi-user/team mode.

Exit gate:
- No control-plane action can succeed without the correct authority model.

## Phase 6: Performance, Backpressure, and Persistence Efficiency

Purpose: prove the server can carry live coding workloads cleanly.

Deliverables:
- Backpressure policy for SSE/WebSocket/event fanout.
- Session-input and event-queue behavior under churn and reconnect storms.
- Runtime trace and persistence write-budget review.
- Focused load/replay pack for long sessions, high event volumes, and multi-surface listeners.

Exit gate:
- Server remains responsive and state-correct under sustained representative load.

## Phase 7: Operator Observability and Remote Lifecycle Control

Purpose: give the server mature operator surfaces instead of only raw routes.

Deliverables:
- Control-plane health and session-state inspection surfaces.
- Remote connection status, published endpoint visibility, and failure diagnostics.
- Session trace, recovery, and delivery-state operator workflows.
- Explicit operator actions for reconnect, retry, recovery review, and stale-client handling.

Exit gate:
- Operators can diagnose and manage remote/live-state failures without guesswork.

## Phase 8: Productized Remote Experience and Client Integration

Purpose: make remote Krusty feel deliberate and polished across PWA/Desktop/mobile.

Deliverables:
- Auto-connect and endpoint handoff rules for trusted remote access.
- Better mobile pocket-state behavior and exact resume UX.
- Push/notification and reconnect flows aligned to session truth.
- Removal of client-side heuristics that should be server-authored.

Exit gate:
- Remote usage feels first-class, not like a local product stretched over transport.

## Phase 9: Final Competitive Audit and Closure

Purpose: verify that the server/control-plane layer is at parity or advantage against professional alternatives.

Deliverables:
- Cross-product comparison focused on transport, recovery, remote access, and live-state exactness.
- Remaining deltas either closed or explicitly accepted as intentional.
- Final closure report for server best-in-class status.

Exit gate:
- Every meaningful server/control-plane domain is at parity or advantage by design.

## Backcheck (Required After Every Phase)

1. Architecture backcheck: Did we keep `krusty-core` as the behavior owner?
2. Transport backcheck: Are stream/reconnect/state contracts exact and deterministic?
3. Security backcheck: Did the trust boundary get stronger, not softer?
4. Deletion backcheck: What duplicate or heuristic logic can now be removed?
5. Product backcheck: Did this make local and private remote usage meaningfully better?

No phase advancement until all five backchecks are complete and recorded.

## Execution Rules

- Never add a second agent brain inside the server.
- Never loosen remote trust defaults just to make remote access easier.
- Every remote-access convenience must be paired with an explicit authority model.
- Every live-state claim must be backed by replay or reconnect evidence.
- Every phase completion must cite proof artifacts, not just passing impressions.
