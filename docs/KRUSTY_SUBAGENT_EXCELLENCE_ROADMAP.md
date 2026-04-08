# Krusty Subagent Excellence Roadmap

## Purpose
This roadmap exists to make Krusty's delegated agents, especially `explore`, professional-grade rather than opportunistic mini-loops. The target is simple:
- delegated agents must be trustworthy sources of evidence
- they must stay scoped
- they must stop decisively
- they must report truthful results
- they must be legible across core, server, and PWA

This is not a generic “improve AI” plan. It is a focused redesign and hardening program for Krusty's built-in delegation model.

## Why This Roadmap Exists
Recent live runs exposed a structural weakness:
- the outer loop, server, and PWA are now mostly behaving correctly
- but delegated `explore` can still fail because the subagent execution model is too dependent on the model faithfully following prompt-only scope and correctly interpreting tool results

Compared to local reference systems:
- OpenCode uses dedicated subagent/task surfaces with narrow role definitions and explicit agent identity
- pi avoids owning built-in subagents in core at all

Krusty currently sits in the harder middle:
- built-in delegation exists
- but it is not yet specialized enough to be consistently reliable under weaker providers

## Design Goal
Move Krusty's delegated path closer to a specialized agent surface:
- explicit assignment
- explicit scope
- explicit target directory
- explicit evidence contract
- explicit failure classes
- explicit delegated UI artifact semantics

## Principles
- `krusty-core` remains the source of delegated truth
- prompt text alone is not a sufficient boundary
- delegated results must be classified by evidence quality, not optimism
- weaker providers must be contained by runtime contracts, not trusted to self-correct
- server/PWA should render delegated truth, not guess it

## Phases

### Phase 1: Scope Binding
Goal:
- bind delegated explorers to actual resolved target directories/files instead of only describing targets in prompt text

Work:
- resolve and validate requested explore targets before spawn
- set real per-task working directories
- fail invalid targets before launch

Exit:
- each explorer agent runs from its assigned target, not just “about” it

### Phase 2: Tool-Truth Grounding
Goal:
- stop delegated agents from claiming tools returned nothing when tool output was successful and non-empty

Work:
- strengthen explorer system prompt with evidence-precedence rules
- add runtime correction guard for misread tool output
- degrade/fail agents that continue ignoring valid tool results

Exit:
- successful tool output becomes the hard truth boundary for delegated reasoning

### Phase 3: Delegated Result Truthfulness
Goal:
- make delegated results explain what failed and why

Work:
- classify outcomes by:
  - usable evidence
  - no usable evidence
  - invalid target
  - misread tool output
  - provider failure
- surface reason codes in delegated evidence JSON and top-level tool output

Exit:
- failed or degraded delegated runs are diagnosable without log spelunking

### Phase 4: Specialized Explorer Contract
Goal:
- make `explore` a more specialized delegated mode, not just a generic read-only loop

Work:
- tighten prompt and progress language around architecture investigation
- encode stop/summarize expectations more explicitly
- add clearer target labels and investigation role identity

Exit:
- explorer agents behave like specialized investigators, not generic assistants with fewer tools

### Phase 5: Server and PWA Delegated Semantics
Goal:
- make delegated artifacts first-class and truthful across server/PWA

Work:
- carry delegated outcome reasons to UI state
- render failure classes and evidence classes clearly
- prevent ambiguous “red X but maybe mostly worked” states

Exit:
- delegated failures and degraded runs are understandable from the UI alone

### Phase 6: Provider-Aware Delegation Policy
Goal:
- keep delegated exploration reliable even when providers differ materially

Work:
- review provider-specific delegated behavior
- constrain or adapt delegated prompts/launch patterns where needed
- keep concurrency stability and reasoning stability separate

Exit:
- MiniMax-class providers no longer break delegation in opaque ways

### Phase 7: Traceability and Auditability
Goal:
- make delegated runs easy to debug from persisted state and traces

Work:
- include delegated reason signals in traces/session state where appropriate
- ensure active and completed runs preserve enough reason context for diagnosis

Exit:
- delegated issues can be understood from state/trace APIs, not just transient logs

### Phase 8: Final Competitive Audit
Goal:
- compare Krusty's delegated model against the strongest local reference patterns again after changes

Work:
- re-compare against local OpenCode and pi reference points
- document intentional deltas
- record closure and residual risks

Exit:
- delegated architecture is either at parity for Krusty's goals or intentionally different for defensible reasons

## Current Baseline Before Execution
Already completed before this roadmap:
- zero-evidence `explore` no longer silently masquerades as healthy at the top level
- fail-fast parent stop after degraded exploration
- actual target directory binding for explore tasks
- runtime misread-tool-output guard
- delegated outcome reason plumbing into final results and PWA rendering

These are now treated as the starting baseline, not the finished answer.
