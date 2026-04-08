# Krusty Subagent Redesign Tracker

## Status

- Phase 1: Complete
- Phase 2: Complete
- Phase 3: Complete
- Phase 4: Complete
- Phase 5: Complete
- Phase 6: Complete
- Phase 7: Complete
- Phase 8: Complete

## Open Findings

### SRD-001: Runtime parity gap

Explorer subagents still run on a thinner runtime than the main agent, which is the largest architectural weakness.

Status: Open
Note: Functional recovery is complete, but main-agent/subagent runtime parity is still a longer-term architecture gap.

### SRD-002: Child evidence contract too weak

Delegated child output still depends too much on polished final prose instead of robust structured evidence.

Status: Closed

### SRD-003: Parent aggregation too child-summary dependent

Parent `explore` still over-relies on child summary quality instead of aggregating evidence directly.

Status: Closed

### SRD-004: Directory structure evidence underweighted

Non-empty directory listings and structure evidence are still too easy to misread or undercount.

Status: Closed

### SRD-005: Partial delegated runs still overclaim

Parent responses can still sound more complete than the delegated evidence warrants.

Status: Closed
Note: Successful single-`explore` turns now finalize directly from delegated evidence instead of giving the parent another model turn to overclaim or relaunch the tool.

### SRD-006: Delegated runtime is not first-class

Explorer children are still implementation-detail loops rather than first-class delegated runtime units with stronger parent linkage and resumable state.

Status: Open
Note: Delegated runs now carry first-class run identity and parent linkage, but they are not yet full child sessions with independent persistence semantics.

### SRD-007: Parent fallback remains too eager

Even when delegated work is partially useful, the parent can still drift into manual probing too quickly.

Status: Closed

## Comparison Anchors

- Krusty explorer:
  - `crates/krusty-core/src/tools/implementations/explore.rs`
  - `crates/krusty-core/src/agent/subagent/execution.rs`
  - `crates/krusty-core/src/agent/subagent/types.rs`
  - `crates/krusty-core/src/agent/subagent/tools.rs`
- OpenCode:
  - `/home/burgess/Work/opencode/packages/opencode/src/tool/task.ts`
  - `/home/burgess/Work/opencode/packages/opencode/src/agent/prompt/explore.txt`
- pi:
  - `/home/burgess/Work/pi-mono/packages/coding-agent/README.md`

## Phase 1 Deliverable

- [KRUSTY_SUBAGENT_RUNTIME_GAP_MATRIX.md](/home/burgess/Work/krusty/docs/KRUSTY_SUBAGENT_RUNTIME_GAP_MATRIX.md)

## Phase 2 Deliverable

- [KRUSTY_FIRST_CLASS_DELEGATED_RUN_MODEL.md](/home/burgess/Work/krusty/docs/KRUSTY_FIRST_CLASS_DELEGATED_RUN_MODEL.md)

## Phase 3 Deliverable

- [KRUSTY_DELEGATED_EVIDENCE_CONTRACT.md](/home/burgess/Work/krusty/docs/KRUSTY_DELEGATED_EVIDENCE_CONTRACT.md)

## Phase 4 Deliverable

- [KRUSTY_EXPLORER_CHILD_RUNTIME_UPGRADE.md](/home/burgess/Work/krusty/docs/KRUSTY_EXPLORER_CHILD_RUNTIME_UPGRADE.md)

## Phase 5 Deliverable

- [KRUSTY_PARENT_AGGREGATION_REWRITE.md](/home/burgess/Work/krusty/docs/KRUSTY_PARENT_AGGREGATION_REWRITE.md)

## Phase 6 Deliverable

- [KRUSTY_PROVIDER_RELIABILITY_LAYER.md](/home/burgess/Work/krusty/docs/KRUSTY_PROVIDER_RELIABILITY_LAYER.md)

## Phase 7 Deliverable

- [KRUSTY_SUBAGENT_SURFACE_PARITY.md](/home/burgess/Work/krusty/docs/KRUSTY_SUBAGENT_SURFACE_PARITY.md)

