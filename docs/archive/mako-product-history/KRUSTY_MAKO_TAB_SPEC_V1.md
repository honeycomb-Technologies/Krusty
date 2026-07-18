## Mako Tab Spec V1

### Purpose

The `Mako` tab is the main companion thread.

It is not a dashboard.
It is not a control panel.
It is not a project list.

It is the place where the user talks to Mako and where Mako reports back on work.

### Single Responsibility

The `Mako` tab answers one question:

`What is Mako doing, and what is it telling me right now?`

Everything else is secondary.

### Core Product Rule

The surface should feel like a normal chat stream.

Underneath, Mako can still own:

- runs
- schedule
- memory
- crew
- channels
- approvals

But those systems should only appear in the thread when they materially matter.

### Layout

The Mako tab should contain only:

1. a minimal header
2. the main thread
3. the composer

Optional:

- one small top status line

Not allowed:

- metric grids
- stacked dashboard cards
- management rows
- multiple peer panels
- top-level directory framing

### Header

The header should stay very light.

Recommended:

- `Mako`
- small state word like `awake`, `working`, or `idle`
- optional overflow/menu button

Possible structure:

```text
Mako                                 awake
Always Swimming.
```

The subtitle may stay if it does not take too much space.

### Thread

The thread is the primary product surface.

It should mostly be plain assistant and user messages.

The thread should read like a living companion conversation, not like an event log.

### Message Types

The thread should support six message types total.

#### 1. Conversation message

Normal user and Mako messages.

Used for:

- asking Mako to do something
- follow-up questions
- explanations
- check-ins
- summaries
- normal companion behavior

This should be the majority of the stream.

#### 2. Approval message

Used when the user must act.

Contains:

- short title
- plain-language reason
- source run/project
- actions:
  - `Approve`
  - `Deny`
  - `Open run`

This is the highest-priority rich object.

#### 3. Run update message

Used when Mako starts work or there is a meaningful state change.

Contains:

- run title
- short state label
- one-line summary
- project reference if relevant
- one action:
  - `Open run`

This should stay compact.

#### 4. Completion message

Used when work finishes and there is something worth opening.

Contains:

- completion summary
- result state
- actions linking into the work:
  - `Open project`
  - `Open run`
  - `Open report`
  - `Open files`
  - `Open diff`

This is the main place where deep links matter.

#### 5. Schedule message

Used when Mako creates, updates, delays, or confirms scheduled work.

Contains:

- what is scheduled
- when it will run
- recurrence if relevant
- actions:
  - `Open schedule`
  - `Edit`

#### 6. Crew message

Used when Mako creates, assigns, or meaningfully updates crew activity.

Contains:

- crew member or team name
- assignment summary
- status
- action:
  - `Open run`
  - or `View crew`

### Links

In this tab, links should mean links into work.

Not links into general app chrome.

Good links:

- `Open project`
- `Open run`
- `Open report`
- `Open files`
- `Open diff`
- `Open schedule`
- `View crew`

Bad links:

- generic links back to `Schedule`, `Runs`, `Details`, or other app tabs when a direct work object can be opened instead

### Project Linking

Project references should absolutely be available in the thread.

But they should not make the whole tab feel directory-based.

The rule is:

- Mako itself is global
- messages may reference projects
- rich objects may deep-link into a project or a project-scoped run

So `Open project` is valid and important.
Grouping the Mako tab by directories is not.

### Composer

The composer should be the normal Krusty composer.

Keep:

- text input
- attachments if already supported
- send
- stop while streaming

Same core AI controls can remain available, but they should be low-prominence.

### AI Controls

Keep the same underlying controls:

- model
- thinking
- permission mode
- research toggle if relevant

But move them into lightweight access:

- header sheet
- overflow menu
- compact picker

They should not visually dominate the Mako thread.

### What Gets Removed From The Current Mako Home

Remove from the Mako tab:

- metric strip as a primary element
- quick management rows
- presence rows on first-open
- crew/channels/details as visible first-order content
- schedule preview blocks unless they appear as inline messages
- any layout that makes the thread only one section among many

### What Stays Out Of The Thread

These should exist elsewhere, not as regular inline thread content:

- diagnostics
- channel editing
- large memory surfaces
- full reports
- long operational lists
- raw system metrics

Those belong in:

- `Attention`
- `Schedule`
- `Logbook`
- `Runs`
- secondary sheets

### Visual Rule

The thread should look almost exactly like a normal chat experience.

Rich objects should be:

- compact
- rare
- meaningful

If the screen looks like stacked components instead of a conversation, it has drifted.

### Default First-Open Experience

The first time a user opens the Mako tab, they should see:

1. header
2. Mako thread
3. composer

If Mako has something active, it can appear naturally in the thread as a message.

Example:

```text
Mako: I’m watching build 215 now.
Mako: I scheduled an auth cleanup run for tomorrow at 9:00 AM.
Mako: I need approval to restart the daemon on `krusty`.
[Approve] [Deny] [Open run]
```

This is better than showing a dashboard block above the thread.

### Success Criteria

The Mako tab is correct when:

- it feels like talking to a companion
- it still gives clear access to work and approvals
- project links are easy to open
- rich objects help without taking over the stream
- the tab does not feel like an admin console
