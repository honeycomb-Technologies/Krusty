# Krusty Full System Audit

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Purpose
This document is the master audit specification for Krusty. Its purpose is to drive a full-system coherence review across `krusty-core`, `krusty-server`, `krusty-cli`, and the PWA/mobile surfaces so every behavioral gap, architectural inconsistency, and operational weakness is captured in one place and tracked to closure.

This is not a roadmap for a single feature. It is the umbrella audit program for understanding the whole system end to end and producing the fix backlog required to make Krusty coherent, reliable, and professional across all layers.

## Audit Objectives
- Establish one canonical understanding of how Krusty works across all crates and apps.
- Identify every meaningful coherence gap between core, server, TUI, PWA, and mobile/desktop surfaces.
- Track product, architecture, performance, memory, transport, and UX defects in one place.
- Separate intentional design choices from accidental behavior drift.
- Produce a fix backlog with clear ownership, severity, and verification requirements.

## Audit Rules
- Audit the actual code on disk, not the intended design alone.
- Prefer primary source inspection over assumptions.
- Treat cross-surface drift as a first-class defect, not a presentation detail.
- Distinguish:
  - architecture issues
  - runtime correctness issues
  - product semantics issues
  - performance and memory issues
  - observability gaps
- Do not close a finding until there is explicit verification evidence.

## Repository Scope
Primary boundaries under audit:
- `crates/krusty-core`: canonical agent brain and runtime contracts
- `crates/krusty-server`: transport, control plane, remote access, session APIs
- `crates/krusty-cli`: terminal and TUI execution surface
- `apps/pwa/app`: installable web client and remote/mobile surface
- `apps/mobile`: mobile-specific surface work
- `apps/desktop`: desktop wrapper surface
- `docs/`: roadmap, closure, and audit truth documents

## Current Source Inventory
Source-scope file counts from the current repository scan, excluding generated output like `node_modules`, `dist`, `.svelte-kit`, `build`, and `target`:
- `crates/krusty-core`: 251 files
- `crates/krusty-cli`: 174 files
- `crates/krusty-server`: 35 files
- `apps/pwa`: 271 files
- `apps/mobile`: 24 files
- `apps/desktop`: 19 files
- total source/control files in `crates`, `apps`, and `docs`: 787 files

High-density `krusty-core` areas:
- `src/extensions`: 87 files
- `src/ai`: 36 files
- `src/tools`: 29 files
- `src/agent`: 28 files
- `src/storage`: 16 files

## Audit Tracks

### 1. Core Runtime Coherence
Scope:
- orchestration loop
- streaming and recovery
- context injection
- plan lifecycle
- delegated/subagent execution
- compaction and continuation

Questions:
- Does `krusty-core` have one canonical control path?
- Are tool, planning, recovery, and continuation semantics consistent across all entrypoints?
- Are delegated runs convergent, explainable, and bounded by evidence rather than drift?

### 2. AI Provider Layer
Scope:
- request transformation
- provider capabilities
- model metadata
- tool-call formatting
- usage accounting
- retries and failure surfaces

Questions:
- Are provider quirks handled centrally?
- Are tool-call IDs, streaming deltas, usage, and capability flags coherent across providers?
- Are there hidden provider-specific failure classes still leaking through?

### 3. Tools and Governance
Scope:
- tool registry
- approval rules
- retry rules
- sandbox and working directory semantics
- neutral vs project mode behavior
- delegated tools (`explore`, `build`)

Questions:
- Are tools governed by one policy model?
- Do tools behave consistently across neutral mode, project mode, and delegated runs?
- Are top-level tool outcomes truthful and useful?

### 4. Storage, Recovery, and Trace Truth
Scope:
- sessions
- agent state
- recovery state
- runtime traces
- schema migrations
- startup reconciliation

Questions:
- Does persisted state match live truth?
- Are stale transient states reconciled cleanly?
- Can traces explain what happened without log spelunking?

### 5. Server and Control Plane
Scope:
- local and remote auth
- Tailnet/private remote access
- session APIs
- chat/SSE transport
- direct tool routes
- presence and operator state

Questions:
- Is the server a clean transport layer over core?
- Is local vs remote behavior explicit and correct?
- Are long-running sessions resilient without wasting resources?

### 6. Surface Parity
Scope:
- TUI
- PWA
- mobile
- desktop wrapper
- ACP/editor pathways where relevant

