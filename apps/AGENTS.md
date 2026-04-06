# AGENTS Guide: /apps

## Purpose
All user-facing application surfaces.

## Guardrails
- Preserve strict separation between app surfaces and core runtime internals.
- Do not duplicate business logic that already exists in `krusty-core` or `krusty-server`.
- Keep desktop and Expo web behavior aligned where features overlap.

## Directory Notes
- `desktop/`: Tauri shell for desktop distribution.
- `mobile/`: Expo mobile app plus the primary React web client.
- `marketing/`: static pages only.
