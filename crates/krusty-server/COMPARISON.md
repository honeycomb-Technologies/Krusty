# Krusty Server Final Competitive Audit

Comparison date: 2026-03-10

## Reference snapshots

| Repo | Local path | Commit |
| --- | --- | --- |
| Krusty | `/home/burgess/Work/krusty` | `a43b4333902e` |
| OpenCode | `/home/burgess/Work/opencode` | `849e1ac54378` |
| pi-mono | `/home/burgess/Work/pi-mono` | `9e22d3913a0e` |
| Codex | `/tmp/codex` | `05332b0e9619` |

## Scope

This audit is about the server and control-plane layer, not the model/runtime core that was already closed separately.

Measured domains:
- server/core contract cleanliness
- streaming transport and reconnect semantics
- live-state exactness across PWA/Desktop/mobile reopen
- remote access and trust boundary design
- operator visibility and control-plane maturity
- transport backpressure behavior

## Final outcome

For Krusty's intended operating mode, the server is now at parity or advantage by design:
- local-first self-host
- private remote access over Tailnet/VPN-style networking
- exact multi-surface session continuity
- `krusty-core` remaining the only agent brain

Krusty is not trying to be a public SaaS control plane. Within its chosen design target, closure is justified.

## Cross-product verdict

| Domain | Krusty verdict | Comparison summary |
| --- | --- | --- |
| Core/server contract | Advantage | Krusty keeps the server as a transport/control-plane layer over `krusty-core`, with session state, recovery, and trace surfaces derived from core truth instead of route-local orchestration. |
| Live-state exactness | Advantage | Exact live partial assistant state, persisted recovery, last event sequence, and presence heartbeats give Krusty a stronger multi-surface resume story than the sampled browser-local or CLI-only competitors. |
| Permission and authority model | Parity | OpenCode has a mature per-session permission system and Codex has strong typed app-server control flow; Krusty now matches the key result with ownership checks, remote bearer authority, and explicit approval binding. |
| Private remote access | Advantage | Krusty now publishes local plus Tailnet-aware private endpoints with persisted remote authority. pi-mono is browser-local, and the sampled Codex/OpenCode references do not show the same private-remote product path in this layer. |
| Transport backpressure | Parity | Codex has a strong bounded app-server queue with lag signaling. Krusty now matches the important behavior on SSE by keeping the queue bounded, dropping noncritical deltas under pressure, and surfacing `lagged` markers before terminal events. |
| Operator visibility | Parity | Access/status, trace, presence, recovery, and active-session inspection give Krusty a real operator surface rather than only raw session routes. |
| Remote/mobile continuity | Advantage | The PWA now restores exact live partial turns, keeps presence fresh, and can re-enter remote sessions with server-authored truth instead of fragile client inference. |

## Major gaps that were closed

1. Server transport used to flatten or approximate active-turn state. It now exposes canonical live partial state and trace sequence.
2. Reopened clients used to rely more on heuristic reconstruction. They now reopen from exact server-owned recovery and session state.
3. The remote trust boundary used to be effectively local-only and brittle. It now has an explicit bearer-token authority path for private remote access.
4. Slow SSE consumers could stall the bridge. The stream now uses a bounded queue with explicit lag signaling so terminal/control events remain deliverable.
5. Multi-surface session observation lacked explicit presence and stale-client handling. It now has a typed presence registry.

## Intentional deltas retained

1. Krusty does not implement public share-link collaboration like OpenCode.
Reason:
The server is intentionally optimized for trusted private networking, not public internet exposure.

2. Krusty does not mirror Codex's full JSON-RPC app-server stack.
Reason:
Krusty reaches the necessary control-plane outcomes with a simpler HTTP/SSE/WebSocket surface while keeping `krusty-core` as the canonical runtime owner.

3. Krusty does not follow pi-mono's browser-local persistence model.
Reason:
Krusty is deliberately server-authored so session truth survives app closes, device switches, and remote resume.

## Closure judgment

Server roadmap closure is justified.

Krusty's server/control-plane layer is now in the professional class for its intended product shape. The remaining differences versus the sampled alternatives are mostly intentional product choices, not missing control-plane fundamentals.

## Direct source anchors used for this audit

- Krusty:
  - `crates/krusty-server/src/auth.rs`
  - `crates/krusty-server/src/lib.rs`
  - `crates/krusty-server/src/presence.rs`
  - `crates/krusty-server/src/remote_access.rs`
  - `crates/krusty-server/src/routes/chat.rs`
  - `crates/krusty-server/src/routes/server.rs`
  - `crates/krusty-server/src/routes/sessions.rs`
  - `crates/krusty-server/src/types.rs`
  - `apps/pwa/app/src/lib/api/client.ts`
  - `apps/pwa/app/src/lib/stores/session.ts`

- OpenCode:
  - `packages/opencode/src/control-plane/workspace.ts`
  - `packages/opencode/src/permission/index.ts`
  - `packages/opencode/src/share/share-next.ts`

- pi-mono:
  - `packages/web-ui/src/index.ts`
  - `packages/web-ui/src/storage/types.ts`
  - `packages/web-ui/src/dialogs/PersistentStorageDialog.ts`

- Codex:
  - `codex-rs/app-server-client/src/lib.rs`
