# Krusty Mako Hybrid Design

## Purpose
Define the final product direction for Mako using the code we already have in Krusty, plus targeted borrowing from local OpenClaw and Claude Code implementations.

This document is not a generic competitor summary. It is a design decision for Krusty:
- what Mako should be
- what it should not be
- what Krusty already has
- what we should borrow
- what we should deliberately leave out

## Product Decision
Krusty should remain one product with three operating modes:
- `Chat` for conversational and research-first work
- `Code` for direct agentic coding
- `Mako` for autonomous project execution

Mako should not become a separate product and should not become a multi-channel gateway in the OpenClaw sense.

Mako should become a distinct workspace inside the existing Krusty client surfaces:
- Expo mobile app
- desktop shell built on the Expo web client
- server-backed control APIs
- with the TUI remaining the more direct Code-first surface rather than the primary Mako control plane

That means the control plane lives in the same shell the user already has, but Mako must stop feeling like "Code with a different tab."

## Design Goal
Build a hybrid system with this shape:
- OpenClaw-style local control plane discipline
- Claude Code-style background task orchestration, scheduling, and memory patterns
- Krusty-native runtime, storage, tool governance, and client surfaces

The target experience is:
- one Krusty app shell
- one persistent local runtime
- one shared project knowledge layer
- one dedicated Mako workspace for `Current`, `Runs`, `Reports`, and `Status`

## Non-Goals
- Rebuilding OpenClaw's broad channel/gateway identity inside Krusty
- Replacing Krusty's coding runtime with Claude Code semantics
- Shipping a separate Mako app before the in-repo clients are differentiated
- Copying consumer messaging metaphors that do not improve project execution

## Current Baseline
The current Krusty codebase already contains important Mako foundations.

### Runtime and server surface already exist
- Mako dispatch and session control routes already exist in [mako.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/mako.rs)
- persisted Mako runtime restart logic already exists in [mako_runtime.rs](/home/burgess/Work/krusty/crates/krusty-server/src/mako_runtime.rs)
- Mako runs already use the coordinator prompt, autonomous permission mode, and full tool registration in [mako_runtime.rs](/home/burgess/Work/krusty/crates/krusty-server/src/mako_runtime.rs)

### Reports and memory are not greenfield
- report persistence already exists in [reports.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/reports.rs)
- report tools already exist in [report.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/report.rs)
- report APIs already exist in [reports.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/reports.rs)
- persistent cross-session memory storage already exists in [memories.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/memories.rs)

### The current UX does not express a distinct Mako mode
- mobile currently renders Chat, Code, and Mako through the same main shell in [index.tsx](/home/burgess/Work/krusty/apps/mobile/app/(tabs)/index.tsx)
- the session list treats Mako as the same structural class as Code in [SessionList.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/SessionList.tsx)
- accordion controls distinguish `chat` versus non-chat, not `code` versus `mako`, in [AccordionControls.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/AccordionControls.tsx)
- reports are viewable in the shared shell via [ReportsViewer.tsx](/home/burgess/Work/krusty/apps/mobile/components/ReportsViewer.tsx)

### The biggest missing runtime property is persistence of execution
The `TickEngine` is not yet a true recurring daemon loop. It currently executes one orchestrator pass, forwards events, and can translate sleep into a sleeping finish state, but it does not continuously re-enter on tick in the always-on sense described by the roadmap. See [tick_engine.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/tick_engine.rs).

## External Systems We Can Learn From

### OpenClaw
OpenClaw is strongest as a local-first control plane.

What stands out in the local codebase:
- daemon/service management across operating systems in [service.ts](/home/burgess/Work/openclaw/src/daemon/service.ts)
- first-class scheduling and cron orchestration in [service.ts](/home/burgess/Work/openclaw/src/cron/service.ts)
- isolated scheduled task execution and delivery policy in [run.ts](/home/burgess/Work/openclaw/src/cron/isolated-agent/run.ts)
- explicit control-plane navigation in [navigation.ts](/home/burgess/Work/openclaw/ui/src/ui/navigation.ts)
- separate operational state surfaces in [app.ts](/home/burgess/Work/openclaw/ui/src/ui/app.ts)

OpenClaw's core value is not "better coding." It is operational control, local ownership, and wide-surface runtime management.

### Claude Code
Claude Code is strongest as a coding-adjacent orchestration runtime.

