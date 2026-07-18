# Krusty Mako Implementation Plan V2

This plan implements the replacement Mako IA and screen map defined in:
- [KRUSTY_MAKO_PRODUCT_MODEL_V3.md](./KRUSTY_MAKO_PRODUCT_MODEL_V3.md)
- [KRUSTY_MAKO_REPLACEMENT_IA.md](./KRUSTY_MAKO_REPLACEMENT_IA.md)
- [KRUSTY_MAKO_SCREEN_MAP_V2.md](./KRUSTY_MAKO_SCREEN_MAP_V2.md)

It is explicitly a replacement plan for the current multi-surface Mako shell.

## Goal
Replace the current Mako UI and context model with:
- one chat-first `Mako` home
- one full-capability `Schedule`
- one unified `Logbook`
- one simplified run detail
- one secondary `Runs` surface for operational scanning
- one real `Crew` model
- one real `Channels` model
- subordinate diagnostics and approval sheets

This is not only a UI rewrite.

It is also a correction of Mako's state ownership:
- Mako identity moves to global Krusty state
- project directories remain run and knowledge context, not persona definition

Visual direction:
- flat and dense over soft and bubbly
- grouped rows and dividers over floating cards where possible
- restrained corner radius and minimal pill usage
- mobile-first information density without dashboard clutter

## Current Surface Mapping
Current Mako components:
- `MakoScreen.tsx`
- `MakoCurrentView.tsx`
- `MakoChatView.tsx`
- `MakoRunsView.tsx`
- `MakoReportsView.tsx`
- `MakoMemoryView.tsx`
- `MakoStatusView.tsx`
- `MakoRunView.tsx`

Replacement mapping:
- current top-level `Current` + `Chat` + parts of `Status` -> new `Mako` home
- current `Runs` -> retained, simplified
- current scheduled wake snippets + schedule picker -> new first-class `Schedule`
- current `Reports` + `Memory` -> `Logbook`
- current `RunView` -> simplified structured run detail
- current top-level `Status` -> `Details` sheet
- no current equivalent -> `Crew`
- no current equivalent -> `Channels`

## Keep / Replace

### Keep
- `MakoTopBar.tsx`
- `Set course` language
- runtime status chip
- `MakoWakeTimeline.tsx`
- scheduling logic
- approvals logic
- reports + memory data model
- `ChatTranscript.tsx`
- `ChatBar.tsx`
- current daemon/runtime model

### Replace or Restructure
- project-root-bound Mako identity loading
- top-level Mako nav
- current `MakoCurrentView.tsx`
- top-level `MakoChatView.tsx`
- top-level `MakoStatusView.tsx`
- current `MakoReportsView.tsx` information model
- mobile `MakoRunView.tsx` section model
- current absence of soul / identity / heartbeat separation
- current absence of crew and channels as explicit product objects

## Replacement Navigation
Top-level Mako nav becomes:
- `Mako`
- `Schedule`
- `Logbook`

Remove:
- top-level `Chat`
- top-level `Status`
- top-level `Reports`

Add subordinate surfaces:
- `Details` sheet
- `Runs` sheet or screen
- `Approval` sheet
- `Schedule editor` sheet
- `Crew` sheet or screen
- `Channels` sheet or screen

## State Model Changes
The main architectural change is separating global Mako state from per-project state.

### New global Mako home
Recommended path family:
- `~/.krusty/mako/MAKO_SOUL.md`
- `~/.krusty/mako/MAKO_IDENTITY.md`
- `~/.krusty/mako/MAKO_HEARTBEAT.md`
- `~/.krusty/mako/MAKO_MEMORY.md`
- `~/.krusty/mako/MAKO_CHANNELS.md`
- `~/.krusty/mako/crew/<agent>/...`

### Existing per-project state retained
- `<project>/.krusty/settings.json`
- `<project>/.krusty/reports/`
- other project-scoped data

Rule:
- global state defines who Mako is
- project state defines what a run is acting on

## Data Contract Changes
Most runtime data already exists. The main work is aggregation, reshaping, and adding global Mako configuration/identity support.

