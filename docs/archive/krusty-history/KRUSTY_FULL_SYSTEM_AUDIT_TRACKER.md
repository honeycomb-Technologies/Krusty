# Krusty Full System Audit Tracker

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Status
Active

## Audit Phases

| Phase | Area | Status | Notes |
|---|---|---|---|
| 0 | Repository inventory and audit framing | Complete | Source inventory captured and master audit docs created |
| 1 | `krusty-core` runtime coherence | In progress | Existing delegated exploration findings need closure |
| 2 | AI/provider layer | Pending | Tool-call transform and provider drift pass still needed |
| 3 | Tools and governance | Pending | Neutral/project/delegated truth pass still needed |
| 4 | Storage, recovery, and trace truth | Pending | Startup reconciliation and persistence truth pass still needed |
| 5 | Server and control plane | Pending | Auth, SSE, remote access, presence, operator surfaces |
| 6 | Surface parity across TUI/PWA/mobile/desktop | Pending | Delegated artifacts, plans, recovery, approvals, workspace state |
| 7 | Performance and memory | Pending | Runtime growth, caches, session history growth, backpressure |
| 8 | Product semantics and UX truth | Pending | Status language, project semantics, error truthfulness |
| 9 | Final closure report | Pending | Cross-subsystem closure and residual risk summary |

## Open Findings Backlog

| ID | Severity | Area | Finding | Status | Primary Files |
|---|---|---|---|---|---|
| FSA-001 | High | Delegated exploration | `explore` can return empty evidence while reporting `success` | Open | `crates/krusty-core/src/tools/implementations/explore.rs` |
| FSA-002 | High | Delegated exploration | successful-but-non-convergent subagent runs can continue too long | Open | `crates/krusty-core/src/agent/subagent/execution.rs` |
| FSA-003 | High | Performance/memory | delegated exploration memory risk not fully closed after prior 22 GB run | Open | `crates/krusty-core/src/agent/cache.rs`, `crates/krusty-core/src/agent/subagent/execution.rs` |
| FSA-004 | Medium | Product semantics | delegated outcome states still need final truthfulness tightening | Open | `crates/krusty-core/src/tools/implementations/explore.rs`, `apps/pwa/app/src/lib/components/chat/DelegatedToolWidget.svelte` |

## Closed Findings Seed

| ID | Area | Finding | Closure |
|---|---|---|---|
| FSA-C001 | Provider transform | MiniMax tool-call ID sanitization collisions | Fixed in `crates/krusty-core/src/ai/transform.rs` |
| FSA-C002 | Tools | `bash` join-handle panic | Fixed in `crates/krusty-core/src/tools/implementations/bash.rs` |
| FSA-C003 | Server transport | SSE disconnect orphaned active runs | Fixed in `crates/krusty-server/src/routes/chat.rs` |
| FSA-C004 | Remote auth | PWA standalone remote token bootstrap failure | Fixed in `crates/krusty-server/src/routes/remote_auth.rs` and PWA bootstrap logic |
| FSA-C005 | Workspace semantics | no-project sessions inherited repo identity | Fixed with explicit workspace-mode contract |

## Verification Standard
No finding moves to `Closed` without:
- code change or explicit intentional-decision record
- targeted validation
- clean repository-level validation where the change radius requires it
- tracker note updated with closure evidence
