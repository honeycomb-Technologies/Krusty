# Krusty Subagent Redesign Comparison Plan

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose

Define the finalized redesign needed to make Krusty's delegated agents, especially `explore`, function like a first-class OpenCode-style system while preserving Krusty's modularity and smaller architectural footprint.

This plan is grounded in:
- direct inspection of Krusty's main-agent and subagent paths
- direct inspection of OpenCode's `task` subagent model
- direct inspection of pi's minimal-core philosophy
- repeated live failures in Krusty's delegated exploration flow

## Executive Conclusion

Krusty's delegated system is currently underperforming because it is built as a thin custom mini-loop that tries to do first-class delegated work without having first-class delegated runtime structure.

OpenCode's key advantage is not just a better prompt. It gives subagents:
- their own child session
- their own explicit agent identity
- their own permission envelope
- resumability
- a strong result boundary

Pi's key advantage is architectural honesty:
- it does not make built-in delegation a core promise unless the base runtime can support it

Krusty is currently in the unstable middle:
- more ambitious than pi
- less first-class than OpenCode

The redesign direction is therefore:
- move closer to OpenCode's first-class delegated session model
- do not copy OpenCode wholesale
- keep Krusty's core as the single behavior brain
- make delegated exploration evidence-driven and session-backed instead of prompt-fragile

## Comparison Baseline

### Krusty Today

Relevant Krusty files:
- `crates/krusty-core/src/tools/implementations/explore.rs`
- `crates/krusty-core/src/agent/subagent/execution.rs`
- `crates/krusty-core/src/agent/subagent/types.rs`
- `crates/krusty-core/src/agent/subagent/tools.rs`
- `crates/krusty-core/src/agent/orchestrator.rs`
- `crates/krusty-core/src/agent/history_policy.rs`
- `crates/krusty-core/src/agent/failure.rs`

Current delegated shape:
- parent agent calls `explore`
- `explore` spawns `SubAgentTask`s
- each child runs a lightweight custom loop
- child outputs are normalized and aggregated into one parent tool result

Current strengths:
- typed delegated progress exists
- target binding is improved
- delegated failure classification is improved
- child output is more truthful than before

Current weaknesses:
- child runtime is thinner than main runtime
- parent still depends too much on child polish
- parent and child do not share one strong evidence contract
- directory-structure evidence is underweighted
- parent fallback/manual continuation is still too eager

### OpenCode

Relevant OpenCode files:
- `/home/burgess/Work/opencode/packages/opencode/src/tool/task.ts`
- `/home/burgess/Work/opencode/packages/opencode/src/agent/prompt/explore.txt`

Important properties:
- subagents are invoked through a dedicated task tool
- each task gets a child session
- child session has parent linkage and resumability
- permission envelope is explicit
- subagent identity is explicit via `subagent_type`
- result boundary is explicit via `<task_result>`

Architectural lesson:
- OpenCode treats subagents as first-class runtime units, not helper loops hiding inside one tool call

### pi

Relevant pi file:
- `/home/burgess/Work/pi-mono/packages/coding-agent/README.md`

Important property:
- pi explicitly avoids built-in subagents in core

Architectural lesson:
- if delegation is not first-class, the professional answer is to not overpromise it

## Core Findings

### Finding 1: Runtime parity gap

Krusty main-agent runtime and Krusty subagent runtime are too different.

The main agent has:
- richer context layering
- stronger orchestration semantics
- more mature failure/recovery behavior
- more stable continuation semantics

Explorer subagents still have:
- a custom lightweight loop
- thinner context
- narrower runtime contract
- higher dependence on prompt obedience

This is the largest structural issue.

### Finding 2: Delegated evidence contract is incomplete

Krusty still asks child agents to provide a good final artifact, then asks the parent to reason from that artifact.

That is backwards.

The stable model is:
- child collects structured evidence
- parent aggregates that evidence
- parent produces the final investigation

### Finding 3: Directory exploration is still too fragile

Repeated real failure:
- preflight seed shows a target has content
- live child concludes the target is empty or inaccessible
- parent gets a partial result and still tries to continue

This means:
- directory tree evidence is too weakly represented
- tool-truth correction is still too narrow

### Finding 4: Parent truthfulness and delegation trust are not aligned

The parent can still:
- overclaim from partial delegated evidence
- mistrust successful delegated evidence
- fall back into manual probing too eagerly

That means delegation is still not a first-class reasoning substrate for the parent.

### Finding 5: Current architecture is too patch-sensitive

Recent work improved:
- truthfulness
- failure classification
- target binding
- evidence retention

But explorer still feels brittle because the architecture is still fundamentally session-light and child-summary-dependent.

## Redesign Principles

1. `krusty-core` remains the only behavior brain.
2. Delegated work becomes first-class runtime state, not ad hoc helper flow.
3. Parent should reason from child evidence, not just child prose.
4. Child success should be evidence-first, prose-second.
5. Surfaces should render delegated truth, not infer it.
6. Do not solve this with arbitrary caps and hope.
7. Do not hide failure by loosening success criteria.

## Target Architecture

### 1. First-Class Delegated Sessions

