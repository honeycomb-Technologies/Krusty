# Krusty Mako Screen Map V2

This document turns the corrected replacement IA into a concrete screen map for both mobile and desktop.

It is based on:
- [KRUSTY_MAKO_PRODUCT_MODEL_V3.md](./KRUSTY_MAKO_PRODUCT_MODEL_V3.md)
- [KRUSTY_MAKO_REPLACEMENT_IA.md](./KRUSTY_MAKO_REPLACEMENT_IA.md)

## Frontend Direction
Visual thesis:
Mako should feel like a calm operational tide pool, not a dashboard wall; one living thread sits at the center, with status and actions orbiting it in restrained layers.

Visual rules:
- flat and list-first, not pill-first
- compact rows before stacked cards
- small radii, thin dividers, restrained surfaces
- status should read as a strip, not a cloud of chips
- settings and secondary surfaces should use grouped lists, not bubble panels

Content plan:
- home = identity, status line, focus queue, thread, composer
- support = schedule and run access
- depth = logbook and run detail
- operations = crew and channels as secondary surfaces
- diagnostics = details sheet, not top-level

Interaction thesis:
- one persistent conversational entry point
- bottom-nav mode changes, not nested tab stacks
- sheets for details, editing, and approvals instead of new full screens when possible

## Global Navigation
Top-level navigation:
- `Mako`
- `Schedule`
- `Logbook`

Subordinate surfaces:
- `Details` sheet
- `Runs` sheet or screen
- `Approval` sheet
- `Schedule editor` sheet
- `Crew` sheet or screen
- `Channels` sheet or screen
- `Run detail`

Rules:
- `Mako` is always the default landing view
- `Details` opens from the top status line
- `Runs` opens from the home screen or status line
- `Crew` opens from the home screen or run context
- `Channels` opens from the header or details
- `Run detail` opens from `Mako`, `Runs`, or `Schedule`
- `Approval` can open from inline cards, the focus queue, or a run
- `Schedule editor` can open from `Set course`, `Schedule`, or a run

## Screen 1: `Mako`

### Purpose
The main controller thread for the always-on system.

### Mobile Structure
```text
Header
- Mako
- (awake)
- Always Swimming.
- identity / crew / channels access

Status line
- 2 running
- 1 waiting
- next wake 9:00 AM
- healthy

Focus queue
- approval or blocker
- primary active run
- next scheduled item

Thread
- user turns
- Mako turns
- inline approval cards
- inline schedule cards
- run update cards
- crew updates

Composer
- Set course input
- send
- schedule shortcut
```

### Desktop Structure
```text
Top header
- Mako
- state chip
- Always Swimming.
- identity / crew / channels controls

Main column
- status line
- focus queue
- thread
- composer

Right rail
- selected active run summary
- next scheduled item
- quick details / diagnostics
- crew presence summary
```

### Header Behavior
The top status line opens `Details`.

The strip should not navigate away from the thread by default.

The header also exposes:
- current identity
- crew access
- channel access

### Focus Queue
This is the anchor block between the status line and the thread.

It should only show the most important 1-3 cards:
- approval or blocker first
- active run second
- next scheduled item third

Each item is a durable object, not only a transcript artifact.

Approval actions:
- `Approve`
- `Deny`
- `Open`
- `Explain`

Run and schedule cards should support:
- `Open run`
- `Pause`
- `Reschedule`
- `Open schedule`

### Active Run Card
This is the main anchor row when active work exists.

It should show:
- run title
- run state
- last update time
- one-line summary
- `Open run`
- one or two quick controls

### Thread Rules
The thread is chronological and conversational, but it is allowed to contain structured inline objects:
- approval cards
- schedule cards
- run summary cards
- completion cards
- promoted knowledge links

The thread should not be overloaded with dense metric blocks.

The thread should be the primary emotional center of the product.

### Composer Rules
`Set course` is the single primary entry point.

Below or beside the composer:
- `Schedule`
- `Attach context` later if needed

Do not create a separate "dispatch" concept in the UI.

## Screen 2: `Runs`

### Purpose
Operational scanning and resumption.

This is a secondary surface, not a persistent top-level tab.

## Schedule Note

See:
- [KRUSTY_MAKO_SCHEDULE_SPEC_V1.md](./KRUSTY_MAKO_SCHEDULE_SPEC_V1.md)

The preferred default schedule layout is:
- month calendar on top
- selected-day agenda below

Supported view modes should be:
- `Month + Day`
- `Week`
- `Month`

Schedule markers should follow the Mako rounded-square language.

### Structure
```text
Header
- Runs
- counts summary

Filter row
- Active
- Sleeping
- Queued
- Completed
- Needs intervention

Run list
- title
- state
- priority
- project
- next wake or last update
- one-line summary
```

### Mobile Behavior
Runs is a clean list, not a card grid.

