# Krusty Core Final Competitive Audit

Comparison date: 2026-03-09

## Reference snapshots

| Repo | Local path | Commit |
| --- | --- | --- |
| Krusty | `/home/burgess/Work/krusty` | `a43b4333902e` |
| OpenCode | `/home/burgess/Work/opencode` | `849e1ac54378` |
| pi-mono | `/home/burgess/Work/pi-mono` | `9e22d3913a0e` |
| Codex | `/tmp/codex` | `05332b0e9619` |

## Final outcome

Krusty is now at parity or advantage by design across the core domains that matter for a professional coding agent, while keeping a materially smaller and more modular runtime surface than the sampled competitor cores.

This audit reflects Krusty after completion of roadmap Phases 1 through 8:
- deterministic continuation + context ledger
- canonical AI call pipeline
- unified tool policy and delegated governance
- crash-safe recovery and cross-surface parity
- planning discipline
- replay-backed observability
- deletion/elegance pass

## Cross-core verdict

| Domain | Krusty verdict | Comparison summary |
| --- | --- | --- |
| Orchestration loop | Parity | Krusty now has a single orchestrator with explicit stop reasons, resumability contracts, crash-safe partial-turn recovery, and replay traces. OpenCode/pi-mono/Codex remain strong here, but Krusty is no longer structurally behind. |
| Context continuity and compaction | Parity | Context ledger, pinned compaction invariants, typed recovery, and pinch carry-forward close the earlier continuity gap. Codex still has a richer internal context manager, but Krusty achieves equivalent continuation outcomes with less machinery. |
| Prompt layering and call shaping | Parity/Advantage | Krusty now routes all main AI surfaces through one canonical options seam and model-profile prompt family path. OpenCode is deeper on transform plugins, but Krusty is cleaner and more typed. |
| Provider/model handling | Parity | Built-in metadata wins over heuristics, custom model IDs are first-class, and provider/model capability shaping is centralized. Codex has a heavier model-manager stack, but Krusty covers the needed runtime outcomes. |
| Tool governance and safety | Parity | Shared `ToolPolicy`, approval/retry/plan-mode enforcement, delegated governance, and audit metadata now cover the major safety envelope. Codex still uses a more explicit dedicated tool-orchestrator type, but Krusty reaches the same governed execution result. |
| Tool evidence contracts | Parity | High-impact tool paths now use structured envelopes or explicit governance metadata instead of ad hoc plain-text-only behavior. This closes one of the remaining professionalism gaps. |
| Planning discipline | Advantage | Active-vs-archived plan lifecycle, explicit work-mode persistence, and no prose-driven task completion give Krusty a very clean durable planning model across resumes and long sessions. |
| Persistence and resume parity | Parity | Partial turns, recovery notices, and surface parity across TUI/server/ACP are explicit and typed rather than implicit. |
| Observability and replay | Parity | Runtime traces are captured once at the canonical event boundary and surfaced as replay summaries and recent event streams. Codex has broader surrounding infrastructure, but Krusty now has enough durable observability to gate regressions by outcome instead of intuition. |
| Elegance / footprint | Advantage | Krusty remains smaller and more modular than the sampled competitor cores while now covering the critical runtime behaviors that used to require heavier systems elsewhere. |

## Former gaps now closed

1. Mid-turn continuation and compaction ambiguity is closed by the context ledger, continuation contract, and typed recovery state.
2. Provider/model drift across AI call surfaces is closed by canonical call-option normalization and shared prompt/model-profile policy.
3. Tool approval/retry/plan-mode drift is closed by shared `ToolPolicy` ownership.
4. Delegated execution drift is closed by inherited governance contracts across subagents, MCP, skills, and direct execution surfaces.
5. Plan/task mutation drift is closed by canonical lifecycle helpers, explicit plan events, and removal of prose-driven completion.
6. Replay/eval blindness is closed by persisted runtime traces plus replay summaries and gate contracts.
7. Duplicate hot-path helper boilerplate was reduced in the deletion pass without losing behavior.

## Intentional deltas retained

These deltas are accepted intentionally and do not block closure because Krusty already meets the required runtime outcome with a smaller or clearer design.

1. Krusty does not mirror Codex’s heavier standalone model-manager and refresh stack.
Reason:
Krusty’s typed provider/model registry, canonicalization seam, and lightweight dynamic metadata path are enough for its runtime goals. Adding Codex-scale cache orchestration would increase complexity without improving core agent behavior proportionally.

2. Krusty does not expose an OpenCode-style deep plugin transform chain around every provider request.
Reason:
Krusty deliberately prefers typed canonical seams and model-profile overlays over open-ended transform layering. This keeps behavior more explicit and easier to reason about while still supporting the main provider differences.

3. Krusty does not use a separate standalone tool-orchestrator type like Codex.
Reason:
The same governance outcomes are already centralized across `tool_control`, `hooks`, `registry`, and delegated policy contracts. Splitting that into another top-level orchestrator object would add a new abstraction without clearly improving behavior.

## Closure judgment

Roadmap closure is justified.

The remaining differences versus OpenCode, pi-mono, and Codex are now mostly shape differences, not missing core behaviors. Where Krusty is simpler, that simplicity is intentional and aligned with the project’s design goal: small footprint, modularity, and professional-grade coding-agent performance without unnecessary machinery.

## Direct source anchors used for this audit

- Krusty:
  - `crates/krusty-core/src/agent/orchestrator.rs`
  - `crates/krusty-core/src/agent/context_ledger.rs`
  - `crates/krusty-core/src/agent/compaction.rs`
  - `crates/krusty-core/src/agent/tool_control.rs`
  - `crates/krusty-core/src/agent/subagent/execution.rs`
  - `crates/krusty-core/src/plan/lifecycle.rs`
  - `crates/krusty-core/src/storage/recovery.rs`
  - `crates/krusty-core/src/storage/runtime_traces.rs`
  - `crates/krusty-core/src/tools/registry.rs`
  - `crates/krusty-core/src/ai/client/core.rs`
  - `crates/krusty-core/src/ai/client/config.rs`
  - `crates/krusty-core/src/ai/model_profile.rs`
  - `crates/krusty-server/src/routes/sessions.rs`

- OpenCode:
  - `packages/opencode/src/session/prompt.ts`
  - `packages/opencode/src/session/processor.ts`
  - `packages/opencode/src/session/compaction.ts`
  - `packages/opencode/src/session/llm.ts`
  - `packages/opencode/src/session/system.ts`
  - `packages/opencode/src/provider/transform.ts`
  - `packages/opencode/src/provider/provider.ts`

- pi-mono:
  - `packages/coding-agent/src/core/agent-session.ts`
  - `packages/coding-agent/src/core/compaction/compaction.ts`
  - `packages/coding-agent/src/core/system-prompt.ts`
  - `packages/coding-agent/src/core/model-registry.ts`
  - `packages/coding-agent/src/core/model-resolver.ts`
  - `packages/ai/src/providers/openai-responses.ts`
  - `packages/ai/src/providers/transform-messages.ts`

- Codex:
  - `codex-rs/core/src/codex.rs`
  - `codex-rs/core/src/compact.rs`
  - `codex-rs/core/src/context_manager/history.rs`
  - `codex-rs/core/src/tools/orchestrator.rs`
  - `codex-rs/core/src/models_manager/manager.rs`
  - `codex-rs/core/src/client_common.rs`
  - `codex-rs/core/src/contextual_user_message.rs`