Each delegated exploration task should become a first-class runtime unit with:
- delegated run id
- parent session linkage
- delegated session metadata
- resumable delegated state
- explicit model/provider inheritance
- explicit delegated permission envelope

This does not need to expose a fully user-facing session tree immediately, but the runtime shape should be session-grade.

### 2. Evidence-First Child Result Schema

Explorer child runs should produce canonical structured artifacts containing:
- target metadata
- files examined
- directories examined
- matched patterns
- key findings
- concerns
- confidence
- stop reason
- failure reason if incomplete

Child prose should become presentation, not the primary truth source.

### 3. Parent Aggregation From Child Artifacts

The parent `explore` tool should aggregate child artifacts into a parent investigation containing:
- coverage map
- successful targets
- failed targets
- evidence references
- overall confidence
- overall outcome

The parent should not need to guess whether delegation “felt complete.”

### 4. Stronger Target-Scoped Runtime

Each child should have:
- actual target cwd
- actual target sandbox root
- explicit target metadata in state
- explicit path-resolution semantics

Directory structure should be treated as real evidence, not a weak precursor to “real” evidence.

### 5. Clear Stop Conditions

Delegated exploration should stop when:
- enough evidence exists for a defensible answer
- coverage reaches target sufficiency
- provider quality degrades into misread or non-progress
- orphan/disconnect policy requires forced synthesis

Stop reasons must be explicit and preserved.

## Finalized Roadmap

### Phase 1: Runtime Gap Matrix

Goal:
- inventory the exact differences between main agent, explorer child, and build child runtimes

Deliverables:
- file-by-file discrepancy matrix
- list of intentional vs accidental differences
- list of runtime features explorer must inherit or emulate

Exit gate:
- no ambiguity remains about where explorer is weaker than the main agent

### Phase 2: First-Class Delegated Run Model

Goal:
- replace the current “thin child loop as an implementation detail” model with a first-class delegated run contract

Deliverables:
- delegated run identity model
- parent linkage model
- delegated state persistence contract
- delegated permission/model inheritance contract

Exit gate:
- explorer children are first-class runtime units, not anonymous helper loops

### Phase 3: Evidence Contract Rewrite

Goal:
- define canonical child evidence artifacts and parent aggregation schema

Deliverables:
- typed child evidence schema
- typed parent aggregate schema
- outcome/failure taxonomy
- directory evidence semantics

Exit gate:
- parent and child share one strong delegated evidence contract

### Phase 4: Explorer Child Runtime Upgrade

Goal:
- make explorer child execution operate on the new delegated run model

Deliverables:
- stronger child run state
- stronger target scoping
- stronger evidence accumulation
- stronger tool-truth correction
- reduced reliance on perfect child prose

Exit gate:
- child runs reliably produce usable evidence artifacts

### Phase 5: Parent Aggregation Rewrite

Goal:
- make `explore` reason from child artifacts instead of child polish

Deliverables:
- parent evidence aggregation
- parent confidence/coverage synthesis
- explicit partial/degraded messaging
- reduced fallback to messy manual probing

Exit gate:
- parent no longer abandons delegation prematurely or overclaims from partial runs

### Phase 6: Provider Reliability Layer

Goal:
- make delegated behavior degrade gracefully across providers, especially MiniMax

Deliverables:
- provider-aware delegated concurrency/fanout policy
- provider-specific task narrowing where needed
- delegated-provider diagnostics

Exit gate:
- weak providers degrade into honest partial coverage, not incoherent delegation

### Phase 7: Surface Parity

Goal:
- make TUI, server, and PWA show the same delegated truth

Deliverables:
- identical delegated outcome semantics across surfaces
- delegated reason rendering
- delegated session/run visibility
- improved singular investigation presentation

Exit gate:
- user-facing delegated behavior is coherent everywhere

### Phase 8: Validation and Closure

Goal:
- prove the redesigned subagent system in real runs

Scenarios:
- successful multi-target codebase exploration
- partial exploration with explicit coverage gaps
- invalid target
- provider degradation
- reconnect/reload during delegated run
- repeated delegated runs in one parent turn

Deliverables:
- closure report
- residual risk list
- comparison summary versus OpenCode and pi after redesign

Exit gate:
- Krusty delegated exploration is functionally reliable and professionally legible

## What We Are Intentionally Copying From OpenCode

- first-class delegated runtime units
- explicit child identity
- explicit parent linkage
- stronger result boundary
- resumability

## What We Are Intentionally Not Copying

- full duplication of the main agent stack inside child runs
- oversized abstraction count
- product choices that do not fit Krusty's surfaces

## What We Are Intentionally Keeping From Krusty

- `krusty-core` as the only behavior brain
- typed delegated progress/events
- stronger modular boundaries
- local-first and server-first shared runtime

## Success Criteria

This redesign is complete when:
- explorer children behave like first-class delegated workers
- parent aggregation is evidence-driven
- partial runs are truthful and useful
- parent no longer drifts into manual chaos after mostly successful delegation
- Krusty's delegated exploration quality is much closer to OpenCode than to the fragile state we have now
