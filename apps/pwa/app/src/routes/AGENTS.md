# AGENTS Guide: /apps/pwa/app/src/routes

## Purpose
Route entrypoints and top-level page composition.

## Guardrails
- Keep route files thin; delegate reusable logic to `src/lib`.
- Preserve startup wiring in `+layout.svelte` (service worker, push reconcile, session routing).
- Keep navigation behavior deterministic across app tabs.
- Standalone/mobile route shells must account for safe-area insets so headers and controls remain on-screen.

## Validation
- `cd apps/pwa/app && bun run check`
