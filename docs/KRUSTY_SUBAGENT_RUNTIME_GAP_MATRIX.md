# Krusty Subagent Runtime Gap Matrix

## Purpose

Phase 1 deliverable for the subagent redesign roadmap.

This document inventories the concrete runtime differences between:
- the main agent loop
- the explorer subagent runtime
- the builder subagent runtime

It separates:
- intentional differences
- accidental differences
- differences that are currently harming explorer reliability

## Scope

Reviewed Krusty files:
- [orchestrator.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/orchestrator.rs)
- [context.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/context.rs)
- [executor.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/executor.rs)
- [history_policy.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/history_policy.rs)
- [failure.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/failure.rs)
- [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs)
- [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs)
- [types.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/types.rs)
- [tools.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/tools.rs)
- [registry.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/registry.rs)

Reference comparison:
- OpenCode [task.ts](/home/burgess/Work/opencode/packages/opencode/src/tool/task.ts)
- OpenCode [explore.txt](/home/burgess/Work/opencode/packages/opencode/src/agent/prompt/explore.txt)
- pi [README.md](/home/burgess/Work/pi-mono/packages/coding-agent/README.md)

## Matrix

| Area | Main Agent | Explorer Subagent | Builder Subagent | Assessment |
|---|---|---|---|---|
| Runtime identity | Full orchestrator run with session id, recovery, trace run id, event stream | `SubAgentTask` in a lightweight pool | `SubAgentTask` in same pool | Explorer/build are implementation-detail runs, not first-class runtime units. This is a harmful gap. |
| Context injection | Full workspace, project, plan, and skills context from `inject_context()` | Custom system prompt only | Custom dynamic builder prompt only | Explorer lacks shared context layering. Harmful for reliability. |
| Conversation semantics | Canonical conversation with compaction, continuation, recovery, history shaping | Local message list inside custom loop | Same | Intentional to a point, but too thin for high-value delegated work. |
| Persistence | Durable session state, recovery state, runtime traces | No first-class delegated session persistence | No first-class delegated session persistence | Harmful. Explorer results exist only as aggregated tool artifacts, not session-grade delegated state. |
| Tool context | Full `ToolContext` with session metadata, db path, process registry, workspace mode, project dir, sandbox root, AI client | `ToolContext` rebuilt from task cwd and delegated policy; no session metadata or project dir | Same shape, plus build permissions | Explorer context is materially thinner than main agent context. Harmful. |
| Tool surface | Full tool registry | `list`, `glob`, `grep`, `read` only | `glob`, `grep`, `read`, `write`, `edit`, `bash` | Explorer tool narrowness is intentional, but current surface may be too weak without stronger evidence semantics. |
| Failure handling | Canonical fail-fast, repeated-failure detection, compaction, continuation guidance | Local loop corrections + eventual normalization | Same + builder-specific cleanup | Explorer has partial custom safeguards, but not parity with main-agent failure semantics. |
| Result contract | Streamed assistant/tool events plus canonical tool history shaping | Child success still normalized from final output plus retained evidence | Same family | Child result still depends too much on completion artifact shape. Harmful. |
| Parent consumption | Parent reasons over canonical conversation + summarized tool results | Explorer aggregated into one tool result, then parent continues | Build similar | Harmful: parent still consumes delegation as a tool result, not a first-class delegated run. |
| Progress transport | Canonical loop events | Typed delegated progress via progress channel | Same | Strong point. This is one of Krusty's better seams. |
| Permissions/governance | Canonical `ToolContext` policy, work mode, user/session metadata | Delegation policy inherited | Delegation policy inherited | Strong point. Governance is better than the rest of the delegated runtime. |
| Model/provider behavior | Main agent uses full call path and context-injected conversation | Child makes direct provider calls from custom loop | Same | Explorer is more provider-sensitive because child runtime is thinner and more prompt-dependent. |
| Resumability | Full recovery model | No child resumability beyond parent rerun | No child resumability beyond parent rerun | Harmful and directly different from OpenCode. |
| Output truth shaping | History policy and fail-fast consume tool summaries | Child produces artifact, then parent may summarize further | Same | Harmful seam: multiple stages can still distort delegated truth. |

## Detailed Findings

### 1. Context Parity Gap

The main agent gets layered context from [context.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/context.rs):
- workspace mode
- project instructions
- plan context
- skill inventory

Explorer children get only the custom system prompt from [types.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/types.rs).

This means child behavior depends much more on one prompt staying perfect.

Assessment:
- accidental harmful difference

### 2. Session-Grade Runtime Gap

The main agent in [orchestrator.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/orchestrator.rs) owns:
- recovery state
- continuation state
- runtime traces
- durable session state

Explorer children in [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs) do not have first-class delegated session identity or persistence.

Assessment:
- harmful architectural gap
- directly where OpenCode is stronger

### 3. Parent/Child Contract Weakness

Explorer child output is normalized and aggregated in [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs) and [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs), then shaped again by [history_policy.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/history_policy.rs), and interpreted by [failure.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/failure.rs).

This multi-stage pipeline has already produced regressions where successful exploration was later misclassified.

Assessment:
- harmful accidental complexity

### 4. Directory Evidence Undervalued

Explorer has improved target binding in [explore.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/explore.rs), but child result usefulness still skews too heavily toward file-backed final summaries.

Observed live failure:
- preflight showed real directory contents
- child still concluded the directory looked empty

Assessment:
- harmful evidence-model gap

### 5. ToolContext Parity Gap

Main agent tool execution in [executor.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/executor.rs) sets:
- session id
- db path
- project dir
- workspace mode
- AI client
- process registry
- output stream
- sandbox root

Subagent tool context in [execution.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/execution.rs) sets:
- working dir
- timeout
- delegated policy
- sandbox root for explore

It omits the richer session-aware runtime metadata.

Assessment:
- harmful capability/context gap

### 6. Explorer Tool Surface Is Intentionally Narrow, But Under-Supported

Explorer uses only `list`, `glob`, `grep`, `read` from [tools.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/subagent/tools.rs).

This is defensible, but only if:
- directory evidence counts strongly
- child evidence artifacts are strong
- parent aggregation is evidence-first

Right now those supporting conditions are not strong enough.

Assessment:
- intentional difference
- currently under-supported by surrounding runtime

### 7. Progress Transport Is Already Strong

Delegated progress forwarding through [executor.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/executor.rs) and server/PWA surfaces is one of Krusty's stronger designs.

Assessment:
- intentional good difference
- keep this seam

### 8. Governance Layer Is Stronger Than Delegated Runtime

Delegation policy in [registry.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/registry.rs) is cleaner than the rest of the delegated architecture.

Assessment:
- keep this
- use it as a stable base for redesign

## Intentional Differences To Keep

- Explorer stays read-only by default
- Builder stays distinct from explorer
- Delegated governance remains inherited from parent
- Typed delegated progress remains a shared contract
- `krusty-core` remains the only behavior brain

## Accidental Differences To Remove

- explorer child lacking session-grade runtime identity
- explorer child lacking layered context parity
- parent consuming delegation as “just another tool result”
- multiple result-shaping seams that can distort delegated truth
- delegated persistence/recovery being much thinner than main-agent recovery

## Phase 1 Conclusion

The core redesign target is now explicit:

Krusty must move from:
- thin delegated helper loops

to:
- first-class delegated runtime units with stronger evidence artifacts and parent aggregation

That is the architectural move required to become OpenCode-like from the subagent angle.
