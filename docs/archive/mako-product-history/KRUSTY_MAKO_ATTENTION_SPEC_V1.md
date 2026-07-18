## Mako Attention Spec V1

### Purpose

`Attention` is Mako's durable notification center.

It is the place for:

- actionable items
- milestone updates
- important completions
- things the user should be able to find again later

It is not:

- the full Mako conversation
- a dashboard
- a raw event log
- a diagnostics panel

### Relationship To Mako

The relationship is:

- `Mako` = narrative stream
- `Attention` = durable notable events

An important update can appear in both places:

- naturally in the `Mako` thread
- durably in `Attention`

### Core Rule

Only high-signal events belong in `Attention`.

If an event does not require action or does not represent a meaningful milestone, it should stay in the thread only.

### What Qualifies For Attention

#### Actionable items

- approval required
- input required
- blocked run
- missing setup that prevents progress

#### Milestone items

- run completed
- scheduled run started
- scheduled run completed
- delegated task completed
- meaningful crew completion

#### Intervention items

- run failed
- run stalled
- run degraded
- repeated failure detected

### What Does Not Qualify

Do not send these into `Attention` by default:

- ordinary conversation
- small progress nudges
- every tool use
- every state transition
- routine wake messages
- diagnostics-only updates
- passive schedule existence

### Item Types

The first version should support these item kinds:

- `approval_required`
- `input_required`
- `run_completed`
- `run_failed`
- `run_stalled`
- `scheduled_run_started`
- `scheduled_run_completed`
- `delegated_task_completed`

### Item Shape

Each `Attention` item should contain:

- `id`
- `kind`
- `title`
- `summary`
- `created_at`
- `resolved_at` optional
- `read`
- `resolved`
- `session_id` optional
- `run_id` optional
- `project_dir` optional
- `primary_action`
- `secondary_action` optional
- `thread_session_id` optional
- `thread_anchor_id` optional

The first version does not need complex threading inside `Attention`.

### Item Behavior

Each item should support:

- collapsed by default
- tap to expand
- `read` and `unread`
- `clear`
- `Open thread`
- direct open action into work when relevant

Rules:

- expanding an unread item marks it as read
- unread state can be manually restored
- resolved or informational items can be cleared
- unresolved approvals or active blockers should stay in the feed until resolved

### Default Actions

Each item should have one obvious primary action.

Examples:

- `approval_required`
  - `Approve`
  - secondary: `Open run`

- `input_required`
  - `Reply`
  - secondary: `Open run`

- `run_completed`
  - `Open run`
  - secondary: `Open project` or `Open report`

- `run_failed`
  - `Open run`

- `run_stalled`
  - `Open run`

- `scheduled_run_started`
  - `Open run`

- `scheduled_run_completed`
  - `Open run`
  - secondary: `Open report`

- `delegated_task_completed`
  - `Open run`
  - secondary: `View crew`

### Project Links

`Attention` should support direct links into work.

Good actions:

- `Open project`
- `Open run`
- `Open report`
- `Open files`
- `Open diff`

This is the right place for those deep links.

### Feed Sections

The first version should stay simple.

Two sections are enough:

- `Needs action`
- `Updates`

`Needs action` contains:

- approvals
- input requests
- blocked/failing items that require intervention

`Updates` contains:

- completions
- scheduled starts/completions
- delegated task completions

### Sorting

Sort order should be:

1. unresolved actionable items
2. unresolved intervention items
3. unread milestone updates
4. older read items

Within each group:

- newest first

### Resolution Rules

`Attention` items should not disappear immediately when seen.

Rules:

- approvals stay until approved or denied
- input requests stay until answered
- failures/stalls stay until the run is resumed, fixed, or dismissed
- completions stay as read history until archived by age or user action

### Badge Rules

The badge should count only unresolved actionable items.

That means:

- `approval_required`
- `input_required`
- unresolved failure/stall/intervention items

Do not include normal completions in the badge.

### Thread Linking

`Attention` items should be able to jump back into the `Mako` thread.

Target behavior:

- open the main `Mako` thread
- scroll to the event anchor
- briefly highlight the message or event location

First implementation can degrade gracefully to:

- open the `Mako` thread
- preserve the thread link contract for later exact anchors

### Visual Model

`Attention` should feel like a clean notification inbox.

Each item should be:

- compact
- readable in one glance
- one-line summary first
- one primary action

Avoid:

- large cards
- stacked diagnostics
- secondary metrics
- long detail text

### Mako Thread vs Attention Feed

Examples:

#### Approval

In `Mako`:

`I need approval to restart the daemon on krusty.`

In `Attention`:

`Approval required`
`Restart daemon on krusty`
`Approve`

#### Completion

In `Mako`:

`I finished the auth cleanup pass and wrote a report.`

In `Attention`:

`Run completed`
`Auth cleanup pass finished`
`Open report`

#### Scheduled start

In `Mako`:

`The nightly release verification run just started.`

In `Attention`:

`Scheduled run started`
`Nightly release verification`
`Open run`

### Backend Contract Direction

The current backend already exposes raw ingredients:

- approvals
- runs
- diagnostics
- next wake
- per-session events
- completion and approval notifications

But `Attention` should become a first-class feed contract instead of only derived UI state.

Recommended next backend shape:

- `GET /api/mako/attention`
- optional filters:
  - `kind`
  - `resolved`
  - `unread`

Item actions:

- `POST /api/mako/attention/:id/read`
- `POST /api/mako/attention/:id/resolve`

The first implementation can still derive from existing data if needed, but the target should be a dedicated feed.

### Success Criteria

`Attention` is correct when:

- it feels like a useful inbox
- it does not duplicate the whole Mako thread
- it makes approvals and completions easy to reopen
- it gives durable access to important events
- the badge means something precise