### Existing data we can reuse
- current Mako aggregate summary
- approvals list
- wake summaries
- run lists
- report and memory lists
- run status and timeline

### New or reshaped client contracts
- `MakoProfile`
  - soul metadata
  - identity
  - heartbeat summary
  - channel summary
- `MakoHomeResponse`
  - status line summary
  - `focus_queue`
  - home thread summary or linked active chat context
- `MakoScheduleResponse`
  - scheduled items across relevant time windows
  - grouped agenda buckets
  - calendar-friendly date-keyed items
- `MakoLogbookResponse`
  - recent items
  - findings
  - memory
  - snapshots
- `MakoDetailsResponse`
  - daemon health
  - queue pressure
  - wake drift
  - failure streaks
- `MakoCrewResponse`
  - crew members
  - role
  - identity
  - status
  - current assignment
- `MakoChannelsResponse`
  - channels
  - delivery role
  - status
  - allowed targets

## Phase Plan

## Phase 1: Global Mako Home And Context Files
### Objective
Move Mako identity out of the active project root and into global Krusty state.

### Work
- add path helpers for global Mako home under `~/.krusty/mako/`
- define file loading for:
  - `MAKO_SOUL.md`
  - `MAKO_IDENTITY.md`
  - `MAKO_HEARTBEAT.md`
  - `MAKO_MEMORY.md`
  - `MAKO_CHANNELS.md`
- preserve project overlays for runs and knowledge
- keep current `MAKO.md` only as a migration shim until the new model is live

### Files
- `crates/krusty-core/src/paths.rs`
- `crates/krusty-core/src/agent/context.rs`
- new Mako profile loader module in `crates/krusty-core/src/storage/` or `src/agent/`

### Result
Mako identity is global and no longer directory-bound.

## Phase 2: Prompt Layering For Soul / Identity / Heartbeat
### Objective
Make Mako's liveness explicit in runtime context instead of collapsing everything into one `MAKO.md`.

### Work
- separate context injection for:
  - soul
  - identity
  - heartbeat
  - memory
  - channels
- keep project context attached to runs and knowledge, not persona
- add heartbeat-specific guidance as its own layer
- stop treating one project-root markdown file as the whole Mako identity

### Files
- `crates/krusty-core/src/agent/context.rs`
- `crates/krusty-core/src/agent/coordinator_prompt.rs`
- new Mako profile/context helpers

### Result
Mako has a real soul / identity / heartbeat stack.

## Phase 3: Crew Model
### Objective
Introduce real persistent agents with identity instead of anonymous background workers.

### Work
- define crew member model:
  - id
  - name
  - role
  - identity
  - soul
  - memory scope
  - tool permissions
  - optional channels
- add storage and runtime loading
- define how Mako delegates to crew members and how updates fold back into the main thread

### Files
- `crates/krusty-core/src/storage/`
- `crates/krusty-core/src/agent/`
- `crates/krusty-server/src/routes/mako.rs`

### Result
Mako can own a real team.

## Phase 4: Chat-First Shell Replacement
### Objective
Replace the current top-level Mako shell with the actual chat-first home.

### Work
- update `MakoScreen.tsx`
- keep top-level nav to:
  - `Mako`
  - `Schedule`
  - `Logbook`
- collapse current `Current`, top-level `Chat`, and status fragments into one main thread home
- add:
  - status line
  - focus queue
  - thread
  - composer
- open `Runs`, `Details`, `Crew`, and `Channels` as secondary surfaces

### Files
- `apps/mobile/components/mako/MakoScreen.tsx`
- `apps/mobile/components/mako/MakoCurrentView.tsx`
- `apps/mobile/components/mako/MakoChatView.tsx`
- `apps/mobile/components/mako/MakoTopNav.tsx`
- new `MakoHomeView.tsx`
- new `MakoFocusQueue.tsx`

### Result
Mako becomes one living controller thread instead of a dashboard.

## Phase 5: First-Class `Schedule`
### Objective
Make schedule a primary surface with equal mobile and desktop capability.

### Work
- add `Agenda` / `Calendar` toggle
- support full mobile calendar view
- support selected-day item list
- support editing from calendar items
- support creation from `Set course`
- add schedule editor sheet

