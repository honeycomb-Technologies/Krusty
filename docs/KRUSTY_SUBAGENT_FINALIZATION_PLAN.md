# Krusty Subagent Finalization Plan

## Goal
Finish Krusty's subagent system so it is:
- first-class like OpenCode's task agents
- reliable across codebases at the runtime level
- insightful at the review level
- coherent across core, server, and PWA

This plan is based on direct comparison with OpenCode's delegated task model in:
- `/home/burgess/Work/opencode/packages/opencode/src/tool/task.ts`
- `/home/burgess/Work/opencode/packages/opencode/src/agent/prompt/explore.txt`

## Comparison Summary

### What OpenCode does well
- A subagent is a real child session, not just a smaller loop.
- Delegation has explicit parent linkage (`parentID`) and resumability (`task_id`).
- Delegation has a specialized agent type (`subagent_type`) instead of a generic helper role.
- Child permissions are explicit and narrow.
- Child output has one clean boundary (`<task_result>...</task_result>`).
- The parent is responsible for summarizing child work for the user.

### Where Krusty still falls short
- Explorer/build subagents are still lightweight custom loops in `crates/krusty-core/src/agent/subagent/`.
- Delegated runs are identifiable, but not yet session-grade runtime units.
- Parent aggregation still depends too much on child prose quality.
- Review quality is improving, but still degrades into structural/file coverage too often.
- Broad delegated audits still need better batching/orchestration semantics than "many child tasks inside one tool call."

## Final Design Target

Krusty subagents should become:
- first-class delegated runs with stable identity and parent linkage
- resumable delegated contexts when the task is worth continuing
- specialized delegated roles (`explore`, `build`, later others)
- evidence-first child runtimes
- parent-owned user-facing synthesis

The child should gather and return evidence.
The parent should decide what to show the user.

## Non-Goals
- Do not duplicate the full main orchestrator inside every subagent.
- Do not make all subagents full write-capable agents by default.
- Do not hardcode Krusty-specific audit rules into the generic delegated runtime.

## Phase 1: Delegated Runtime Unit
Replace "anonymous helper loop" semantics with a first-class delegated runtime unit.

### Required changes
- Formalize delegated run identity as more than `delegated_run_id`.
- Add delegated run metadata contract:
  - parent session id
  - delegated run id
  - delegated role
  - target scope
  - resumable or ephemeral
  - provider/model used
- Define a delegated run lifecycle:
  - created
  - running
  - synthesizing
  - completed
  - degraded
  - failed
  - cancelled

### Why
OpenCode's biggest advantage is not just prompt wording. It is that delegation is treated as a first-class runtime object.

### Exit criteria
- Delegated runs are modeled explicitly across core, server, and PWA.

## Phase 2: Child Session Semantics
Move explorer/build closer to child-session semantics without forcing full orchestrator duplication.

### Required changes
- Give each delegated run a persistent child context contract.
- Preserve child-local evidence and progress independent of parent text history.
- Allow resuming a delegated run when the parent decides continuation is useful.
- Keep delegated runs read-only unless the delegated role explicitly allows writes.

### Why
The current loop is too disposable. OpenCode's `task_id` model is stronger because the child is not recreated as a stateless helper every time.

### Exit criteria
- Long-running or interrupted delegated investigations can resume meaningfully.

## Phase 3: Role-Specific Delegated Agents
Stop treating subagents as one generic delegated loop with one generic review prompt.

### Required changes
- Define role-specific delegated contracts:
  - `explore`
  - `build`
  - future `review`, `search`, etc.
- Each role gets:
  - its own system prompt
  - its own completion contract
  - its own evidence rubric
  - its own stop conditions

### Why
OpenCode's explore prompt is narrow and specialized. Krusty still overloads one delegated runtime too much.

### Exit criteria
- `explore` and `build` are no longer just policy variants on the same thin behavior.

## Phase 4: Evidence Contract Completion
Make the child output contract good enough that the parent can trust evidence without trusting child polish.

### Required changes
- Keep the structured child report contract.
- Require child evidence to distinguish:
  - structural coverage
  - semantic coverage
  - strengths
  - gaps
  - confidence
- Make target-specific expectations pluggable by target class:
  - orchestration/runtime
  - provider/client
  - persistence/storage
  - server/API
  - UI/surface

### Why
A codebase-agnostic audit still needs target-class-aware review criteria. Universal runtime does not mean universal review rubric.

### Exit criteria
- A child report can be judged on evidence quality, not just "did it produce text."

## Phase 5: Parent-Owned Review Synthesis
Make the parent the authoritative review writer.

### Required changes
- Parent consumes child evidence artifacts, not child summary prose alone.
- Final user-facing audit should contain:
  - executive summary
  - per-target review
  - cross-cutting strengths
  - cross-cutting weaknesses
  - explicit coverage caveat
- Runtime telemetry stays in metadata/state, not main prose.

### Why
OpenCode's `task_result` boundary is strong because the parent decides what the user sees. Krusty still lets child polish leak upward too directly.

### Exit criteria
- User-facing broad audits read like architecture reviews, not tool-status summaries.

## Phase 6: Broad Audit Orchestration
Treat broad audits as coordinated delegated programs, not just many child targets stuffed into one tool invocation.

### Required changes
- Keep batching.
- Add explicit batch-level progress and aggregation.
- Allow the parent to reassess coverage between batches.
- Stop or narrow later batches when earlier coverage already shows thin areas that need deeper inspection.

### Why
Broad multi-target audits are where current quality still falls off.

### Exit criteria
- Broad audits stay stable and become progressively more insightful, not just longer.

## Phase 7: Provider Reliability Layer
Treat provider-specific delegated behavior as a first-class policy layer.

### Required changes
- Keep provider-aware batching/concurrency.
- Add provider-aware delegated completion behavior:
  - stricter placeholder rejection
  - stronger prompt narrowing
  - narrower task scopes for weaker providers
- Allow delegated role/model policy independent of the main agent when justified.

### Why
MiniMax has repeatedly exposed delegated weaknesses. Runtime quality and provider policy need a clean seam.

### Exit criteria
- Delegated behavior is robust without pretending all providers behave equally.

## Phase 8: Server and PWA Parity
Keep delegated runs first-class all the way to the user surface.

### Required changes
- Preserve delegated run identity and lifecycle in APIs.
- Preserve child evidence and parent review separately.
- Show user-facing review prominently.
- Keep raw evidence/progress/coverage metadata available but secondary.

### Why
A first-class delegated runtime is incomplete if the server/PWA flatten it back into generic tool chatter.

### Exit criteria
- Delegated runs are legible and honest across server and PWA.

## Phase 9: Validation and Closure
Do not declare subagents complete until they pass the real use cases.

### Validation scenarios
- scoped single-target explore
- dense target explore (`agent`, `ai`, `storage`)
- broad multi-target architecture audit
- interrupted delegated run
- resumed delegated run
- provider-sensitive delegated audit on MiniMax
- server/PWA reload during delegated execution

### Closure criteria
- scoped audits are reliable
- broad audits are stable
- final reviews are insightful enough to be trusted
- parent does not abandon into manual probing after successful delegation
- placeholder child summaries no longer leak into final artifacts

## Immediate Priority Order
1. Finish delegated runtime/session semantics.
2. Complete role-specific evidence rubric for `explore`.
3. Strengthen parent-owned review synthesis.
4. Re-audit broad Krusty architecture review quality.

## Definition of Done
Krusty subagents are "done" when:
- delegation is first-class
- explorer/build are role-specialized
- parent writes the final review from evidence
- broad audits are both stable and useful
- server/PWA show the delegated system cleanly

