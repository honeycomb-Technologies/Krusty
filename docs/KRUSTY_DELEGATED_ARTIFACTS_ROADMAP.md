# Krusty Delegated Artifacts Roadmap

## Objective
Make delegated `explore` and `build` first-class across `krusty-core`, `krusty-server`, and the PWA so web/mobile surfaces have semantic parity with the TUI instead of relying on generic tool widgets.

## Design Principles
- `krusty-core` remains the source of truth for delegated execution semantics.
- `krusty-server` transports typed delegated progress and result state; it does not reconstruct state from text.
- The PWA renders dedicated delegated artifacts, not guessed output formats.
- Repeated top-level delegated calls are distinct from child agents and must be legible as such.
- Reload, recovery, and trace surfaces must preserve delegated context.

## Current Baseline
- Core `explore` and `build` already emit typed `AgentProgress` updates on dedicated channels.
- TUI already consumes those progress streams and renders dedicated delegated blocks.
- Server SSE currently exposes only generic tool lifecycle events.
- PWA currently stores delegated tools as flat generic `ToolCall` records and tries to infer structure from `toolCall.output`.

## Phases

### Phase 1: Canonical Delegated Event Contract
Define first-class server event types for delegated progress and delegated completion metadata.

Deliverables:
- Add typed SSE event(s) for delegated progress.
- Include parent tool call id, delegated kind, task id, display name, status, current action, tool count, tokens, completion summary, and build-specific metrics.
- Keep the generic tool lifecycle events for compatibility, but stop depending on them for delegated clarity.

Exit Gate:
- Server contract can represent delegated runtime state without text parsing.

### Phase 2: Server Transport Bridging
Bridge existing core progress channels into the server SSE stream.

Deliverables:
- Forward `explore_progress_tx` / `build_progress_tx` updates through the canonical delegated event contract.
- Preserve parent tool call association.
- Ensure event delivery respects existing backpressure behavior.

Exit Gate:
- Live server streams emit typed delegated progress while explore/build are running.

### Phase 3: PWA Delegated State Model
Add a first-class delegated artifact model in the PWA session store.

Deliverables:
- Extend tool state with delegated artifact data keyed by parent tool call id.
- Track live agent rows, aggregate stats, summaries, files examined, errors, and policy notes.
- Distinguish repeated top-level delegated calls from child-agent rows.

Exit Gate:
- PWA can store and update delegated runtime state without flattening it into generic tool output.

### Phase 4: Dedicated PWA Rendering
Replace delegated-tool fallback rendering with first-class explore/build components.

Deliverables:
- Render live agent rows, counts, timings, summaries, files examined, and errors.
- Parse the real JSON result envelope returned by core instead of looking for markdown sections.
- Make repeated delegated runs legible as separate top-level artifacts.

Exit Gate:
- Completed and in-flight delegated runs are understandable at a glance on web/mobile.

### Phase 5: Persistence and Recovery Parity
Preserve delegated artifact meaning across reload and interruption.

Deliverables:
- Extend recovery/live-partial handling to retain delegated artifact snapshots where needed.
- Restore delegated state from persisted session state and final tool results.
- Keep interrupted delegated runs explicit instead of collapsing to generic pending tool calls.

Exit Gate:
- Reload during or after explore/build preserves understandable delegated state.

### Phase 6: Trace and Session API Parity
Expose delegated artifacts clearly in operator/debugging surfaces.

Deliverables:
- Ensure session state / trace APIs carry enough information to inspect delegated runs.
- Keep delegated chronology and final status diagnosable from server APIs.

Exit Gate:
- Delegated runs are inspectable through session trace/state surfaces, not only through the live PWA view.

### Phase 7: Validation and Closure
Run a full parity/backcheck pass and close the program.

Validation Matrix:
- Single-agent explore
- Multi-agent explore
- Repeated top-level explore calls in one assistant turn
- Multi-agent build
- Reload during delegated execution
- Reload after delegated completion
- Interrupted delegated run recovery
- Server trace/state inspection for delegated runs

Required Validation:
- `cargo fmt --all --check`
- `cargo check --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cd apps/pwa/app && bun run check`
- `cd apps/pwa/app && bun run build`

Exit Gate:
- Delegated explore/build are first-class across core, server, and PWA with no unresolved high-severity drift.
