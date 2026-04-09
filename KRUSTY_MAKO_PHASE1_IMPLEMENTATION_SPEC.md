# Krusty Mako Phase 1 Implementation Spec

## Purpose
Define the first implementation pass that turns Mako from a shared chat-like tab into a real always-on workspace with:
- `Current`
- run-level `Wake`
- a true daemon-grade runtime loop
- a dedicated Mako identity layer via `MAKO.md`

This spec is intentionally concrete. It names the exact seams to change, what stays internal, and how to keep the code elegant, modular, and performant.

## Design Anchor
This spec implements the decisions already captured in:
- [KRUSTY_MAKO_HYBRID_DESIGN.md](/home/burgess/Work/krusty/KRUSTY_MAKO_HYBRID_DESIGN.md)
- [KRUSTY_MAKO_ROADMAP.md](/home/burgess/Work/krusty/KRUSTY_MAKO_ROADMAP.md)

## Quality Bar
All implementation work must satisfy these constraints:

- Modular: Mako code should live in Mako-specific modules, not spread as conditionals through generic chat code.
- Elegant: preserve the existing architecture instead of adding special-case branching everywhere.
- Performant: avoid opening many SSE streams, avoid unnecessary rerenders, and keep mobile navigation/state lightweight.
- Additive: prefer adding Mako-specific routes, types, and components over overloading generic chat semantics.
- Typed: keep server, API client, and mobile types aligned and explicit.

## Product Slice
Phase 1 delivers the first credible Mako experience.

### In scope
- `Current` as the top-level Mako home
- `Runs`, `Reports`, and `Status` as top-level Mako navigation
- `Set course` as the user-facing action for new work
- run-level navigation with `Overview`, `Wake`, `Tasks`, `Chat`, `Artifacts`
- a true recurring `TickEngine`
- durable runtime state that distinguishes awake, sleeping, paused, blocked, and completed
- `MAKO.md` prompt injection for Mako sessions only

### Out of scope
- `Deep current` implementation
- broad schedule editor UX beyond summary visibility
- remote gateway, nodes, devices, or browser control features
- large plugin or extension work
- replacing the TUI with a Mako-first control plane

## Brand Chrome
The top-level Mako chrome should use this lockup:

```text
Mako                         (awake)
Always Swimming.
```

Rules:
- `Mako` is the title
- the right side is a compact runtime state chip such as `awake`, `sleeping`, `paused`, `blocked`
- `Always Swimming.` is the persistent subtitle/tagline on the top-level Mako home
- do not overload other screens with themed copy; run views should stay more operational

This is a Mako-specific chrome treatment, not a global Krusty pattern.

## Information Architecture

### Top-level Mako navigation
- `Current`
- `Runs`
- `Reports`
- `Status`

### Primary action
- `Set course`

### Run-level navigation
- `Overview`
- `Wake`
- `Tasks`
- `Chat`
- `Artifacts`

### Navigation rules
- `Current` is the landing surface for the Mako tab
- selecting a run from `Current` or `Runs` opens the run workspace
- `Wake` is the canonical event timeline for a run
- `Chat` is subordinate to `Current` and run views, not the root structure

## Screen Model

### `Current`
Must show:
- brand chrome
- `Set course`
- active run summaries
- waiting-on-you summary
- compact chat surface
- lightweight schedule/wake summary

Should not show:
- full transcript as the main content
- dense generic session list UI copied from Code

### `Runs`
Must show:
- active
- queued
- sleeping
- completed

Should be optimized for scanning and resuming, not for chat.

### `Reports`
Must show:
- existing reports
- memory/project knowledge summary

Phase 1 can wrap current report primitives rather than fully redesign them.

### `Status`
Must show:
- daemon/runtime state
- approvals waiting on user
- wake/sleep summary
- diagnostics/health summary

Phase 1 can be intentionally compact, but it must exist as a dedicated surface.

### Run workspace

#### `Overview`
High-level run state:
- title
- current phase
- next action
- summary

#### `Wake`
Canonical timeline:
- timestamped events
- tool or milestone summaries
- sleep/wake transitions
- user-message checkpoints

#### `Tasks`
- current autonomous tasks
- status chips
- ownership/progress

#### `Chat`
- direct run-level communication with Mako