Questions:
- Does each surface show the same semantic truth?
- Are live runs, delegated runs, plans, recovery, and approvals coherent across surfaces?
- Is the PWA/mobile path first-class rather than an approximation?

### 7. Performance and Memory
Scope:
- delegated exploration/build memory growth
- streaming backpressure
- trace persistence overhead
- large session growth
- cache behavior
- resource cleanup after disconnects and restarts

Questions:
- Is memory growth bounded?
- Are expensive paths observable?
- Do long sessions degrade gracefully instead of catastrophically?

### 8. Product Semantics and UX Truth
Scope:
- workspace mode
- project selection and creation
- delegated artifact presentation
- status badges
- error communication
- continuity and reconnect semantics

Questions:
- Does the product say what is actually happening?
- Are success, partial, degraded, and failed states represented truthfully?
- Do users understand session/project state at a glance?

## Known Findings Seeded Into This Audit
These are already discovered and must remain visible in the master audit until explicitly verified closed.

### Open Findings
1. Delegated `explore` can return an empty investigation while being marked `success`.
   - Symptom: top-level `explore` result reports success even when `files_examined_count = 0`.
   - Consequence: parent agent falls back into messy manual reads/tool thrash because it received no usable evidence.
   - Primary files:
     - `crates/krusty-core/src/tools/implementations/explore.rs`
     - `crates/krusty-core/src/agent/subagent/execution.rs`

2. Delegated exploration still has a successful-but-non-convergent path.
   - Symptom: subagents can keep issuing valid read/glob/grep actions long after architecture evidence should be sufficient.
   - Consequence: long-running investigations, wasted tokens, background drain after disconnect.
   - Primary files:
     - `crates/krusty-core/src/agent/subagent/execution.rs`
     - `crates/krusty-core/src/agent/subagent/types.rs`
     - `crates/krusty-core/src/tools/implementations/explore.rs`

3. Memory-risk audit is improved but not fully closed.
   - Symptom: a previous live server run reached roughly `22 GB` RSS during delegated exploration.
   - Confirmed fix landed: shared explore file cache now has byte caps.
   - Remaining concern: large serialized subagent histories and retry cloning may still amplify memory usage under pathological runs.
   - Primary files:
     - `crates/krusty-core/src/agent/cache.rs`
     - `crates/krusty-core/src/agent/subagent/execution.rs`
     - `crates/krusty-server/src/lib.rs`
     - `crates/krusty-server/src/routes/chat.rs`

4. Delegated outcome semantics still need final tightening.
   - Symptom: UI and parent-loop handling can still receive truthy completion states from weak or fallback summaries.
   - Consequence: mismatch between what the agent did and what the user sees.
   - Primary files:
     - `crates/krusty-core/src/tools/implementations/explore.rs`
     - `crates/krusty-core/src/agent/history_policy.rs`
     - `apps/pwa/app/src/lib/components/chat/DelegatedToolWidget.svelte`

### Recently Closed Findings
1. MiniMax tool-call ID sanitization collisions.
   - Fixed centrally in `crates/krusty-core/src/ai/transform.rs`.

2. `bash` tool panic: `JoinHandle polled after completion`.
   - Fixed in `crates/krusty-core/src/tools/implementations/bash.rs`.

3. SSE disconnect orphaned active runs.
   - Fixed in `crates/krusty-server/src/routes/chat.rs` by continuing background drain without active delivery.

4. Remote PWA auth bootstrap failed in standalone homescreen mode.
   - Fixed with durable remote auth cookie bootstrap in:
     - `crates/krusty-server/src/routes/remote_auth.rs`
     - `crates/krusty-server/src/auth.rs`
     - `apps/pwa/app/src/lib/api/client.ts`

5. Sessions accidentally inherited Krusty repo context when no workspace was selected.
   - Fixed with explicit neutral/project workspace semantics across:
     - `crates/krusty-core/src/storage/sessions.rs`
     - `crates/krusty-server/src/routes/chat.rs`
     - `crates/krusty-core/src/agent/context.rs`
     - PWA workspace/session stores

## Audit Output Requirements
Every substantive finding added under this program should include:
- title
- severity
- subsystem
- symptom
- root cause
- impact
- files involved
- current status
- verification required to close

## Completion Standard
This audit is complete only when:
- every major subsystem has been reviewed
- every material finding is tracked in the master tracker
- each finding is either closed with evidence or explicitly marked intentional/deferred
- the remaining risk surface is clear enough that future work is targeted instead of confused
