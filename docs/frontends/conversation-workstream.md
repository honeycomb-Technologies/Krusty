# Mitsuro Conversation Workstream

Status: proposed product contract; implementation requires design approval.

## Objective

Make Mitsuro's mobile and terminal conversations feel like one capable coding
agent expressed through two native surfaces. The transcript must communicate
what the agent is doing, what changed, what needs attention, and what the final
answer is without reducing the stream to raw Markdown or a stack of generic
cards.

The surrounding mobile FAB and toolbox system is outside this contract.

## Reference findings

### Grok Build

The locally archived Grok Build 0.2.33 TUI uses a compact semantic activity
rail. Reasoning, commands, active work, and final prose remain visually related
without putting every event in a bordered panel. It also keeps context usage,
completed action counts, model, effort, and permission mode visible without
interrupting the transcript.

Patterns to retain:

- turn-oriented presentation;
- compact action rows with explicit state;
- a continuous visual rail for related activity;
- elapsed time and completion metadata;
- full-width final prose;
- progressive disclosure instead of raw tool payloads.

### Litter

Litter's native clients model transcript turns and presentation items before
rendering them. Its iOS implementation preserves rich detail for the current
and latest completed activity, groups exploration work, horizontally scrolls
code, and incrementally reparses only the unstable streaming tail.

Patterns to retain:

- stable turn and item identity;
- prefix/tail streaming render caches;
- native code, diff, approval, and widget surfaces;
- collapsed historical turns with useful previews;
- platform-native text selection and accessibility;
- rich detail retention bounded to recent activity.

### Mitsuro today

Mobile reconstructs visual segments independently from `ChatMessage` content
and tool arrays. The TUI reconstructs another presentation from string roles,
then overlays independently sized blocks on placeholder rows. These separate
derivations can drift in ordering, lifecycle, height, and recovery behavior.

The TUI's placeholder/overlay renderer also makes scroll correctness depend on
several calculations agreeing exactly. A stale height, width, clip boundary, or
cleared region can leave visible terminal artifacts.

## Product principles

1. User messages may be bubbles; assistant work is a timeline.
2. Prose is the primary surface. Chrome exists to explain work, not decorate it.
3. One semantic event has one stable identity and one lifecycle.
4. Active work is obvious without motion throughout the entire screen.
5. Completed routine actions collapse; failures and decisions remain visible.
6. Raw tool JSON is a diagnostic detail, never the default presentation.
7. Code and diffs are first-class selectable content, not Markdown accidents.
8. Streaming must not cause completed content to reflow, flash, or lose state.
9. Mobile and TUI share meaning and ordering, not pixel layout.
10. Every renderer paints or owns its complete visible region.

## Canonical presentation model

The shared model should be derived from canonical loop events and persisted
conversation state in the client-state boundary. It must not live in a React
component or Ratatui widget.

```text
ConversationTurn
  id
  source_turn_id
  state: queued | active | awaiting_input | complete | failed | interrupted
  started_at / completed_at
  usage_summary
  items: PresentationItem[]

PresentationItem
  UserMessage
  AssistantText
  Reasoning
  ToolActivity
  ExplorationGroup
  Code
  Diff
  Widget
  Approval
  Question
  Error
  Compaction
  Completion
```

Every item needs a stable ID, sequence, status, summary, optional detail model,
and accessibility label. Provider-specific stream events are normalized before
they reach this layer.

## Mobile grammar

### User input

- Keep the existing right-aligned blue user bubble.
- Attachments belong to the same turn boundary.
- Queued or unsent state is visible without changing the message color enough
  to reduce contrast.

### Assistant prose

- Render full width with no assistant bubble.
- Use the system UI font for prose and monospace for paths, commands, code, and
  compact activity metadata.
- Preserve ordinary Markdown hierarchy but avoid card styling for paragraphs,
  lists, and headings.
- A long press copies the semantic text of the item; explicit copy controls are
  present for code, commands, diffs, and error details.

### Work rail

- A thin leading rail visually joins reasoning and activity within a turn.
- Neutral work uses a subdued foreground rail.
- Active work uses the Mitsuro thinking/accent color with restrained animation.
- Success, warning, approval, and failure colors are status indicators, not
  large background fills.
- Final prose may continue after the rail rather than remaining boxed inside it.

### Reasoning

- Active: `Thinking...` with one localized animation.
- Complete: `Thought for 0.9s`, collapsed by default.
- Expansion shows the retained reasoning summary or content when policy allows.
- Reasoning never competes visually with the final answer.

### Tools

- Default row: semantic icon, action verb, primary target, duration, state, and
  disclosure affordance.
- Running tools expand enough to show live progress.
- The latest completed meaningful tool may retain detail.
- Older routine tools collapse to rows.
- Read/search/list bursts merge into an exploration group.
- Tool errors expand automatically and expose a recovery action when one exists.
- Approval, question, and plan-confirm tools remain interactive first-class
  items rather than ordinary tool rows.