#### `Artifacts`
- files
- reports
- tests
- diffs
- other outputs

## Architecture Strategy
Do not rewrite the whole mobile app around Mako yet.

Phase 1 should:
- keep the outer Expo tab shell intact
- introduce a dedicated Mako screen container inside the existing Mako tab
- build Mako-specific components under a dedicated component namespace
- avoid pushing more Mako branching into the generic chat shell

This keeps the change reversible and reduces regression risk.

## Proposed File Structure

### Mobile
Add a dedicated Mako component namespace:

```text
apps/mobile/components/mako/
  MakoScreen.tsx
  MakoTopBar.tsx
  MakoTopNav.tsx
  MakoCurrentView.tsx
  MakoRunsView.tsx
  MakoReportsView.tsx
  MakoStatusView.tsx
  MakoRunView.tsx
  MakoSetCourseComposer.tsx
  MakoRunList.tsx
  MakoWakeTimeline.tsx
  MakoStatusBadge.tsx
  hooks/
    useMakoCurrent.ts
    useMakoRun.ts
    useMakoNavigation.ts
```

Rules:
- do not make `apps/mobile/app/(tabs)/index.tsx` the permanent home of Mako logic
- use `index.tsx` only to hand off into `MakoScreen` when the Mako tab is active
- do not add more Mako conditionals to generic `ChatBar` or `AccordionControls` than strictly necessary

### API client
Extend existing Mako methods in:
- [client.ts](/home/burgess/Work/krusty/packages/api/src/client.ts)
- [types.ts](/home/burgess/Work/krusty/packages/api/src/types.ts)

Add types for:
- `MakoCurrentResponse`
- `MakoCurrentRunSummary`
- `MakoRunWakeEvent`
- `MakoStatusSummary`

### Server
Extend:
- [mako.rs](/home/burgess/Work/krusty/crates/krusty-server/src/routes/mako.rs)
- [types.rs](/home/burgess/Work/krusty/crates/krusty-server/src/types.rs)
- [mako_runtime.rs](/home/burgess/Work/krusty/crates/krusty-server/src/mako_runtime.rs)

### Core runtime
Extend:
- [tick_engine.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/tick_engine.rs)
- [loop_events.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/loop_events.rs)
- [context.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/context.rs)
- [project_settings.rs](/home/burgess/Work/krusty/crates/krusty-core/src/storage/project_settings.rs)

## Runtime Model
The runtime must stop behaving like one pass plus optional sleep.

### Required state model
Mako runtime state should explicitly support:
- `awake`
- `sleeping`
- `paused`
- `blocked`
- `completed`
- `failed`

This state should be persisted and surfaced cleanly in both `Current` and `Status`.

### Tick loop behavior
`TickEngine` must:
- re-enter after completed turns when enabled
- emit `TickInjected`
- persist tick count and last wake time
- preserve sleep duration and wake target
- stop cleanly on cancel
- not busy-loop while idle

### Sleep/wake semantics
`sleep` means:
- the run is still alive
- the runtime is intentionally idle
- the next wake should be observable in `Current` and `Status`

It should not feel like terminal completion.

## Server Contract

### Keep existing internal transport
Preserve:
- `POST /api/mako/dispatch`
- `GET /api/mako/sessions`
- `GET /api/mako/sessions/:id/status`
- `GET /api/mako/sessions/:id/events`

User-facing language becomes `Set course`, but transport can remain `dispatch`.

### Add aggregated `Current` route
Add:

```text
GET /api/mako/current
```

This route should return a server-aggregated snapshot for the top-level Mako home.

Suggested shape:
- runtime summary counts
- waiting-on-you count
- active run summaries
- sleeping run summaries
- next scheduled wake summary
- pending approvals count

Reason:
- `Current` should not open many SSE streams
- mobile should not assemble its main dashboard through N+1 requests

### Add normalized wake projection
Phase 1 can derive `Wake` from existing event streams, but the server types should explicitly support a normalized wake-event shape for the client.

## Mobile Data Strategy

### `Current`
Use:
- one aggregated snapshot request
- refresh on focus
- manual pull-to-refresh if needed

Do not:
- open one SSE stream per run
- render Current from raw generic chat messages

### Run view
Use:
- one active SSE stream for the selected run
- normalized local reducer/state hook for wake/task/chat/artifact updates