What stands out in the local codebase:
- durable and session-scoped scheduling in [CronCreateTool.ts](/home/burgess/Work/claude-code/src/tools/ScheduleCronTool/CronCreateTool.ts)
- background scheduled work injection in [useScheduledTasks.ts](/home/burgess/Work/claude-code/src/hooks/useScheduledTasks.ts)
- background task orchestration and visibility in [BackgroundTasksDialog.tsx](/home/burgess/Work/claude-code/src/components/tasks/BackgroundTasksDialog.tsx)
- remote session and approval mediation in [RemoteSessionManager.ts](/home/burgess/Work/claude-code/src/remote/RemoteSessionManager.ts)
- session memory extraction in [sessionMemory.ts](/home/burgess/Work/claude-code/src/services/SessionMemory/sessionMemory.ts)

Claude Code's core value is not "full control plane." It is long-running coding work, scheduling, remote execution, permission mediation, and memory around active tasks.

## Comparison Matrix
| Layer | Krusty Today | OpenClaw Strength | Claude Code Strength | Mako Direction |
| --- | --- | --- | --- | --- |
| Runtime loop | Has Mako session/runtime plumbing but no true recurring daemon loop yet | Persistent local service model | Strong background task execution model | Make Mako a real persistent local runtime |
| Control plane | Partial server APIs and shared client shell | Best-in-class operational control surfaces | Moderate task visibility | Build a Krusty-native control plane inside existing clients |
| Scheduling | Sleep exists, but not a first-class schedule product | Cron is first-class | Scheduling is integrated into coding workflow | Add first-class schedule and dispatch queue |
| Task orchestration | Has agent tools, reports, session storage | More ops-oriented than coding-oriented | Strong teammate/background/remote task patterns | Borrow Claude-style orchestration patterns |
| Reports | Already implemented and surfaced | Secondary concern | Secondary concern | Keep reports as shared project knowledge |
| Memory | Storage exists but is not yet strongly productized | Not the main differentiator | Strong session memory automation | Build a Mako memory layer on top of Krusty storage |
| UX identity | Mako currently feels too similar to Code | Distinct control workspace | Distinct task/status surfaces | Give Mako a dedicated workspace inside the same app shell |
| Mobile/desktop continuity | Already has Expo mobile and desktop wrapper | Strong operator dashboard model | Strong handoff and remote session concepts | Keep one Krusty app shell across mobile and desktop |
| Gateway / channels | Minimal today | Broad and central to product | Not the focus | Defer unless clearly needed later |

## Final Product Shape
Mako should become Krusty's autonomous project workspace.

It should feel like:
- a living home surface for the always-on assistant
- a live run observer
- a task and schedule board
- a reports and memory workspace
- an approvals and status surface

It should not feel like:
- a generic chat tab
- a clone of OpenClaw's channel-first gateway
- a deep-link wrapper around another app

## UX Decision
The user-facing shell should remain unified.

That means:
- the Expo app remains the main client foundation
- the desktop shell continues to wrap the same web surface
- Mako gets a distinct workspace inside that shell rather than a separate product
- the TUI remains important for direct coding, but not as the main home for control-plane style Mako management

The control chat can live inside that same UX, but it should be treated as one part of the Mako workspace rather than the whole thing.

## Final Navigation
The exact user-facing Mako navigation should be:

### Top-level Mako navigation
- `Current`
- `Runs`
- `Reports`
- `Status`

`Current` is the always-on home for Mako. It should contain:
- `Set course` as the primary action for giving Mako new work
- active run summaries
- waiting-on-you items
- a compact chat surface for steering and clarification

`Runs` is the operational list and should hold:
- active runs
- queued runs
- sleeping runs
- completed runs
- scheduled work when it is better treated as future run state than general system health

`Reports` is the knowledge surface and should hold:
- reports
- persistent memory
- promoted findings and project context snapshots

`Status` is the operator surface and should hold:
- daemon health
- approvals
- schedule health and upcoming wake events
- diagnostics

This keeps the top-level navigation compact enough for mobile while still making schedule and approvals first-class through `Current` and `Status`.

### Run-level navigation
Each individual run should use:
- `Overview`
- `Wake`
- `Tasks`
- `Chat`
- `Artifacts`

`Wake` is the canonical timeline for what just happened in a run.

`Artifacts` should remain plain language and can include:
- touched files
- diffs
- reports
- test results
- other run outputs

The current shared message-thread shell is still useful, but in Mako it should become a subordinate surface:
- command and clarification channel
- activity feed
- intervention point

It should no longer be the only expression of the mode.

## Identity Layer
OpenClaw's `SOUL.md` points to a real missing aspect in Krusty.

Krusty already has:
- project rules through `AGENTS.md` and similar workspace files
- persistent memory through the memory store
- a low-level prompt append hook through `.krusty/settings.json`

