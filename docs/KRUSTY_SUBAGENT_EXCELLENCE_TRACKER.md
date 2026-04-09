# Krusty Subagent Excellence Tracker

## Status
Active

## Phase Tracker

| Phase | Area | Status | Notes |
|---|---|---|---|
| 1 | Scope binding | Complete | Real target resolution and working-dir binding landed |
| 2 | Tool-truth grounding | Complete | Prompt grounding + runtime correction guard landed |
| 3 | Delegated result truthfulness | Complete | Outcome reasons and evidence-aware classification landed |
| 4 | Specialized explorer contract | Complete | Explorer now requires a structured `<explore_report>` artifact, target preflight brief, and file-backed success |
| 5 | Server and PWA delegated semantics | Complete | UI now carries the stronger delegated failure class including missing structured reports |
| 6 | Provider-aware delegation policy | In progress | MiniMax concurrency tightened, but broader provider validation is still needed |
| 7 | Traceability and auditability | Pending | Reason codes not yet fully persisted in trace/session APIs |
| 8 | Final competitive audit | Pending | Re-compare after the remaining phases land |

## Open Findings

| ID | Severity | Finding | Status |
|---|---|---|---|
| SAE-001 | High | Delegated explorer architecture is still too generic compared to dedicated subagent/task systems | Open |
| SAE-002 | High | MiniMax delegated exploration remains unproven after structural fixes | Open |
| SAE-003 | Medium | Delegated reason signals are not yet fully persisted deeply enough for postmortem-grade auditability | Open |
| SAE-004 | Medium | Explorer still needs a full post-redesign competitive re-audit against local reference systems | Open |

## Closed In This Program

| ID | Finding | Closure |
|---|---|---|
| SAE-C001 | Explore targets were prompt-only, not bound execution scope | Fixed in `crates/krusty-core/src/tools/implementations/explore.rs` |
| SAE-C002 | Explorer agents could claim tools returned nothing despite real output | Fixed in `crates/krusty-core/src/agent/subagent/types.rs` and `crates/krusty-core/src/agent/subagent/execution.rs` |
| SAE-C003 | Degraded delegated runs lacked explicit reason codes | Fixed in `crates/krusty-core/src/agent/subagent/types.rs` and `crates/krusty-core/src/tools/implementations/explore.rs` |
| SAE-C004 | PWA delegated widget could not explain delegated failure class | Fixed in `apps/pwa/app/src/lib/stores/session.ts` and `apps/pwa/app/src/lib/components/chat/DelegatedToolWidget.svelte` |
| SAE-C005 | Vague prose could still count as usable delegated evidence | Fixed by requiring structured `<explore_report>` output in `crates/krusty-core/src/agent/subagent/types.rs` and normalizing explorer completion in `crates/krusty-core/src/agent/subagent/execution.rs` |
| SAE-C006 | Explorer agents lacked target-specific preflight grounding | Fixed in `crates/krusty-core/src/tools/implementations/explore.rs` |