Tapping a run opens `Run detail`.

Long-press or overflow can expose:
- pause
- resume
- reschedule
- cancel

### Desktop Behavior
Desktop may show:
- left list
- right preview pane

But it should still preserve the same list-first mental model.

## Screen 3: `Schedule`

### Purpose
Canonical planning surface for future work.

### Global Rule
Mobile and desktop are equal in capability.

Both must support:
- `Agenda`
- `Calendar`
- create
- edit
- recurrence
- day inspection

### Mobile Layout
```text
Header
- Schedule
- New

Mode toggle
- Agenda
- Calendar

If Agenda:
- Today
- Tomorrow
- This week
- overdue / missed

If Calendar:
- month grid
- selected date strip
- items for selected date below
```

### Desktop Layout
```text
Header
- Schedule
- New

Mode toggle
- Agenda
- Calendar

Main body
- calendar or agenda on the left
- selected day / selected item details on the right
```

### Schedule Item Card
Every scheduled item should show:
- title
- recurrence or one-time time
- target project
- state
- linked run when applicable

Actions:
- open
- edit
- pause
- run now

## Screen 4: `Logbook`

### Purpose
Durable project knowledge across Mako activity.

### Structure
```text
Header
- Logbook
- Search

Section tabs or filters
- Recent
- Findings
- Memory
- Snapshots

List
- title
- type
- source run or date
- project scope
```

### Item Types
`Logbook` must unify:
- reports
- memories
- promoted findings
- current snapshots

It should not visually force all of these into one card style if that obscures what they are.

### Mobile
Single-column searchable list.

Tapping an item opens a detail page or sheet.

### Desktop
List-detail layout is preferred.

## Screen 5: `Run detail`

## Screen 6: `Crew`

### Purpose
Operational view of Mako's active and available agents.

### Structure
```text
Header
- Crew
- active count

Agent list
- name
- role
- current status
- current assignment
- latest update
```

Rules:
- list-first
- no card wall
- opening an agent shows identity, role, permissions, memory scope, and active work

## Screen 7: `Channels`

### Purpose
Delivery and routing surface for Mako.

### Structure
```text
Header
- Channels

Channel list
- channel name
- enabled / disabled
- delivery purpose
- current health
- allowed targets
```

Rules:
- operational and compact
- not a daily destination
- most users arrive here from settings/details, not from routine home use

### Purpose
Focused view of one run.

### Mobile Model
One structured page with sections, not five peer tabs.

```text
Header
- run title
- current state
- back

Summary
- project
- status
- next wake
- priority

Wake
- most recent events

Tasks
- in progress
- pending
- completed

Chat
- run-specific thread
- inline interventions

Artifacts
- reports
- files
- diffs
- outputs
```

### Desktop Model
Desktop may use segmented sections or anchor tabs, but should still feel like one run workspace, not a separate product shell.

### Quick Actions
At top of run detail:
- pause / resume
- reschedule
- open approval
- open artifact

## Sheet 1: `Details`

### Purpose
Subordinate diagnostics and runtime state.

### Contents
- daemon health
- queue pressure
- wake drift
- failure streaks
- last error
- knowledge freshness

This is not a primary destination.

## Sheet 2: `Approval`

### Purpose
Expanded context for an approval item.

### Contents
- approval reason
- related run
- relevant tool or action
- consequence summary

Actions:
- approve
- deny
- open run

## Sheet 3: `Schedule editor`

### Purpose
Create or edit a scheduled run.

### Fields
- instruction
- target project
- one-time vs recurring
- date/time
- recurrence rule
- priority

### Modes
- quick preset
- detailed calendar selection

## Interaction Flows

### Flow A: Give Mako new work
1. user lands on `Mako`
2. user types in `Set course`
3. optional schedule sheet opens
4. run appears in thread and the focus stack or `Schedule`

### Flow B: Handle approval
1. approval appears inline in thread and in the focus stack
2. user taps `Approve`
3. optional expanded approval sheet opens when more context is needed
4. approval leaves the focus stack and remains as historical trace in thread/run

### Flow C: View future work
1. user opens `Schedule`
2. toggles `Agenda` or `Calendar`
3. taps scheduled item
4. edits or opens linked run

### Flow D: Inspect a run
1. user taps active item from the focus stack, `Runs`, or `Schedule`
2. `Run detail` opens
3. user sees status, wake, tasks, chat, artifacts in one structured page

## What Gets Removed From Current Mako
- top-level `Chat`
- top-level `Status`
- top-level `Reports`
- metric-card-heavy home composition
- five-tab mobile run detail
- equal visual weight across all Mako concepts

## What Gets Preserved
- `Always Swimming.`
- `Set course`
- `Wake`
- approvals
- scheduling
- memory
- reports
- strong runtime state underneath the UI
