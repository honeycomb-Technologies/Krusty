# Krusty Mako Schedule Spec V1

## Purpose

`Schedule` is the planning surface for Mako.

It is not the active work surface.
It is not a diagnostics page.
It is not a dashboard.

It is where the user and Mako can see planned work across time.

## Core Product Rule

`Schedule` is primarily for visualization and light planning.

The agent has full control over creating, updating, and managing scheduled work.
The user should be able to review, adjust, and understand the plan without being forced into a heavy scheduler workflow.

## Main Questions This Tab Answers

`What is planned?`

`When will it run?`

`What team or run does it belong to?`

`What can I quickly adjust without leaving the view?`

## Default Layout

The default screen should be:

1. month calendar on top
2. daily agenda on bottom

This should feel like one integrated planning surface, not separate tabs competing for attention.

## View Modes

Required modes:

- `Month + Day`
- `Week`
- `Month`

Optional later:

- `Agenda only`

Rules:

- mobile and desktop should both support the same modes
- the difference is density, not capability
- `Month + Day` should be the default because it gives overview plus concrete detail at once
- the mode switch should use compact toggle buttons, not tabs or pills that dominate the screen
- the tab should stay list-first and utility-first, not card-heavy

## Default View: `Month + Day`

### Top Half

Month calendar:

- one visible month
- previous / next month navigation
- compact top toggle row for `Month + Day`, `Week`, and `Month`
- selected day state
- event presence shown using the Mako rounded-square marker language
- no dots
- no circular indicators

### Bottom Half

Selected-day agenda:

- chronologically ordered scheduled items for the selected day
- rendered as a compact list
- title
- time
- recurrence if relevant
- project / run / crew context if relevant
- tap to open the item detail or linked run

### Schedule Item Row

The default agenda row should stay minimal.

It should contain:

- title
- one short detail line
- time
- recurrence label if applicable
- optional project or crew context when useful

Example:

```text
Morning repo health check
krusty • reviewer crew
9:30 AM • weekdays
```

This is not a big card.
It is a compact planning row.

It should read more like a work list than a scheduler card.

### Schedule Item Detail

Tapping a scheduled item should open a detail sheet or drill-in view.

That detail should show:

- title
- short description or purpose
- exact timing
- recurrence pattern
- linked project
- linked crew if assigned
- linked run if one already exists

The preferred presentation is a minimal utility sheet with grouped rows, not a full management screen.

Recommended layout:

1. title
2. one short purpose/detail line
3. `Time`
4. `Repeats`
5. `Crew`
6. `Project`
7. small action row

The recurrence control should use explicit weekday toggle buttons:

- `S M T W T F S`
- active days are visibly toggled on
- changing the pattern nudges the series by default

The detail view is for understanding and making minimal adjustments, not for turning the item into a full run-management screen.

### Empty State

If the selected day has no items:

- show a calm empty state
- keep the calendar visible
- allow quick schedule creation from that day

## Week View

Purpose:

- tighter planning across several days
- better for team planning and comparing nearby work

Should show:

- day columns
- horizontal planning lanes
- more detailed markers than month view
- same event objects as month/day
- clearer list/block labeling than month view

Keep it restrained:

- no dense enterprise calendar chrome
- no unnecessary grid decoration

Week view should communicate more than month view.

Month view:

- broad placement
- color-coded presence
- compact rounded-square indicators

Week view:

- clearer item bars
- stronger labels
- easier comparison across nearby days
- enough detail to understand what is happening without opening every item
- still visually restrained, without dense enterprise calendar chrome

### Week View Model

Week view is not a vertical time-block calendar.

It should not assume:

- a fixed end time
- duration blocks stacked by hour
- enterprise calendar hour grids

Scheduled work in Mako starts at a time and completes when it completes.

So week view should show:

- start time
- title
- horizontal placement across one or more days

If the same scheduled item appears across consecutive days, it should collapse into one continuous horizontal bar spanning those days.

Example:

- a weekday repo health check should appear as one bar spanning Monday through Friday
- a Friday-only item should appear as a single-day bar

This keeps the week view compact and improves scanability.

The selected day should still drive the agenda below or beside the week view.

## Month View

Purpose:

- broad plan visualization
- long-range scheduling

Should show:

- month grid
- selected day
- color-coded event presence using the rounded-square language
- day selection opens or updates the day agenda below or in a lower pane/sheet

Month view should stay high-level.

It is for:

- seeing where work is clustered
- seeing which days are active
- selecting a day to inspect in the agenda below

It is not for reading every item in place.

## Schedule Item Model

Each scheduled item should support:

