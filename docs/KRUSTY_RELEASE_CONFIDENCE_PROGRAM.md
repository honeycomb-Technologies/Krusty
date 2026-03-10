# Krusty Release Confidence Program

Last updated: 2026-03-09
Owner: Runtime hardening program
Status: Active

## Purpose

Turn the post-roadmap verification work into a repeatable release gate so Krusty stays verified after the one-time hardening effort.

This document defines:
- release checklist
- replay/workload ownership
- regression gate expectations
- operator runbook
- go/no-go rules

## Release Checklist

A release is only eligible when all of the following are true:

1. Validation passes with no warnings:
   - `cargo fmt --all --check`
   - `cargo check --workspace`
   - `cargo clippy --workspace -- -D warnings`
   - `cargo test --workspace`
   - `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run check`
   - `cd apps/pwa/app && BUN_TMPDIR=/tmp/bun-tmp BUN_INSTALL=/tmp/bun-install /home/burgess/.bun/bin/bun run build`

2. Replay/workload pack passes:
   - long-session compaction scenario
   - approval pause/resume scenario
   - interruption recovery scenario
   - loop-guard rejection scenario

3. Core watchpoints remain green:
   - context continuity and recovery
   - server contract fidelity
   - surface parity
   - workload replay confidence
   - security/ops confidence

4. No unresolved high-severity finding exists in:
   - orchestration
   - tool governance
   - session ownership
   - remote access boundaries
   - credential handling

5. Intentional deltas are still intentional:
   - no accidental architecture drift against [COMPARISON.md](/home/burgess/Work/krusty/crates/krusty-core/COMPARISON.md)
   - no silent reintroduction of duplicate control paths

## Replay Ownership

Replay/workload validation is owned by `krusty-core` runtime trace infrastructure in [runtime_traces.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/runtime_traces.rs).

Ownership rules:
- New control-path behavior must emit canonical `LoopEvent`s instead of creating parallel trace logic.
- New long-running or recovery-sensitive features must add or extend a replay scenario before release.
- Recovery-sensitive regressions are judged on both whole-session summary and latest-run summary when resume semantics matter.

## Regression Gate Expectations

Default release gate:
- terminal reason must be `completed` or `awaiting_input`
- zero agent errors
- zero provider failures
- zero tool execution errors
- zero server tool errors

Scenario-specific gates may also require:
- minimum run count
- minimum turn count
- minimum compaction count
- minimum awaiting-input count
- required event types

If a scenario needs looser expectations, that relaxation must be explicit in code and justified in review.

## Operator Runbook

When a regression is reported:

1. Pull the session trace summary and recent trace events from the session trace API.
2. Determine whether the failure belongs to:
   - provider interruption
   - loop guard
   - tool execution
   - ownership/auth boundary
   - compaction/recovery
3. Check whether the failing behavior is already represented in the replay pack.
4. If not, add the representative replay scenario before or alongside the fix.
5. Re-run the full validation checklist above.

When a release candidate is evaluated:

1. Confirm all validation commands pass with no warnings.
2. Confirm no route or surface bypasses the canonical core behavior owner.
3. Confirm no server edge allows unaudited remote command execution.
4. Confirm replay pack still covers compaction, pause/resume, interruption recovery, and loop containment.

## Go/No-Go Rules

`Go`:
- all checklist items pass
- no unresolved high-severity finding
- no warning-bearing validation step

`No-Go`:
- any validation warning or failure remains
- any high-severity security/ownership gap remains
- any new runtime behavior lands without canonical replay coverage

## Maintenance Rules

- Do not delete replay coverage because a bug is “obviously fixed.”
- Do not weaken the server trust boundary for convenience; add explicit configuration and audit if remote access is ever reintroduced.
- Do not add route-local policy that diverges from `krusty-core`.
- Keep this document and [KRUSTY_POST_ROADMAP_VERIFICATION_TRACKER.md](/home/burgess/Work/krusty/docs/KRUSTY_POST_ROADMAP_VERIFICATION_TRACKER.md) updated in the same change when release rules materially change.
