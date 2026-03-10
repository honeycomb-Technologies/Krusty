# Krusty Post-Roadmap Verification Plan

Last updated: 2026-03-09
Owner: Runtime hardening program
Status: Active

## Goal

Verify that the completed core roadmap holds up under real usage, close any high-signal residual defects, and harden the surrounding server and surface layers without reopening broad architecture churn.

This program is audit-first and evidence-gated. It treats `krusty-core` as feature-complete unless a phase produces concrete evidence that a behavior gap still exists.

## Principles

- Verify before redesigning.
- Treat `krusty-core` as the canonical behavior owner.
- Expand outward from core to server to surfaces.
- Prefer replay packs, contract review, and targeted findings over broad speculative rewrites.
- Do not copy competitor subsystem shape unless Krusty is missing the underlying runtime outcome.

## Coverage Matrix

| Area | Covered in phase(s) |
| --- | --- |
| Core control-path watchdog audit | 1 |
| Server/API contract fidelity | 2 |
| Surface parity across CLI/TUI/ACP/PWA/Desktop | 3 |
| Real-world workload and replay validation | 4 |
| Security and operational hardening | 5 |
| Release confidence and ship criteria | 6 |

## Phase Plan

## Phase 1: Core Watchdog Audit

Purpose: review the highest-risk `krusty-core` control paths after roadmap closure.

Scope:
- orchestrator loop
- stream processing and recovery
- compaction and continuation
- tool fail-fast / approval control
- planning lifecycle
- subagent containment
- runtime trace capture

Deliverables:
- Focused findings log with severity and file references.
- Decision on whether any core fixes are required before expanding outward.
- Regression pack list for validated edge cases.

Exit gate:
- No unresolved high-severity defects in core control paths.
- Any medium-severity issues are either fixed or explicitly deferred with rationale.

## Phase 2: Server/API Audit

Purpose: verify `krusty-server` is a faithful transport/access layer over core behavior.

Scope:
- session routes
- chat/stream routes
- tool routes
- auth and ownership checks
- session state / trace endpoints

Deliverables:
- Contract matrix for request/response behavior vs core semantics.
- Findings on ownership drift, state drift, and streaming contract mismatches.

Exit gate:
- Server behavior does not re-implement or contradict core policy.

## Phase 3: Surface Parity Audit

Purpose: verify each interface obeys the same semantics.

Scope:
- CLI
- TUI
- ACP
- PWA/Desktop integration surfaces

Deliverables:
- Parity matrix covering session start/resume, work-mode changes, planning, tool approvals, recovery, and trace visibility.

Exit gate:
- No major semantic drift between surfaces for the same underlying action.

## Phase 4: Workload and Replay Validation

Purpose: validate behavior under representative real-world coding-agent workloads.

Scope:
- long coding sessions
- compaction edge cases
- repeated tool loops
- provider interruptions
- delegated builder/explore flows
- recovery after interruption

Deliverables:
- Curated replay/workload pack.
- Pass/fail summary with repeated failure signatures.

Exit gate:
- Stable pass rate and no recurring high-severity failure signature across the representative workload set.

## Phase 5: Security and Operations Audit

Purpose: verify operational safety, approval boundaries, and recoverability.

Scope:
- sandbox boundaries
- destructive tool approval behavior
- multi-tenant session ownership
- credential handling
- rollback and operator visibility

Deliverables:
- Security/ops findings log.
- Required mitigations or explicit accepted risks.

Exit gate:
- No unresolved high-risk security or operational control gap.

## Phase 6: Release Confidence Program

Purpose: convert the verification results into a sustainable release decision process.

Scope:
- release checklist
- replay ownership
- regression gating expectations
- operator runbooks
- intentional-delta review

Deliverables:
- Final release-confidence checklist.
- Maintenance rules for keeping the system verified after this program completes.

Exit gate:
- Clear go/no-go criteria exist for future releases and regressions are measurable.

## Backcheck (Required After Every Phase)

1. Architecture backcheck: Did we preserve the core’s canonical ownership boundaries?
2. Behavior backcheck: Are failure paths deterministic and verified?
3. Parity backcheck: Do adjacent surfaces still obey the same rules?
4. Deletion backcheck: What repeated or obsolete logic can now be removed?
5. Confidence backcheck: What evidence now exists that did not exist before this phase?

## Execution Rules

- Do not broaden scope without a concrete finding.
- No major architecture work unless a phase produces evidence that the current design fails its target behavior.
- Every finding must include file references and an explicit severity.
- Every phase must end with a go/no-go decision for advancing to the next phase.