### Code

- Theme-safe surface in light and dark modes.
- Language and Copy/Copied controls in a compact header.
- Native text selection.
- Horizontal scrolling with a visible indicator.
- No nested vertical scrolling in the transcript; long blocks use an explicit
  expand/collapse affordance.
- Diffs use a dedicated diff renderer and never inherit generic code colors.

### Widgets

- Widgets are inline instruments associated with the assistant item that
  produced them.
- They may contain internal controls and cards when those structures convey
  information, but the outer transcript should not add another generic card.
- Widget loading, ready, stale, and failed states are explicit.

### Turn completion and history

- Completion metadata is a compact footer: outcome, elapsed time, tool count,
  and optional usage.
- Keep the newest three turns expanded by default.
- Collapsed historical turns show user intent, final outcome, duration, tool
  count, errors, and widget count without reparsing their full content.

## TUI grammar and renderer

The TUI consumes the same ordered presentation items but builds a terminal
display list:

```text
DisplayRow {
  stable_id,
  item_id,
  row_within_item,
  cells,
  hit_target,
  copy_range
}
```

The display list is the only source of truth for total height, scroll limits,
hit testing, selection, and rendering. Custom widgets contribute rows through
the same layout contract instead of being painted over placeholder paragraphs.

Renderer requirements:

- calculate layout once per width and content revision;
- virtualize by row range after layout;
- fully paint every visible row, including background and cleared remainder;
- use stable row IDs so streaming updates replace only the unstable tail;
- clamp scroll against the same display list used for drawing;
- preserve the user's viewport while scrolled away from the bottom;
- follow the tail only while explicit auto-follow is active;
- keep animation invalidation localized to active rows;
- make selection, mouse hit testing, copying, and hyperlinks consume display-list
  coordinates rather than independently reconstructed offsets.

## Streaming invariants

1. Completed presentation items are immutable.
2. Only the active item's unstable tail may be reparsed for text deltas.
3. Tool lifecycle updates mutate the item with the same tool-call ID.
4. Reconnect/backfill deduplicates by durable event sequence and stable item ID.
5. A terminal event ends visible activity even if the transport closes late.
6. Clean EOF without a terminal event becomes a visible recovery error.
7. Expansion, copy feedback, and scroll position survive unrelated deltas.
8. The user can scroll away from active output without being pulled to bottom.

## Implementation slices

### Slice 0: readability repair

- override all Markdown library light defaults;
- provide native code selection, copy feedback, and horizontal scrolling;
- verify blockquotes, inline code, fenced code, and indented code in both themes.

### Slice 1: shared contract

- define typed presentation items in the shared client boundary;
- derive ordering and lifecycle once from canonical events;
- add fixtures for interrupted, resumed, compacted, approved, failed, and
  delegated turns;
- keep legacy clients functional during migration.

### Slice 2: mobile Workstream

- introduce the turn timeline and work rail;
- migrate reasoning and ordinary tools;
- migrate code, diffs, approvals, questions, errors, and widgets;
- add historical turn previews and stable-tail streaming.

### Slice 3: deterministic TUI

- introduce the display-list layout contract;
- migrate prose and basic activity rows;
- migrate interactive blocks and terminal panes;
- remove placeholder overlay rendering after parity is proven.

### Slice 4: hardening and release

- performance profiling on long and rapidly streaming sessions;
- mobile interaction and accessibility QA;
- TUI scroll/resize/expand stress runs;
- Honey deployment and iOS TestFlight verification.

## Acceptance gates

### Mobile

- fenced and indented code remain readable in light and dark themes;
- code selects, copies, and scrolls horizontally on iPhone;
- an active turn can stream prose, reasoning, tools, and widgets without losing
  row identity or expansion state;
- scrolling up disables tail follow until the user returns to latest;
- errors and approvals remain visible and actionable after reconnect;
- a 500-item transcript remains responsive and visually stable;
- VoiceOver announces item type, state, action, and disclosure controls;
- Dynamic Type does not clip controls or make content unreachable.

### TUI

- buffer snapshots are stable for each supported width and scroll offset;
- scrolling one row at a time across every block boundary leaves no stale cell;
- resize, sidebar animation, expand/collapse, and streaming can be interleaved
  without height or hit-test drift;
- selection and copied text match the visible rows;
- the viewport does not jump when historical rows are unchanged;
- bursty streams remain responsive and do not monopolize a frame.

### Shared behavior

- mobile and TUI fixtures produce the same semantic item ordering and lifecycle;
- persisted/replayed turns match live turns;
- provider differences do not leak into presentation components;
- no raw tool envelope is shown by default.

## Design decisions requiring approval

The recommended defaults are:

- system font for prose, monospace for technical metadata;
- user bubble plus full-width assistant timeline;
- compact reasoning by default;
- three recent turns expanded;
- active and latest-completed tools retain rich detail;
- Mitsuro accent color for active work, semantic colors only for outcomes.
