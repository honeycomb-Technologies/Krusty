# AGENTS Guide: /apps/desktop/shell

## Purpose
Tauri wrapper project around the Expo web build.

## Guardrails
- Keep JS package scripts and Tauri config in sync.
- Do not embed business logic in shell bootstrap code.
- Changes here must preserve app startup reliability and update behavior.

## Validation
- `cd apps/desktop/shell && bun install`
- `cd apps/desktop/shell && cargo check --manifest-path src-tauri/Cargo.toml`
