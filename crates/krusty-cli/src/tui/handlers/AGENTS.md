# AGENTS Guide: /crates/krusty-cli/src/tui/handlers

## Purpose
TUI event handlers and stream processing.

## Guardrails
- Keep keyboard/mouse/render handling deterministic.
- Handle partial stream events safely; never panic on malformed chunks.
- Drain bursty stream output incrementally; do not let a single stream monopolize a frame and starve input/render.
- Keep session/tool side effects explicit and traceable.
- Keep model selection and quick-toggle flows on a shared handler path so persistence, auth rebinds, and recent-model state do not drift.
- Plan/task UI state must come from persisted plan lifecycle or explicit loop events, not heuristic parsing of assistant prose.

## Validation
- `cargo check -p krusty`
