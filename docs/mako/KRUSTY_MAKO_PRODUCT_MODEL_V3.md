# Krusty Mako Product Model V3

## Purpose
This document replaces the older Mako framing with the correct product model:

- Mako is a living top-level controller
- Mako is chat-first
- Mako is not directory-first
- Mako should feel alive through soul, identity, heartbeat, memory, channels, and crew

This is the canonical product model for the next Mako rewrite.

## The Core Correction
The previous Mako direction drifted because runtime mechanics became navigation.

We surfaced:
- runs
- status
- reports
- diagnostics
- scheduling controls

too directly in the shell.

That made Mako feel like:
- a project dashboard
- a directory-aware task console
- a shell for internal mechanics

instead of:
- an always-on companion
- a controller thread
- a living system with durable operational objects around it

The correct principle is:

`The assistant is the product. The control plane supports the assistant.`

## Product Statement
`Code` is where the user works directly with Krusty.

`Mako` is where the user directs, monitors, and steers the always-on system.

Mako should feel like:
- one living conversation
- one operational intelligence layer
- one memory-bearing presence
- one system that can run work, surface approvals, keep time, and remember

Mako should not feel like:
- a directory browser
- a card dashboard
- a task manager pretending to be a personality

## The Mako Stack
Mako needs six first-class layers.

### 1. Soul
Soul is Mako's voice, posture, attitude, and boundaries.

This is what makes Mako feel alive instead of generic.

Soul should define:
- tone
- bluntness
- warmth
- brevity
- stance
- default conversational rhythm

Soul is not:
- a tool policy dump
- a task ledger
- a project instruction file

### 2. Identity
Identity is what gives Mako stable presence.

Identity should define:
- name
- creature
- vibe
- theme
- emoji or presence marker
- visible status language
- channel-facing naming and prefix behavior

Identity is what makes Mako feel like someone, not just something.

### 3. Heartbeat
Heartbeat is Mako's recurring life loop.

Heartbeat is not just scheduler state.

Heartbeat should govern:
- periodic main-thread turns
- proactive check-ins
- quiet monitoring
- wake reasons
- what "idle but alive" actually means

Heartbeat should be small, explicit, and durable.

### 4. Memory
Memory is Mako's continuity.

Memory should include:
- curated durable memory
- recent operational memory
- project findings
- snapshots and state summaries

Memory is not the same thing as reports.

Reports are artifacts.
Memory is carried understanding.

### 5. Channels
Mako should exist across channels, not only inside the app shell.

Channels define:
- where Mako can speak
- where Mako can surface alerts or approvals
- how Mako routes updates
- how Mako behaves differently on private vs shared surfaces

Channels are a first-class system object, but not a top-level daily destination.

### 6. Crew
Mako should be able to operate a team of agents with distinct identities.

Crew members should not just be anonymous background workers.

Each crew member should have:
- identity
- role
- soul
- memory scope
- tool permissions
- optional channel behavior
- runtime status

Mako is the coordinator.
Crew members are distinct working presences.

## Mako Is Not Directory-Based
This is the most important architectural correction.

Mako should not derive its identity from the currently selected project root.

Today, Krusty already has:
- global state in `~/.krusty`
- per-project state in `<project>/.krusty`

