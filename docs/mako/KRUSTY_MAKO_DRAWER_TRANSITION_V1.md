## Mako Drawer Transition V1

### Goal

Make `Mako` feel global, chat-first, and alive.

The drawer should stop presenting `Mako` as a directory-grouped session mode.
`Chat` and `Code` can stay session-oriented.
`Mako` should become a small set of durable product surfaces.

### Product Rule

- `Chat` is direct conversation.
- `Code` is direct project execution.
- `Mako` is the persistent control plane.

Project directories belong to `runs`, not to `Mako` itself.

### Minimal Drawer Model

For now, the Mako drawer should show only:

- `Mako`
- `Attention`
- `Schedule`
- `Logbook`
- `Runs`

This is the smallest useful model.

### What To Remove

Remove from the Mako drawer:

- directory grouping
- folder icons
- directory counts
- per-directory accordions
- `new session with directory` behavior
- first-level `Crew`
- first-level `Channels`
- first-level `Details`

These can exist in the product, but they should not sit in the first-open drawer.

### What Each Drawer Item Means

#### `Mako`

Opens the main controller thread.

This is the default home.
It should contain:

- a thin status strip
- one focus block
- the main thread
- the composer

#### `Attention`

Shows anything blocked on the user.

Examples:

- approvals
- follow-up questions
- blocked runs
- missing credentials or setup

This should behave like a queue, not a second dashboard.

#### `Schedule`

Shows all scheduled work.

Must support:

- agenda view
- calendar view
- one-time runs
- recurring runs
- opening and editing a scheduled item

#### `Logbook`

Durable knowledge.

Contains:

- reports
- memory
- snapshots
- promoted findings

This should replace the current fragmented `Reports` mental model.

#### `Runs`

Operational list only.

Contains:

- active
- sleeping
- queued
- completed

This is where project-specific run context belongs.

### What Moves Out Of The Drawer

These should move behind secondary navigation inside `Mako`, not stay first-level in the drawer:

- `Crew`
- `Channels`
- `Details`

Suggested placement:

- `Crew` and `Channels` under a `Manage` sheet or presence/settings entry
- `Details` behind the status strip or a diagnostics action

### First-Open Mako Home

The first-open Mako home should be:

1. status strip
2. focus block
3. main thread
4. composer

Optional below-the-fold items:

- small upcoming schedule preview
- one active run preview

Not on the first-open surface:

- multi-panel dashboards
- metric mosaics
- direct project directory selection
- full crew management
- diagnostics-heavy surfaces

### Transition Rule

Keep project scope, but move it one level down.

Old model:

- Mako grouped by directory

New model:

- Mako is global
- runs carry `project_dir`
- schedule items carry `project_dir`
- logbook entries can still be project-scoped when opened or filtered

### Current Code Drift

#### Drawer Drift

[SessionDrawer.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/SessionDrawer.tsx)

- groups Mako by directory via `groupSessionsByDirectory('mako')`

[SessionList.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/SessionList.tsx)

- groups Mako by directory via `groupSessionsByDirectory('mako')`
- renders Mako as a folder accordion just like `Code`

#### Top-Level Mako Drift

[index.tsx](/home/burgess/Work/krusty/apps/mobile/app/(tabs)/index.tsx)

- passes `workspaceDirectory` into `MakoScreen`

[MakoScreen.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoScreen.tsx)

- still treats `workspaceDirectory` as a top-level screen concern
- still exposes secondary surfaces in the same top-level flow

[MakoCurrentView.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoCurrentView.tsx)

- still opens `Crew`, `Channels`, `Runs`, and `Details` as first-order actions from the main home
- still carries a denser control surface than needed

### Exact UI Cut List

#### Phase A: Drawer Replacement

Update:

- [SessionDrawer.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/SessionDrawer.tsx)
- [SessionList.tsx](/home/burgess/Work/krusty/apps/mobile/components/chat/SessionList.tsx)

Changes:

- stop grouping Mako sessions by directory
- replace the Mako accordion list with a static Mako nav list
- keep the existing session list behavior for `Chat`
- keep the existing project grouping behavior for `Code`
- remove the directory picker footer when `Mako` is selected

New Mako drawer rows:

- `Mako`
- `Attention`
- `Schedule`
- `Logbook`
- `Runs`

#### Phase B: Home Simplification

Update:

- [MakoScreen.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoScreen.tsx)
- [MakoCurrentView.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoCurrentView.tsx)
- [useMakoNavigation.ts](/home/burgess/Work/krusty/apps/mobile/components/mako/hooks/useMakoNavigation.ts)

Changes:

- remove `workspaceDirectory` from the top-level Mako shell
- make `Mako` the true default and primary route
- reduce visible first-order surfaces to:
  - `Mako`
  - `Schedule`
  - `Logbook`
- make `Runs` secondary
- move `Crew`, `Channels`, and `Details` behind secondary navigation
- keep the status strip tappable into diagnostics

#### Phase C: Project Scope Reattachment

Update:

- [MakoScheduleView.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoScheduleView.tsx)
- [MakoRunView.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoRunView.tsx)
- [MakoLogbookView.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoLogbookView.tsx)
- [MakoReportsView.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoReportsView.tsx)
- [MakoMemoryView.tsx](/home/burgess/Work/krusty/apps/mobile/components/mako/MakoMemoryView.tsx)

Changes:

- keep project scoping only on runs, schedule items, and filtered knowledge
- stop treating project scope as the identity of Mako
- allow filtering the logbook by project without making Mako itself feel project-bound

### Implementation Order

1. replace the Mako drawer
2. simplify first-open Mako home
3. demote `Crew`, `Channels`, and `Details`
4. remove top-level `workspaceDirectory` dependency from Mako shell
5. keep project scope attached to runs and schedule

### Success Check

The redesign is successful if:

- Mako no longer looks like a foldered session mode
- the first thing a user sees is the Mako thread, not a control dashboard
- directories still matter for work, but only when opening runs or scheduling project work
- the drawer feels minimal and obvious