### Files
- `apps/mobile/components/mako/MakoScheduleView.tsx`
- `apps/mobile/components/mako/MakoCalendarView.tsx`
- `apps/mobile/components/mako/MakoAgendaView.tsx`
- `apps/mobile/components/mako/MakoScheduleEditorSheet.tsx`
- adapt existing `MakoSchedulePicker.tsx`
- extend `schedule.ts`

### Backend/API
- expose a schedule-friendly endpoint or expand current aggregate route
- include date-grouped scheduled items for agenda and calendar rendering

### Result
Schedule becomes a real planning tool, not a side control.

## Phase 6: `Logbook`
### Objective
Replace `Reports` with a unified knowledge surface.

### Work
- rename user-facing `Reports` to `Logbook`
- unify:
  - reports
  - memory
  - findings
  - snapshots
- keep list-detail model simple

### Files
- replace `apps/mobile/components/mako/MakoReportsView.tsx`
- absorb or embed `apps/mobile/components/mako/MakoMemoryView.tsx`
- add `apps/mobile/components/mako/MakoLogbookView.tsx`

### Result
Knowledge becomes intentional instead of report-card-like.

## Phase 7: Run Detail Simplification
### Objective
Replace five-tab mobile run detail with one structured page.

### Work
- remove top-level run tabs on mobile
- create one scroll view with sections:
  - summary
  - wake
  - tasks
  - chat
  - artifacts
  - crew activity when relevant
- keep desktop segmented controls only if needed

### Files
- refactor `apps/mobile/components/mako/MakoRunView.tsx`
- reuse `MakoWakeTimeline.tsx`
- preserve run chat via `ChatTranscript.tsx` and `ChatBar.tsx`

### Result
Run detail feels coherent and easier to follow on phone.

## Phase 8: Secondary Operational Surfaces
### Objective
Move non-primary concerns out of top-level nav while preserving their usefulness.

### Work
- add `Details`
- add `Runs`
- add `Crew`
- add `Channels`
- add `Approval`
- add `Schedule editor`

### Files
- `apps/mobile/components/mako/MakoStatusView.tsx`
- `apps/mobile/components/mako/MakoRunsView.tsx`
- new `MakoCrewView.tsx`
- new `MakoChannelsView.tsx`
- new sheet components

### Result
The shell stays simple while real operational depth remains available.

## Phase 9: Polish
### Objective
Make the experience feel calm and intentional instead of crowded.

### Work
- tighten spacing
- reduce card count
- flatten overly rounded chrome
- use restrained section transitions
- improve hierarchy around the focus queue and thread
- ensure mobile-first readability

## Suggested Delivery Order
1. global Mako home and context files
2. prompt layering for soul / identity / heartbeat
3. crew model
4. chat-first shell replacement
5. `Schedule`
6. `Logbook`
7. run detail simplification
8. secondary operational surfaces
9. polish

## Cut List From Current UI
These should be removed or demoted during the rewrite:
- project-root-bound Mako identity
- top-level `Chat`
- top-level `Status`
- top-level `Reports`
- metric-card-heavy home layout
- multi-tab mobile run detail
- competing sibling surfaces on the Mako home
- anonymous-worker mental model for delegated agents

## Acceptance Criteria
The replacement is successful when:
- users can understand Mako from the home screen in under a few seconds
- Mako no longer feels defined by the selected directory
- Mako feels like a living system instead of a dashboard
- crew, channels, and memory feel like coherent parts of the product rather than implementation leftovers
- approvals cannot be lost in transcript history
- scheduled work is understandable in both agenda and calendar form on mobile and desktop
- `Logbook` clearly explains what stored knowledge is
- run detail is easier to use on phone than the current tabbed structure

## Immediate Build Slice
If implementation starts now, the first slice should be:

1. replace top-level nav in `MakoScreen.tsx`
2. create `MakoHomeView.tsx`
3. wire status strip, focus stack, and thread
4. move diagnostics behind a `Details` sheet

That gets the product out of the current conceptual dead-end fastest.
