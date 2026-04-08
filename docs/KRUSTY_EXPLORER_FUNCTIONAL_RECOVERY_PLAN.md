# Krusty Explorer Functional Recovery Plan

## Purpose
Restore `explore` to being a useful information-gathering tool again.

This plan exists because the recent hardening work improved truthfulness and containment, but exposed a deeper problem: Krusty's explorer subagents are still too different from the main agent runtime, and that gap is now the main reason `explore` is not functionally reliable.

## What The Audit Shows
- The main agent and explorer subagents are materially different runtimes.
- The main agent has richer context, richer tool access, and a more complete orchestration path.
- Explorer subagents currently run on a thinner custom loop with a narrower tool surface and much weaker runtime scaffolding.
- The recent fixes mostly improved failure honesty, not delegated capability.
- Live evidence now shows subagents often fail by ending without usable investigation artifacts rather than by crashing.

## Root Cause
`explore` is currently trying to do a high-complexity delegated audit with a low-capability delegated runtime.

This is the core mismatch:
- hard task: parallel architecture investigation
- thin runtime: prompt + mini-loop + limited tools

That mismatch is why the tool used to appear to "work" loosely, but now fails visibly once stricter quality checks are applied.

## Design Goal
Make explorer subagents function more like a real specialized task agent and less like a reduced copy of the main loop.

The target state:
- reliable scoped exploration
- concrete evidence gathering
- deterministic result assembly
- minimal provider-specific brittleness
- parent aggregation that uses evidence instead of rescuing weak child output

## Phases

### Phase 1: Runtime Gap Inventory
Goal:
- document every meaningful difference between the main agent loop and the explorer subagent loop

Must compare:
- context injection
- prompt layering
- tool surface
- streaming vs non-streaming behavior
- checkpointing and retry semantics
- result assembly
- history construction
- provider call path

Exit:
- one explicit diff table of "main agent has X / explorer lacks Y"

### Phase 2: Explorer Capability Recovery
Goal:
- restore the minimum runtime capabilities explorer needs to gather evidence reliably

Areas to evaluate:
- whether explorer needs `list`
- whether explorer needs a bounded read-only shell path for directory inspection
- whether file discovery should be pre-seeded by core instead of left entirely to the model
- whether explorer should receive stronger working-set context from the parent

Exit:
- explorer has the smallest viable tool/runtime surface required for dependable investigation

### Phase 3: Specialized Explorer Session Contract
Goal:
- stop treating explorer as a generic reduced loop

Work:
- add an explicit specialized explorer task contract
- define mandatory intermediate state:
  - target scope
  - evidence ledger
  - files touched
  - current investigation objective
- make final report assembly a runtime step, not only a model-formatting hope

Exit:
- explorer success depends on captured evidence state, not prose compliance alone

### Phase 4: Parent/Subagent Cooperation Redesign
Goal:
- make the parent orchestrator aggregate child evidence instead of depending on child polish

Work:
- parent should receive structured child evidence fragments
- parent should assemble the final exploration artifact from child evidence plus child summaries
- failed children should degrade the result without collapsing the whole investigation unnecessarily

Exit:
- explorer remains useful even if some child agents are only partially competent

### Phase 5: Provider-Specific Reliability Pass
Goal:
- make delegated exploration stable on weaker providers without depending on ideal reasoning behavior

Work:
- review MiniMax-specific failure patterns
- tune delegated concurrency, launch shape, and task size
- constrain explorer fanout when provider quality drops
- consider provider/model routing rules for delegated exploration specifically

Exit:
- delegated exploration has a provider-aware reliability policy instead of one universal behavior

### Phase 6: Server/PWA Artifact Recovery
Goal:
- keep web surfaces truthful while explorer regains functional strength

Work:
- ensure partial-but-useful exploration is rendered as such
- keep failure reasons explicit
- make parent-aggregated results singular and legible

Exit:
- explorer appears coherent in the UI even during degraded runs

### Phase 7: Competitive Re-Audit
Goal:
- re-compare the repaired explorer against local OpenCode and pi reference patterns

Exit:
- either parity is restored for Krusty's goals, or remaining deltas are explicit and intentional

## Success Criteria
Explorer is considered recovered when:
- it reliably returns evidence-backed results on real codebase audits
- it stops failing on routine scoped exploration
- it no longer depends on the parent to rescue empty child outputs
- it behaves legibly in server/PWA
- its limitations are provider-aware and explicit instead of random
