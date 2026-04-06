# AGENTS Guide: /crates/krusty-server/src/routes

## Purpose
HTTP route handlers and endpoint contracts.

## Guardrails
- Keep request/response shapes synchronized with CLI, web, and mobile clients.
- Validate and sanitize all user inputs before side effects.
- Preserve streaming route stability and backpressure behavior.
- Chat routes must honor persisted session model unless an explicit per-request override is provided.
- Session routes must keep `working_dir` as runtime source-of-truth and treat `target_branch` as optional session intent metadata (never a hard execution override by itself).
- Session trace/diagnostic endpoints must expose core-derived summaries and events, not rebuild telemetry from route-local heuristics.
- Session routes should share common session-manager/session-loading helpers rather than duplicating existence and ownership plumbing per handler.
- Session creation, read, pinch, and approval routes must preserve multi-tenant ownership end-to-end; never create or expose a session without binding it back to the authenticated user context when one exists.
- Session presence routes must stay ownership-checked, server-authored, and stale-aware; clients may heartbeat state but must not become the authority for session truth.
- Tool execution routes must pass the same governance context as orchestrated runs (permission mode, delegated turn budget, and extensibility managers) so direct execution does not silently diverge from core behavior.
- Direct tool execution must keep `working_dir` scoped to the same allowed workspace root as the rest of the server file/path surfaces.
- Chat streaming must keep a bounded queue with explicit lag signaling; never let a slow SSE client silently stall or redefine core loop semantics.
- Server control-plane routes must expose remote-access state, token rotation, and published private endpoints without bypassing the bearer-token remote authority model.
- Push endpoints (`/push/*`) must stay aligned with mobile/web diagnostics and test-send flows.
- Port proxy endpoints (`/ports/*`) must remain localhost-scoped and deny recursive self-proxy loops.

## Validation
- `cargo check -p krusty-server`
- test affected endpoints from client code paths when contracts change.