What Krusty does not have is a dedicated Mako identity layer.

Mako should get a first-class `MAKO.md` file at the project root.

`MAKO.md` should be:
- loaded only for Mako sessions
- about stance, proactivity, interruption style, autonomy level, and how Mako should relate to the user
- concise and versionable

`MAKO.md` should not hold:
- repository operating rules that belong in `AGENTS.md`
- long-term factual memory
- task history or changelog content

Role split:
- `AGENTS.md` defines repository rules and engineering constraints
- `MAKO.md` defines the Mako-specific working relationship and voice
- persistent memory stores durable facts, preferences, and project knowledge
- reports store research and analysis artifacts

`.krusty/settings.json.system_prompt_append` should remain a low-level escape hatch, not the main identity mechanism.

## Background Consolidation
OpenClaw's dreaming system is not just decorative plugin flavor.

In the local OpenClaw codebase it is part of the active memory subsystem, with scheduling, diary output, and promotion into durable memory.

Krusty does not currently have an equivalent background consolidation loop for Mako.

Mako should gain that capability later, but it should not copy the `Dreaming` name.

For Mako, the right concept is:
- background consolidation of completed runs, reports, tasks, and decisions into durable project knowledge
- optional human-readable diary or summary output
- automatic promotion of important facts into persistent memory

Working term:
- `Deep current`

`Deep current` should be a later-phase capability, not a phase 1 requirement.

The important decision is to borrow the behavior, not the label.

## Anthropic-Adjacent Cross-Reference
The local Claude Code checkout is useful here, but the terms are not all equally real in code.

### `buddy`
`buddy` does not appear to be a first-class product/runtime concept in the local Claude Code checkout.

Design implication:
- do not anchor Mako planning on `buddy` as if it were a stable product primitive
- treat the "always-on buddy" framing as our product language, not Anthropic source truth

### `kairos`
`kairos` does appear locally, but as an assistant-mode gate and behavior bundle rather than a standalone control-plane surface.

In the local Claude Code codebase it is tied to:
- assistant-mode activation and custom prompt addenda
- scheduled task runtime gating
- brief/status-oriented user messaging
- pre-seeded assistant team behavior
- keeping the main agent responsive by backgrounding long-running work

This is the valuable lesson from `kairos`:
- assistant mode is not just a prompt
- it changes scheduling, responsiveness, visibility, and delegation behavior together

For Mako, that means:
- `Mako` is the mode
- `Current` is the home surface
- the runtime, prompt, queueing, and visibility model all need to change together

### `Cowork`
`Cowork` references in the local Claude Code tree point to the host/container environment and surrounding shell assumptions more than the core assistant identity.

Local evidence suggests Cowork contributes:
- environment-specific memory path routing
- plugin/layout distinctions
- computer-use/runtime hosting assumptions
- desktop/daemon bridge semantics

This is useful for Mako as a product lesson:
- separate the always-on runtime from the shell that hosts it
- keep the control-plane UX distinct from the execution substrate

### `dispatch`
`dispatch` does not appear to be a strong Anthropic-branded surface concept in the local Claude Code checkout.

Most local hits are internal queue/guard state such as `dispatching`, not a named user-facing workspace.

Design implication:
- keeping `dispatch` internal for Krusty's APIs is fine
- copying it as a user-facing noun would add little value and blur Mako's identity

## Restructure Map
This is the concrete answer to "what parts of Mako are being restructured to fit what?"

### 1. Shared Mako chat shell -> `Current` workspace
Current source areas:
- [index.tsx](/home/burgess/Work/krusty/apps/mobile/app/(tabs)/index.tsx)
- [SessionList.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/SessionList.tsx)
- [AccordionControls.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/AccordionControls.tsx)
- [ChatBar.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/ChatBar.tsx)

Restructure target:
- the Mako tab stops being a generic transcript shell
- top-level Mako becomes `Current`, with `Set course`, active runs, waiting-on-you, and compact chat
- `Runs`, `Reports`, and `Status` become sibling surfaces rather than drawers hidden inside one shared chat layout

### 2. Mako session detail -> run workspace with `Wake`
Current source areas:
- [index.tsx](/home/burgess/Work/krusty/apps/mobile/app/(tabs)/index.tsx)
- [useWidgetSync.ts](/home/burgess/Work/krusty/apps/mobile/hooks/useWidgetSync.ts)
- [types.rs](/home/burgess/Work/krusty/crates/krusty-server/src/types.rs)

