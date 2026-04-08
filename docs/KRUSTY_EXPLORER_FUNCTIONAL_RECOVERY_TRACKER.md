# Krusty Explorer Functional Recovery Tracker

## Status
Active

## Phase Tracker

| Phase | Area | Status | Notes |
|---|---|---|---|
| 1 | Runtime gap inventory | Complete | Confirmed the major gap: subagents had a thinner tool/runtime surface and weaker evidence retention than the main agent |
| 2 | Explorer capability recovery | Complete | Explorer now has `list` and retains evidence from successful `list`/`glob`/`grep`/`read` outputs |
| 3 | Specialized explorer session contract | In progress | Structured report path is now salvageable from real evidence, but the dedicated explorer runtime is still thinner than the main agent |
| 4 | Parent/subagent cooperation redesign | In progress | Child evidence is now stronger, but parent aggregation still needs more deliberate evidence-led assembly |
| 5 | Provider-specific reliability | In progress | MiniMax delegated exploration is now serialized to favor correctness over throughput |
| 6 | Server/PWA artifact recovery | Pending | Functional recovery must remain visible and truthful in UI |
| 7 | Competitive re-audit | Pending | Re-check against local reference systems after recovery work lands |

## Confirmed Findings

| ID | Severity | Finding | Status |
|---|---|---|---|
| EFR-001 | High | Explorer subagents run on a materially thinner runtime than the main agent | Open |
| EFR-002 | High | Current explorer success depends too much on model obedience and output formatting | Open |
| EFR-003 | High | Parent aggregation still depends too much on child polish instead of child evidence | Open |
| EFR-004 | Medium | MiniMax-specific delegated exploration remains unreliable even after capability recovery | Open |
| EFR-005 | Medium | MiniMax-specific delegated exploration remains unreliable even after recent hardening | Open |

## Immediate Conclusion
The explorer is not primarily broken because of one remaining bug. It is underpowered relative to the job it is being asked to perform.

## Closed In This Program

| ID | Finding | Closure |
|---|---|---|
| EFR-C001 | Explorer subagents lacked the `list` capability that the main agent relies on for fast scope grounding | Fixed in `crates/krusty-core/src/agent/subagent/tools.rs` and `crates/krusty-core/src/agent/subagent/types.rs` |
| EFR-C002 | Successful `list`/`glob`/`grep`/`read` outputs were not retained strongly enough as explorer evidence | Fixed in `crates/krusty-core/src/agent/subagent/execution.rs` |
| EFR-C003 | Missing structured reports caused immediate failure even when real evidence existed | Fixed by synthesizing a valid `<explore_report>` from real evidence in `crates/krusty-core/src/agent/subagent/types.rs` and `crates/krusty-core/src/agent/subagent/execution.rs` |
| EFR-C004 | MiniMax delegated fanout was still too aggressive for reliable exploration | Fixed by serializing MiniMax explore concurrency in `crates/krusty-core/src/tools/implementations/explore.rs` |