- `id`
- `title`
- `summary`
- `start_at`
- `recurrence`
- `project_dir` if relevant
- `run_id` if already created
- `crew_slug` if assigned
- `status`
- `created_by`
- `editable`

The `summary` field should be short and human-facing.
It is the detail line shown in the agenda row and item detail.

### Status

Useful statuses:

- `scheduled`
- `running`
- `completed`
- `paused`
- `cancelled`

These are for understanding context, not for turning Schedule into a run monitor.

## Primary Actions

Each schedule item should support minimal direct edits:

- edit name
- edit timing
- edit recurrence
- edit assigned crew
- open linked run
- open linked project
- pause or cancel if appropriate

The key rule is:

- lightweight edits happen here
- deep operational control happens in `Runs` or the main Mako thread

Minimal adjustments means:

- quick rename
- quick date/time change
- recurrence adjustment
- crew reassignment

It does not mean:

- rewriting the whole plan structure
- editing a run transcript
- changing deep runtime controls here

## Creation Model

There are two ways items should appear:

1. Mako creates them from conversation
2. user creates or adjusts them directly in `Schedule`

Examples:

- user tells Mako: `Run a repo health check every weekday morning`
- Mako creates the recurring schedule item
- user later opens `Schedule` and shifts it from `9:00 AM` to `9:30 AM`

This is the correct collaboration model.

## Editing Model

Editing should be minimal and local.

Allowed inline:

- rename
- time/date shift
- recurrence change
- crew assignment

Preferred interaction:

- tap item
- open detail sheet
- adjust the specific field directly
- save and stay in Schedule

Allowed in detail sheet:

- notes
- linked run/project context
- schedule provenance

Avoid:

- huge form-based editors
- multi-step wizard flows
- a separate product feeling inside `Schedule`

## Recurrence

Required first-class recurrence options:

- once
- daily
- weekdays
- weekly
- monthly

Weekly recurrence should use explicit day-of-week selection:

- Sunday
- Monday
- Tuesday
- Wednesday
- Thursday
- Friday
- Saturday

This should be shown as a simple check-row, not a complex rule builder.

Timing should be editable alongside day-of-week selection.

If a recurring schedule is adjusted:

- the default behavior should be to nudge the entire recurrence pattern
- the user should not have to adjust every instance one by one

Examples:

- moving `9:00 AM weekdays` to `9:30 AM weekdays` shifts the whole recurrence
- changing `Mon/Wed/Fri` to `Tue/Thu` updates the recurring schedule itself, not just one occurrence

This tab is about plan maintenance, not per-instance patchwork by default.

### Day Tap Behavior

Tapping a day should:

- select that day
- update the agenda below
- keep the user in the same schedule view

It should not immediately open a modal unless there is a strong reason.

### Item Tap Behavior

Tapping a schedule item should:

- open the schedule item detail
- show title and details
- show whether it is recurring
- show when it happens
- show quick edit affordances

If the item is linked to a run or project, the detail should also expose:

- `Open run`
- `Open project`

Advanced later:

- custom recurrence rules

But the UI should be designed so advanced recurrence can slot in later without redesigning the whole surface.

## Relationship To Other Tabs

`Mako`
- conversational control and companion updates
- Mako may mention schedule changes here

`Attention`
- only milestone or actionable schedule events belong there
- for example:
  - scheduled run started
  - scheduled run finished
  - scheduled item blocked or failed

`Runs`
- active operational detail

`Logbook`
- durable knowledge generated from scheduled work

## Visual Rules

Schedule should follow the newer Mako visual language:

- flat and list-first
- compact
- restrained chrome
- rounded-square markers
- no dots
- no bubbly card mosaics

The calendar should feel quiet and intentional.

## Library Direction

Adopt:

- `react-native-calendars`

Reason:

- open source
- Expo-compatible
- supports month/calendar and agenda patterns
- mature enough for mobile and web
- can be wrapped behind a Krusty adapter

## Adapter Rule

Do not let the calendar library become the product model.

Krusty should own:

- event model
- recurrence model
- actions
- linking to projects/runs/crew
- presentation rules

The library should only provide the calendar mechanics and base rendering primitives.

## First Implementation Cut

1. replace the current handmade month grid with a `react-native-calendars` adapter
2. keep the integrated `Month + Day` layout
3. preserve `Agenda` and `Week` as configurable modes
4. keep direct tap-to-edit for time/title/recurrence lightweight
5. use rounded-square markers consistently

## Non-Goals

Not needed for V1:

- enterprise calendar complexity
- external calendar syncing
- drag-and-drop across every surface
- separate schedule diagnostics
- making Schedule the main operational home

The tab is for planning clarity and light adjustment, not replacing Mako or Runs.
