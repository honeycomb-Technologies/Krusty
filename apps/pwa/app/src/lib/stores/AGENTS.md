# AGENTS Guide: /apps/pwa/app/src/lib/stores

## Purpose
Centralized client state management.

## Guardrails
- Store modules own shared state transitions and persistence strategy.
- Avoid cyclical store dependencies.
- Keep state shape stable and migration-safe for persisted keys.
- Session-affecting preferences (mode/model) must be propagated to backend calls and persisted consistently.

## Validation
- `cd apps/pwa/app && bun run check`
