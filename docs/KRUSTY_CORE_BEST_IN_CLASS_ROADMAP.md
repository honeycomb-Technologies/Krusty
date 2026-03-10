# Krusty Core Best-In-Class Roadmap

Last updated: 2026-03-09
Owner: Core runtime program
Status: Active

## Goal

Make Krusty best in class across core architecture and runtime behavior when compared against top coding-agent systems, while preserving Krusty's small-footprint modular design.

This roadmap is phase-gated. No phase advances until its backcheck is complete.

## Non-Negotiables

- No hidden behavior caps.
- One canonical policy path per concern.
- Cross-surface parity (CLI/TUI/ACP/server/subagents).
- Reliability over novelty.
- Elegance without regressions in quality or performance.

## Coverage Matrix

This roadmap explicitly covers all core areas:

| Core area | Covered in phase(s) |
| --- | --- |
| Orchestrator state machine | 1, 2 |
| Context ledger + compaction continuity | 1 |
| Prompt architecture and capability packs | 2 |
| Provider normalization + model registry policy | 2 |
| Tool runtime, approval, sandbox, escalation | 3 |
| History shaping and evidence contracts | 3 |
| Persistence, session recovery, resume | 4 |
| CLI/TUI/ACP/server parity | 4 |
| Subagent governance | 5 |
| MCP/skills/plugins/extensions governance | 5 |
| Planning and task execution rigor | 6 |
| Observability, replay, evaluation discipline | 7 |
| Simplification and deletion pass | 8 |
| Competitive final audit | 9 |

## Phase Plan

## Phase 0: Program Definition and Baseline

Purpose: freeze target architecture and scoring model so execution is measurable.

Deliverables:
- Master scorecard for every core subsystem.
- Entry/exit criteria for each phase.
- Backcheck template used after every phase.
- Risk register and rollback rules.

Exit gate:
- Every subsystem has `current state`, `target state`, `owner`, and `phase mapping`.
- Every phase has objective pass/fail conditions.

## Phase 1: Context Engine and Deterministic Continuation

Purpose: eliminate context identity drift across compaction and interruption.

Deliverables:
- Context ledger model: canonical/summarized/dropped/pinned/replayed.
- Deterministic continuation contract for interruption and overflow.
- Resume behavior is explicit and auditable.

Exit gate:
- Long-running sessions preserve task identity under compaction and restart.

## Phase 2: Canonical AI Execution Pipeline

Purpose: remove fragmented request/prompt/provider paths.

Deliverables:
- One canonical pipeline for streaming/simple/tools/subagents/ACP/server.
- Provider transforms and capability policies applied consistently.
- Prompt-pack assembly path shared across all AI call surfaces.

Exit gate:
- No major AI request path bypasses canonical policy pipeline.

## Phase 3: Tool and Sandbox Maturity

Purpose: match professional-grade tool policy depth.

Deliverables:
- Full tool policy matrix (read/write/network/process/git/elevated).
- Unified escalation and approval outcomes.
- Extended evidence contracts for all high-impact tools.

Exit gate:
- All tool executions follow one policy engine with parity across surfaces.

## Phase 4: Persistence, Recovery, and Surface Parity

Purpose: make interrupted work safe and consistent.

Deliverables:
- Crash-safe partial-turn persistence.
- Resume invariants for CLI/TUI/ACP/server.
- Explicit non-resumable reasons when replay is unsafe.

Exit gate:
- Recovery behavior is deterministic and aligned across all entrypoints.

## Phase 5: Subagents and Extensibility Unification

Purpose: unify subagents, MCP, skills, plugins, and extensions under core policy.

Deliverables:
- Context/approval/quota inheritance rules.
- Failure containment and audit trail for delegated execution.
- Result normalization for extensible tool surfaces.

Exit gate:
- No delegated path can bypass core policy or degrade context integrity silently.

## Phase 6: Planning and Execution Discipline

Purpose: tighten plan-state continuity and execution quality.

Deliverables:
- Durable plan lifecycle rules.
- Strong mode transitions (plan <-> build).
- Completion/actionability checks and consistency guarantees.

Exit gate:
- Plan execution quality is stable across resumes and long sessions.

## Phase 7: Observability, Replay, and Evaluations

Purpose: move from intuition to measurement.

Deliverables:
- Structured runtime traces and failure taxonomy.
- Replay suites for long runs, compaction, tool loops, provider edge cases.
- Regression gating aligned to quality, reliability, and continuity outcomes.

Exit gate:
- Changes can be approved/rejected by metrics and replay results.

## Phase 8: Elegance and Deletion Pass

Purpose: reduce complexity without reducing capability.

Deliverables:
- Remove duplicate execution paths.
- Merge overlapping abstractions.
- Keep one owner module per concern.

Exit gate:
- Simpler architecture, equal or better reliability and performance.

## Phase 9: Final Competitive Audit and Closure

Purpose: verify best-in-class status against leading systems.

Deliverables:
- Full cross-core comparison against top competitors.
- Remaining deltas either closed or explicitly accepted as intentional.
- Final closure report.

Exit gate:
- Every core domain at parity or advantage by design.

## Backcheck (Required After Every Phase)

1. Architecture backcheck: Did we simplify or just add machinery?
2. Behavior backcheck: Are edge cases and failure paths deterministic?
3. Parity backcheck: Do all relevant surfaces obey the same rules?
4. Deletion backcheck: What can be removed now?
5. Competitive backcheck: Which competitor gap was closed measurably?

No phase advancement until all five backchecks are complete and recorded.

## Execution Rules

- Never proceed on "looks good"; proceed only on explicit pass criteria.
- Every phase must include rollback notes.
- Every new policy path needs focused tests.
- Every completion claim must include proof artifact references.