Restructure target:
- a selected Mako session becomes a run workspace
- run-level navigation becomes `Overview`, `Wake`, `Tasks`, `Chat`, `Artifacts`
- event streams that are currently just generic SSE/chat updates become the basis of `Wake`

### 3. Internal dispatch transport -> user-facing `Set course`
Current source areas:
- [mako.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/mako.rs)
- [client.ts](/home/burgess/Work/krusty/packages/api/src/client.ts)

Restructure target:
- preserve `/api/mako/dispatch` and related internals as transport
- expose `Set course` in the UI instead of `Dispatch`
- add queue/schedule summaries on `Current` and `Status` rather than making `dispatch` the branded concept

### 4. Tick runtime -> true always-on loop
Current source areas:
- [tick_engine.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/tick_engine.rs)
- [mako_runtime.rs](/home/burgess/Work/krusty/crates/krusty-server/src/mako_runtime.rs)
- [loop_events.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/loop_events.rs)

Restructure target:
- recurring re-entry after completion
- durable sleeping/paused/blocked states
- state and event semantics that can drive `Current`, `Wake`, and `Status` cleanly

### 5. Generic coordinator prompt -> layered Mako identity
Current source areas:
- [coordinator_prompt.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/coordinator_prompt.rs)
- [context.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/context.rs)
- [project_settings.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/project_settings.rs)

Restructure target:
- keep the coordinator prompt for execution strategy
- add `MAKO.md` for working relationship, proactivity, interruption style, and Mako-specific voice
- stop using low-level prompt append text as the main way to define Mako identity

### 6. Reports and memory primitives -> `Reports` knowledge surface
Current source areas:
- [reports.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/reports.rs)
- [report.rs](/home/burgess/Work/krusty/crates/krusty-core/src/tools/implementations/report.rs)
- [memories.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/memories.rs)
- [ReportsViewer.tsx](/home/burgess/Work/krusty/apps/mobile/components/ReportsViewer.tsx)

Restructure target:
- keep the underlying stores
- move them under a first-class `Reports` surface in Mako
- add promotion and retrieval policy so reports and memory behave like one project knowledge system

### 7. Passive persistence -> `Deep current`
Current source areas:
- [memories.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/memories.rs)
- [reports.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/reports.rs)
- [context.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/context.rs)

Restructure target:
- background consolidation of runs, reports, tasks, and decisions
- promotion of durable facts into memory
- optional diary/summary output for review

### 8. Raw runtime/admin state -> `Status`
Current source areas:
- [server.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/server.rs)
- [mako.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/mako.rs)
- [types.rs](/home/burgess/Work/krusty/crates/krusty-server/src/types.rs)

Restructure target:
- daemon health, approvals, wake schedule, diagnostics, and operator controls become a dedicated `Status` surface
- not mixed into the main transcript

## What Stays, What Changes

### Keep mostly as-is
- report storage
- memory storage
- Mako route namespace
- internal dispatch APIs
- existing autonomous task primitives

### Extend substantially
- tick runtime
- server event/state transport
- context injection
- client-side Mako navigation
- approval and schedule visibility

### Replace at the UX layer
- Mako-as-shared-chat-tab
- `Dispatch` as user-facing vocabulary
- transcript-only run inspection

## Architecture Decisions

### 1. Keep one shell, split the workspace
Use the current mobile/web/desktop client family as the host shell.

Do not build a separate Mako app.

Instead:
- preserve shared auth, session transport, notifications, and server connectivity
- add a distinct Mako information architecture and component set
- let Chat and Code remain simpler conversation-first experiences

### 2. Make the runtime truly daemon-grade
Mako cannot be credible until it actually re-enters on tick.

Required outcome:
- the runtime continues after a completed pass
- scheduled wakeups are first-class
- sleep is an actual idle state, not a terminal-feeling stop
- session observation remains continuous across wake, work, and pause

This is the first hard dependency.

### 3. Use Krusty's existing reports and memory stores as the knowledge substrate
Reports and memories should be shared project assets across modes.

That means:
- Chat can create research artifacts
- Code can consult or update them
- Mako can generate, consume, and organize them continuously

The missing work is:
- better retrieval and injection policy
- Mako-facing UX
- automation for summarization and ongoing project state capture
- background consolidation passes (`Deep current`)

### 4. Add a dedicated Mako identity layer
Mako should not rely only on the generic coordinator prompt plus ad hoc prompt append text.

We should add:
- `MAKO.md` as a Mako-only workspace file
- prompt injection rules that apply it only to Mako sessions
- a clear separation between rule files, identity files, and memory files

This captures the useful part of OpenClaw's `SOUL.md` pattern without copying its naming.

