# Krusty Mako Replacement IA

## Purpose
This document turns the corrected Mako product model into the information architecture that should replace the current shell.

Canonical product model:
- [KRUSTY_MAKO_PRODUCT_MODEL_V3.md](./KRUSTY_MAKO_PRODUCT_MODEL_V3.md)

The goal is to make Mako feel like:
- one always-on companion thread
- one clear control surface for automation
- one living system with soul, identity, heartbeat, memory, channels, and crew

This replaces the older dashboard-oriented direction.

## Product Statement
`Code` is where the user works directly with Krusty.

`Mako` is where the user directs, monitors, and steers the always-on system.

Mako is:
- assistant-first
- chat-first
- globally alive

Mako is not:
- project-root-defined
- directory-first
- a dashboard made of internal runtime categories

## Design Principles
- Chat-first, but not chat-only.
- Mako identity should come from global Mako state, not the active project root.
- Operational state must never depend on the user scrolling through transcript history.
- The home screen must answer "what is happening now?" before anything else.
- Approvals, schedules, crew activity, and active work must remain durable objects, not transient chat bubbles.
- Mobile and desktop are equal in capability.
- Deep diagnostics must exist, but they should not dominate the default experience.

## State Ownership
Mako should live in global Krusty state and layer projects into runs.

Recommended split:
- global Mako home: `~/.krusty/mako/`
- per-project state: `<project>/.krusty/`

That means:
- soul is global
- identity is global
- heartbeat is global
- core memory is global
- channels are global
- crew definitions are global
- project overlays shape run context and knowledge, not who Mako is

## Replacement Top-Level IA
The top-level Mako structure should be:

- `Mako`
- `Schedule`
- `Logbook`

Secondary surfaces:
- `Runs`
- `Details`
- `Crew`
- `Channels`

There should not be a top-level `Chat`.
There should not be a top-level `Status`.
There should not be a top-level `Reports`.

`Details`, `Crew`, `Channels`, and `Runs` are supporting surfaces opened from the thread, status line, or linked objects.

## Home Surface: `Mako`
`Mako` is the primary home and the default landing surface.

It should contain:
- brand chrome
- current Mako identity and liveness state
- an at-a-glance status line
- one adaptive focus queue
- the main Mako conversation thread
- the main `Set course` composer

It should not contain:
- dense metric grids
- multiple equal-weight panels competing with the thread
- raw diagnostics by default
- project-root identity chrome

### Home Layout
The home surface should be structured in this order:

1. `Mako (awake)` + `Always Swimming.`
2. compact status line
3. focus queue
4. main thread
5. `Set course` composer

### Status Line
The status line is compact and glanceable.

It should show:
- current runtime state
- running count
- waiting approvals count
- next wake time
- health

Example:

```text
Mako                         (awake)
Always Swimming.

2 running • 1 waiting • next wake 9:00 AM • healthy
```

Tapping the status strip opens `Details`, which contains:
- daemon health
- queue pressure
- wake drift
- failure streaks
- runtime diagnostics

`Details` is a subordinate sheet, not a main destination.

### Focus Queue
The focus queue replaces separate `Attention`, `Now`, and `Upcoming` sections.

It is the single compact area between the status line and the thread.

It should show only the most important 1-3 operational objects:
- approvals or blockers first
- active run second
- next scheduled item third

If one category has nothing important, it does not reserve space.

This keeps the home screen adaptive instead of making multiple permanent sections compete for attention.

Example:

```text
Focus
[Approval] Restart daemon on workspace A
[Run] PR merge validation • running now
[Next] Auth cleanup • today 9:00 AM
```

### Main Thread
This is the primary conversation with Mako.

The thread should contain:
- user messages
- Mako updates
- run summaries
- inline approval cards
- inline schedule cards
- completion summaries
- small intervention actions
- crew updates when relevant

The thread should not be the only place where these objects live.

## Durable Object Rules
Important operational objects must have more than one representation.

### Approvals
Approvals must exist in three places:
- inline in the thread when they occur
- in the focus stack or approval queue until resolved
- in the run history after resolution

This prevents them from getting lost in chat while preserving conversational continuity.

Approval cards should support:
- approve
- deny
- open run
- explain

### Scheduled Work
Scheduled work must exist in three places:
- inline in the thread when created or changed
- in `Schedule` as the canonical planning surface
- in the associated run history and summary

### Reports and Memory
Knowledge artifacts must exist in two places:
- referenced inline in thread when relevant
- stored in `Logbook` as the canonical knowledge surface

### Crew Activity
Crew activity must exist in three places:
- inline in the thread when it changes user-visible state
- in `Crew` as the operational list of agents
- in the parent run history

## `Runs`
`Runs` remains an important operational surface, but it is secondary rather than top-level.

It should open from:
- the status strip
- a `View all runs` action on the home screen
- schedule items or run links in the thread

`Runs` is the operational scanning surface.

It should contain:
- active
- sleeping
- queued
- completed

It should be optimized for:
- scanning
- resuming
- filtering
- opening a run quickly

It should not try to be a second home screen.

Recommended filters:
- `Active`
- `Sleeping`
- `Queued`
- `Completed`
- `Needs intervention`

## `Schedule`
`Schedule` is the canonical view for future work.

It should support:
- one-time runs
- daily runs
- weekly runs
- later cron-style advanced options

