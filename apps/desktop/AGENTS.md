# AGENTS Guide: /apps/desktop

## Purpose
Desktop delivery layer for Krusty.

## Guardrails
- Desktop shell is a host for the Expo web build, not a separate product surface.
- Keep desktop-specific code focused on windowing, permissions, startup wiring, and packaging.
- Avoid introducing runtime behavior that diverges from shared server/mobile-web contracts.

## Validation
- `cd apps/desktop/shell && cargo check --manifest-path src-tauri/Cargo.toml`
