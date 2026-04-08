# Krusty Delegated Exploration Hardening Roadmap

## Goal

Make delegated `explore` behave like a professional investigation workflow:

- starts fast
- gathers evidence
- converges decisively
- returns one clear artifact
- stops when more exploration is no longer buying signal
- avoids wasteful background drain after disconnect

## Problem Statement

Recent live runs showed a specific failure class:

- delegated sub-agents keep succeeding
- but they do not converge into a summary
- message history keeps growing
- the server continues draining the run after client disconnect
- memory stays bounded for now, but the workload remains structurally unhealthy

This is a successful-non-converging delegated exploration problem, not a simple crash path.

## Phase 1: Convergence Contract

Define explicit explorer stopping behavior.

Changes:
- strengthen explorer system prompt so agents stop once they have enough evidence
- make evidence sufficiency explicit: architecture, key modules, patterns, concerns, references
- instruct explorers to summarize instead of continuing blind reads once the high-level picture is clear

Exit:
- explorer prompt encodes sufficiency and stopping rules directly

## Phase 2: Successful-Stale Delegated Detection

Add delegated convergence detection for successful but low-value churn.

Changes:
- detect repeated successful read/glob/grep cycles without convergence
- detect high token/tool growth without completion summary emergence
- detect repeated broad exploration patterns with little new evidence

Exit:
- delegated runs can terminate by graceful convergence, not only by hard failure

## Phase 3: Evidence Sufficiency Heuristics

Teach `explore` what “enough architecture coverage” means.

Changes:
- track representative coverage of repo structure and subsystem entrypoints
- favor synthesis once the run has enough evidence to answer the original prompt
- keep the heuristics generic, not repo-specific

Exit:
- broad architecture exploration stops after representative coverage

## Phase 4: Orphaned Delegated Run Policy

Handle disconnected long exploration more intelligently.

Changes:
- distinguish active-viewer delegated runs from orphaned delegated runs
- when an `explore` run is orphaned, apply stricter convergence behavior
- force summarize rather than drain indefinitely when the user is gone and the run is stale

Exit:
- orphaned delegated exploration does not continue indefinitely in the background

## Phase 5: Delegated Result Semantics

Preserve accurate stop reasons in structured results.

Changes:
- enrich delegated result metadata with stop reason and degradation cause
- distinguish useful partial/degraded summaries from full success and full failure

Exit:
- delegated artifacts explain why they stopped, not just whether they “worked”

## Phase 6: Delegated Identity and UX Clarity

Make delegated progress legible across server/PWA.

Changes:
- derive non-ambiguous delegated agent labels
- surface phase-style progress states where possible
- keep one top-level delegated artifact clear and singular

Exit:
- delegated state is readable at a glance

## Phase 7: Delegated Trace and Memory Observability

Improve delegated runtime forensics.

Changes:
- capture delegated-run memory watermarks and progression
- expose convergence-related metadata in trace/session state where practical
- make future “it felt frozen” reports inspectable from normal APIs

Exit:
- delegated runs are measurable and auditable

## Phase 8: Recovery and Cleanup Semantics

Make interrupted delegated exploration leave clean residue.

Changes:
- align delegated cleanup with recovery state
- clear stale delegated runtime state on restart
- preserve only useful resumable evidence

Exit:
- delegated interruption does not leave misleading active state behind

## Phase 9: Validation Matrix

Backcheck with:

- normal multi-directory architecture explore
- partially successful explore
- failed explore
- disconnect during explore
- reconnect during explore
- fully orphaned explore
- memory stability during long delegated run

Exit:
- delegated exploration is stable, bounded, and legible

## Phase 10: Generalization Pass

Apply the same delegated control logic to `build` where it fits.

Exit:
- delegated control policy is coherent across delegated surfaces