See:
- [paths.rs](/home/burgess/Work/krusty/crates/krusty-core/src/paths.rs#L9)

The corrected Mako model should use that same split:

- global Mako home in `~/.krusty/mako/`
- per-project overlays in `<project>/.krusty/`

That means:
- Mako's soul is global
- Mako's identity is global
- Mako's heartbeat is global
- Mako's core memory is global
- projects become contextual scopes attached to runs, not the thing that defines who Mako is

## Proposed Mako Home Layout
The recommended global Mako home is:

```text
~/.krusty/mako/
  MAKO_SOUL.md
  MAKO_IDENTITY.md
  MAKO_HEARTBEAT.md
  MAKO_MEMORY.md
  MAKO_CHANNELS.md
  crew/
    researcher/
      IDENTITY.md
      SOUL.md
      MEMORY.md
    builder/
      IDENTITY.md
      SOUL.md
      MEMORY.md
    reviewer/
      IDENTITY.md
      SOUL.md
      MEMORY.md
```

Project directories still keep project-specific Krusty state:

```text
<project>/.krusty/
  settings.json
  reports/
  optional project memories or overlays later
```

Rule:
- project state shapes work
- Mako home shapes identity
- legacy generic names like `SOUL.md` remain compatibility fallbacks, but new Krusty-native setups should prefer `MAKO_*`

## Product Shell
The top-level product should be:

- `Mako`
- `Schedule`
- `Logbook`

Secondary surfaces:
- `Runs`
- `Details`
- `Crew`
- `Channels`
- `Approval`
- `Schedule editor`

This keeps one primary home while preserving real control surfaces.

## Home: `Mako`
`Mako` is one primary controller thread.

It should contain:
- a compact status strip
- a focus queue
- the main thread
- the main `Set course` composer

It should not contain:
- multiple peer dashboards
- permanent panel stacks
- exposed implementation categories everywhere

### Home Order
1. `Mako`
2. current status line
3. focus queue
4. main thread
5. composer

### Status Line
The status line should answer, at a glance:
- what Mako is doing
- what needs the user
- when it wakes next
- whether it is healthy

Example:

```text
Mako                         (awake)
Always Swimming.

2 running • 1 waiting • next wake 9:00 AM • healthy
```

### Focus Queue
The focus queue replaces multiple competing home panels.

It should show the top 1-3 important operational objects:
- unresolved approval or blocker
- primary active run
- next scheduled or overdue item

These objects also live elsewhere. They are not thread-only artifacts.

### Main Thread
This is the center of the product.

The thread contains:
- user turns
- Mako updates
- inline approvals
- inline schedule updates
- run summaries
- completion summaries
- crew updates when relevant

The thread is the living surface.

But important objects must remain durable outside it.

## Durable Object Rules
### Approvals
Approvals must appear in:
- the thread
- the focus queue until resolved
- run history after resolution

They must not be lost by scrolling.

### Scheduled Work
Scheduled work must appear in:
- the thread when created or changed
- `Schedule` as the canonical time surface
- run history

### Knowledge
Knowledge must appear in:
- the thread when relevant
- `Logbook` as the canonical durable surface

### Crew Activity
Crew activity must appear in:
- the thread when it changes user-visible state
- `Crew` as the canonical operational view
- the parent run timeline

## Schedule
`Schedule` is a first-class planning surface.

Both mobile and desktop must support:
- `Agenda`
- `Calendar`
- day inspection
- recurring runs
- editing from scheduled items

The difference is density, not capability.

## Logbook
`Logbook` replaces the shallow `Reports` mental model.

It should unify:
- reports
- durable memory
- findings
- snapshots

It is where the user goes to understand what Mako has learned.

## Crew
`Crew` is the operator view for distinct working agents.

It should show:
- active crew members
- identity and role
- current status
- assigned work
- recent updates

Crew members should be real named presences, not only task rows.

They can be:
- specialized
- semi-persistent
- assigned to different kinds of work

But Mako remains the primary conversational face of the system.

## Channels
`Channels` is a routing and delivery surface.

It should control:
- where Mako can send updates
- where approvals can surface
- where heartbeat nudges can go
- which channels each crew member can use

Channels is important, but should remain subordinate to the main Mako thread.

## What Changes From The Current Mako
These are the main corrections:

1. Mako identity moves out of the project root and into global Krusty state.
2. `MAKO.md` alone is no longer enough.
3. The shell becomes chat-first instead of panel-first.
4. `Runs`, `Status`, and `Reports` stop acting like peer homes.
5. `Crew` and `Channels` become real system concepts.
6. Projects shape runs and knowledge, not Mako's personality.

## Immediate Implementation Consequences
### Replace
- project-root-only Mako identity loading
- dashboard-like Mako home layout
- extra peer navigation surfaces

### Add
- global Mako home path support
- separate soul / identity / heartbeat loaders
- crew model and storage
- channel model for Mako delivery
- chat-first shell around existing runtime

## The Correct Build Order
1. establish global Mako home and context files
2. split soul / identity / heartbeat / memory in prompt injection
3. define crew and channel data model
4. replace Mako shell with one chat-first home
5. keep `Schedule` and `Logbook` as supporting surfaces
6. simplify `Runs` and `Details`

## Decision
Mako should be built as:
- a living controller
- with a soul
- with an identity
- with a heartbeat
- with memories
- with channels
- with a crew

Everything else should orbit that.
