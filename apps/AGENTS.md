# AGENTS Guide: /apps

## Purpose
All user-facing application surfaces.

## Guardrails
- Preserve strict separation between app surfaces and core runtime internals.
- Do not duplicate business logic that already exists in `krusty-core` or `krusty-server`.
- Keep desktop and Expo web behavior aligned where features overlap.
- Keep model-speed and reasoning controls driven by shared client state or server contracts rather than ad-hoc component-local mappings.
- Notification and Live Activity actions that mutate session state must carry explicit session context; never assume the currently focused chat is the correct target.

## Directory Notes
- `desktop/`: Tauri shell for desktop distribution.
- `mobile/`: Expo mobile app plus the primary React web client.
- `marketing/`: static pages only.