Reason:
- one run at a time is the right performance boundary
- `Wake` needs incremental event handling, not repeated full reloads

## `MAKO.md` Identity Layer

### Goal
Add a Mako-specific identity file without weakening existing repo rules.

### File
At project root:

```text
MAKO.md
```

### Semantics
`MAKO.md` is only for:
- proactivity
- working relationship
- interruption style
- autonomy boundaries
- Mako-specific voice and stance

It is not for:
- repo coding rules
- long-term memory
- changelogs
- task logs

### Injection rules
Implement in [context.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/context.rs):
- search for `MAKO.md` and `mako.md`
- inject only when `session_type == "mako"`
- keep `AGENTS.md` and related project files as the higher-level repo rules
- inject `MAKO.md` after project rules, before lower-level prompt append text

### Ordering target
For Mako sessions:
1. workspace context
2. environment context
3. persistent memory
4. project rule files (`AGENTS.md`, etc.)
5. `MAKO.md`
6. project prompt append
7. plan/tasks/reports/coordinator prompt

## Componentization Rules

### Do
- create Mako-specific components
- create Mako-specific hooks/selectors
- derive UI sections from typed Mako view models
- keep chat rendering reusable only where it still fits naturally

### Do not
- keep building Mako inside generic `chatContent`
- let `ChatBar` become the control plane
- model `Wake` as fake assistant/user message bubbles

## Performance Rules

### Mobile
- no per-run SSE fanout on `Current`
- memoize run lists and timeline projections
- keep top-level Mako navigation local and lightweight
- avoid expensive list rerenders by normalizing run/event data

### Server
- aggregate `Current` server-side
- keep event payloads additive and typed
- avoid repeated DB work per run when one summary query can answer the screen

### Runtime
- tick loop must sleep efficiently
- state persistence should be incremental, not full-session rewrites

## Implementation Sequence

### Step 1: Mako screen split
Build:
- `MakoScreen`
- `MakoTopBar`
- `MakoTopNav`
- `MakoCurrentView`
- `MakoRunView`

Refactor:
- [index.tsx](/home/burgess/Work/krusty/apps/mobile/app/(tabs)/index.tsx) so the Mako tab delegates into Mako-specific UI rather than generic `chatContent`

### Step 2: Server summary contract
Build:
- `GET /api/mako/current`
- new API types for current summary and wake projection

Extend:
- [client.ts](/home/burgess/Work/krusty/packages/api/src/client.ts)
- [types.ts](/home/burgess/Work/krusty/packages/api/src/types.ts)

### Step 3: Run view and `Wake`
Build:
- `MakoWakeTimeline`
- run reducer/hook
- event normalization from SSE into wake entries

### Step 4: True tick loop
Refactor:
- [tick_engine.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/tick_engine.rs)
- [mako_runtime.rs](/home/burgess/Work/krusty/crates/krusty-server/src/mako_runtime.rs)

Deliver:
- recurring re-entry
- durable sleep/wake states
- clean surfaced runtime status

### Step 5: `MAKO.md`
Refactor:
- [context.rs](/home/burgess/Work/krusty/crates/krusty-core/src/agent/context.rs)

Deliver:
- Mako-only identity injection
- clear rule/identity separation

## Acceptance Criteria

### Product
- Mako no longer feels like Code with a different tab
- `Current` is the obvious home for the always-on assistant
- `Wake` feels like a real run timeline, not a transcript workaround
- `Set course` is the visible top-level action

### Runtime
- Mako survives beyond one orchestrator pass
- sleeping runs remain visibly alive
- runtime state maps cleanly to UI state chips

### Architecture
- Mako UI logic is concentrated in Mako-specific modules
- generic chat components do not accumulate Mako complexity
- no per-run SSE explosion on the top-level Mako home

## Non-Blocking Follow-Ons
These are expected next, but not required for Phase 1 completion:
- `Deep current`
- richer `Reports` redesign
- richer `Status` diagnostics
- schedule editor UX
- desktop-specific layout refinement

## Immediate Next Build
Start with:
1. `MakoScreen` and top-level `Current` shell
2. `/api/mako/current`
3. `MakoRunView` with `Wake`
4. true `TickEngine` loop
5. `MAKO.md` injection