### Capability Rule
Mobile and desktop must be equal in scheduling capability.

The difference between them should be spatial presentation, not feature availability.

That means both mobile and desktop must support:
- `Agenda`
- `Calendar`
- schedule creation
- schedule editing
- recurring patterns
- day-level inspection
- run-level editing from scheduled items

### Mobile Schedule UX
Mobile should support both:
- `Agenda`
- `Calendar`

`Agenda` can be the default entry for readability, but `Calendar` must be a first-class toggle, not a hidden or reduced secondary mode.

Agenda should show:
- today
- tomorrow
- this week
- missed or overdue wakes

Calendar should support:
- month grid
- day tap
- selected-day schedule list
- recurrence visibility
- editing from the selected day

### Desktop Schedule UX
Desktop should support the same modes:
- `Agenda`
- `Calendar`

Desktop can simply show more at once:
- wider agenda density
- richer month grid context
- more visible day details beside the calendar

### Schedule Interaction
Creating or editing schedule should be possible from:
- `Set course`
- run detail
- `Schedule`

But `Schedule` is the canonical place to understand the whole system across time.

## `Logbook`
`Logbook` replaces the current separate `Reports` concept as the user-facing knowledge surface.

It should feel like:
- accumulated knowledge
- durable memory
- findings and snapshots

It should not feel like a card deck of report files.

## `Crew`
`Crew` is a secondary operational surface.

It should show:
- active agents
- role
- identity
- current assignment
- state
- latest update

Crew members are not anonymous workers. They are distinct presences inside Mako's system.

## `Channels`
`Channels` is a secondary routing surface.

It should show:
- which channels Mako can speak on
- where approvals can route
- where heartbeat nudges can surface
- which crew members can use which channels

Channels is not a peer home screen. It is a supporting control surface.

It should unify:
- reports
- memory
- promoted findings
- current snapshots
- searchable historical knowledge

It should feel like:
- durable knowledge
- useful history
- project understanding

It should not feel like:
- a raw card list with unclear data classes

Recommended sections:
- `Recent`
- `Findings`
- `Memory`
- `Snapshots`

## Run Detail
Run detail should be simplified.

On mobile, it should not default to five top tabs.

Instead, mobile run detail should be one scrollable structured view with sections:
- summary
- wake
- tasks
- chat
- artifacts

Desktop can optionally keep section tabs or a segmented header, but the mobile model should be one continuous run page.

### Run Detail Must Show
- title
- current status
- next wake
- task progress
- recent wake events
- inline run chat
- artifacts and outputs

### Run Detail Must Not Show
- duplicated top-level status panels
- unrelated system-wide diagnostics
- parallel nav structures that compete with the main Mako home

## `Set course`
`Set course` remains the primary action language.

It should live in the main composer area and support:
- direct instructions
- follow-ups
- corrections
- optional schedule
- optional priority

The user should not have to decide whether they are "dispatching" versus "chatting."

Every meaningful instruction to Mako should begin from the same entry point.

## Information Density Rules
The Mako home should be trimmed aggressively.

### Keep
- one obvious thread
- one obvious input
- one status strip
- one durable inbox
- one small upcoming schedule preview

### Remove or Demote
- large metric cards
- top-level `Status`
- top-level `Chat`
- duplicate run lists on the home screen
- diagnostics that are not immediately actionable

## Suggested Mobile Wireframe
```text
Mako                         (awake)
Always Swimming.

2 running • 1 waiting • next wake 9:00 AM

Focus
[Approval] Restart daemon on workspace A
[Approve] [Deny] [Open]

[Run] PR merge validation
Running now • updated 2m ago
[Open run] [Pause] [Reschedule]

[Next] Today 9:00 AM  Auth cleanup
[Open schedule] [View all runs]

Thread
You: Check branch behavior after merge.
Mako: I merged the branch, started build 213, and I am waiting on processing.
Mako: I also found a pending restart approval.

[ Set course...                              ] [Send]
```

Bottom navigation:

```text
Mako | Schedule | Logbook
```

## Replacement Decisions
This replacement IA makes the following explicit changes to the previous plan:

- `Current` is renamed and collapsed into `Mako`
- top-level `Runs` is removed
- top-level `Chat` is removed
- top-level `Status` is removed
- top-level `Reports` becomes `Logbook`
- `Schedule` becomes a first-class surface
- approvals move from being mostly a status concern to a durable focus/inbox concern
- run detail becomes a sectioned page rather than a multi-tab mini-product on mobile

## Implementation Direction
If this IA is accepted, the next UI pass should:

1. replace top-level Mako nav with `Mako / Runs / Schedule / Logbook`
2. collapse current Mako home into the new chat-first home surface
3. collapse `Attention`, `Now`, and `Upcoming` into one adaptive focus stack
4. move diagnostics behind the status strip into a details sheet
5. redesign reports + memory as `Logbook`
6. create agenda + calendar `Schedule`
7. simplify run detail into one structured page on mobile

## Why This Is Better
This model is simpler because it matches how the user actually thinks:

- talk to Mako
- see what needs me
- see what is happening now
- view future scheduled work
- open durable knowledge when needed

It also preserves the real strength of the system:
- rich runtime state
- approvals
- scheduling
- memory
- reports

But it stops exposing all of those as equal-weight first impressions.