## Phase 1 Findings

### SRD-P1-001: Explorer lacks session-grade delegated runtime identity

Main agent runs are durable and trace-backed; explorer children are still implementation-detail loops.

Status: Open

### SRD-P1-002: Explorer lacks layered context parity with main agent

Explorer children rely mostly on a single custom prompt instead of the full context stack.

Status: Open

### SRD-P1-003: Parent consumes delegated work as a tool result, not a first-class delegated run

This leaves parent/subagent cooperation too fragile and summary-dependent.

Status: Open

### SRD-P1-004: Directory structure evidence is under-modeled

Explorer children can still misread non-empty directory state and fail to treat it as usable architecture evidence.

Status: Open

## Phase 2 Findings

### SRD-P2-001: Delegated work lacked stable run identity

Closed by introducing `delegated_run_id` through child task, child result, progress event, history summary, and server snapshot contracts.

Status: Closed

### SRD-P2-002: Delegated runtime lacked explicit parent linkage at progress/snapshot level

Closed by carrying `parent_session_id` through delegated progress and server snapshot state.

Status: Closed

## Phase 3 Findings

### SRD-P3-001: Delegated evidence semantics were too prose-dependent

Closed by introducing an explicit evidence artifact in `SubAgentResult` and using it as the canonical child evidence shape.

Status: Closed

### SRD-P3-002: Directory structure evidence was under-modeled

Closed at the contract level by separating `paths_examined`, `files_examined`, and `directories_examined`.

Status: Closed

## Phase 4 Findings

### SRD-P4-001: Directory evidence was still being flattened inside child runtime

Closed by preserving directory markers from `list` output and allowing child reports to treat `paths_examined` as first-class evidence.

Status: Closed

### SRD-P4-002: Child report repair was still concrete-file biased

Closed by switching repair and synthesis paths to path-backed evidence instead of requiring concrete file reads in every successful architecture exploration.

Status: Closed

## Phase 5 Progress

### SRD-P5-001: Parent aggregation lacked explicit coverage map

Closed by adding `coverage.status`, `usable_targets`, `degraded_targets`, and `failed_targets` to explore aggregation and preserving that through history shaping.

Status: Closed

### SRD-P5-002: Parent aggregation lacked deterministic investigation/gap summaries

Partially closed by adding `investigation_summary`, `confidence`, and `coverage_gap_notice` to parent explore output and preserving them through history shaping.

Status: Closed

### SRD-P5-003: Parent fallback remained too eager after usable partial delegation

Partially closed by adding a loop guard that stops broad read-only fallback after a usable delegated exploration already instructed the parent to summarize.

Status: Closed

## Phase 6 Progress

### SRD-P6-001: Delegated provider shaping was still too generic

Partially closed by making the explorer child assignment provider-aware, with a narrower MiniMax workflow layered on top of the existing concurrency and stagger controls.

Status: Closed

## Phase 8 Closure

### SRD-P8-001: Live verification of delegated `explore`

Closed by live server verification on March 11, 2026 against session `714ba029-612c-4a89-aceb-29d093c1f09d`.

Verified behavior:
- one top-level `explore` call
- no second phantom `explore` retry
- no fallback into broad manual reads
- delegated children stayed scoped to bound targets
- parent finished in a single turn with a deterministic evidence-based summary
- session trace ended with `text_delta`, `turn_complete(has_more=false)`, and `finished(completed)`

Status: Closed

## Phase 7 Progress

### SRD-P7-001: Surface rendering lagged behind parent aggregation truth

Partially closed by surfacing `investigation_summary`, `confidence`, and `coverage_gap_notice` in the PWA delegated artifact model and widget.

Status: In progress

## Closure Rule

This tracker is only complete when:
- the redesign phases are all closed
- the delegated runtime contract is coherent end to end
- explorer is functionally reliable again
- the parent/subagent cooperation path is trustworthy in real runs
- Krusty's delegated architecture is demonstrably closer to OpenCode's first-class model than to the current thin-loop model