### 5. Borrow control-plane patterns from OpenClaw, not product identity
Borrow:
- daemon lifecycle discipline
- status dashboards
- scheduling visibility
- approvals and operational observability
- clearer separation between active work and system administration

Do not borrow:
- multi-channel inbox as the center of the product
- broad gateway identity
- integration breadth before Krusty's core project runtime is solid

### 6. Borrow orchestration patterns from Claude Code, not handoff UX
Borrow:
- scheduled work model
- background task visibility
- remote session thinking
- approval mediation
- automatic memory extraction from long-running work

Do not borrow:
- "handoff to another surface" as the primary control model
- chat-only framing for long-running autonomous work

## What Mako Is Missing

### Runtime gaps
- true recurring tick execution
- explicit scheduler semantics
- durable queueing and wake policy
- stronger runtime state transitions for sleeping, paused, blocked, and waiting states

### Product gaps
- dedicated Mako information architecture
- final user-facing navigation and hierarchy
- first-class `Set course` surface
- schedule management UX
- approvals center
- project status surface
- live run visibility that is not just a message stream

### Knowledge gaps
- dedicated `MAKO.md` identity layer
- automatic project memory shaping
- retrieval policy for memories and reports
- stronger relationship between tasks, reports, and project state
- background consolidation (`Deep current`)

### Operational gaps
- daemon lifecycle tooling
- health diagnostics
- visibility into stuck or sleeping sessions
- explicit operator controls

## Implementation Path

### Phase 1: True daemon runtime
Goal:
- make `TickEngine` a real re-entry loop
- define durable wake and sleep transitions
- keep session observation continuous

Success criteria:
- Mako sessions survive beyond one orchestrator pass
- sleep behaves as an idle state
- tick cadence is observable and controllable

### Phase 2: Mako workspace split
Goal:
- replace the current "same chat shell" feeling with a distinct Mako workspace in the existing app shell

Scope:
- top-level `Current`, `Runs`, `Reports`, and `Status`
- run list and run detail views
- `Set course` separated from the chat transcript
- run-level `Overview`, `Wake`, `Tasks`, `Chat`, and `Artifacts`
- task, report, and status panels

Success criteria:
- Mako is visually and structurally distinct from Code
- the user can manage autonomous work without living inside one thread transcript

### Phase 3: Set course and scheduling
Goal:
- make Mako a reliable fire-and-forget system

Scope:
- internal dispatch queue behind the user-facing `Set course` action
- scheduled tasks
- wake policy
- milestone notifications
- run priority and queue state

Success criteria:
- the user can submit, defer, inspect, and resume work predictably

### Phase 4: Memory and project knowledge
Goal:
- turn current report and memory primitives into a coherent project knowledge layer

Scope:
- `MAKO.md` as a first-class identity file
- memory extraction jobs
- report-to-memory promotion rules
- project snapshot summaries
- retrieval policy for future Mako runs
- `Deep current` background consolidation passes

Success criteria:
- Mako feels stateful across days, not just within one run

### Phase 5: Control-plane hardening
Goal:
- add the operational clarity borrowed from OpenClaw

Scope:
- daemon status and lifecycle controls
- health and diagnostics
- approval center
- runtime metrics and stuck-state visibility

Success criteria:
- autonomous behavior is observable, interruptible, and trustworthy

## Deferred Decisions
These should stay explicitly deferred until the earlier phases are working.

- remote gateway or node network features
- broad device or browser control surfaces
- channel-first communication fabric
- OpenClaw-style ecosystem breadth
- large plugin marketplace work

Deferring them is intentional. Mako first needs to be a strong autonomous project runtime inside Krusty.

## Final Direction
The hybrid answer is:
- Krusty for the core runtime, storage, and client shell
- OpenClaw for control-plane discipline
- OpenClaw for the `SOUL.md` lesson and memory consolidation lesson, but not for direct naming copy
- Claude Code for orchestration, scheduling, approvals, and memory patterns

The product we should build is not "OpenClaw inside Krusty" and not "Claude Code with a different skin."

It is:
- one Krusty shell
- one persistent local runtime
- one dedicated Mako workspace
- one shared project knowledge layer
- one operationally visible autonomous system

## Next Build Order
1. Fix the runtime so Mako is actually always-on.
2. Split Mako into a real workspace inside the Expo and desktop shell.
3. Add `Set course`, internal dispatch queue, and schedule as first-class concepts.
4. Add `MAKO.md` plus coherent project knowledge and `Deep current` consolidation.
5. Harden the control plane with approvals, diagnostics, and runtime visibility.
