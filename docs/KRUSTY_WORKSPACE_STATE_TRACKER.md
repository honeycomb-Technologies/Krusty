# Krusty Workspace State Tracker

## Program
Workspace State Roadmap

## Objective
Make neutral mode, explicit project mode, and promoted project creation behave deterministically across core, server, and UI surfaces.

## Current Baseline
- Canonical session contract now persists `project_dir` + `workspace_mode`
- Neutral `/api/chat` sessions persist `working_dir = null`, `project_dir = null`, `workspace_mode = neutral`
- Project instructions and project-local skills only activate from explicit project context
- Remote PWA boot, reload, and workspace-state visibility are stable

## Remaining Program Status

| Phase | Name | Status | Notes |
| --- | --- | --- | --- |
| 1 | Canonical Workspace Contract | Complete | Sessions now persist `project_dir` and `workspace_mode`; migration 20 upgrades existing rows. |
| 2 | Prompt and Context Discipline | Complete | Neutral sessions inject explicit neutral guidance and no longer load project instructions/skills. |
| 3 | Neutral Execution Policy | Complete | Neutral execution now prefers user home when available, then safe server root, without promoting project identity. |
| 4 | Promotion Rules | Complete | Workspace promotion is explicit through canonical session updates and the `set_workspace_context` tool. |
| 5 | New Project Creation Flow | Complete | Sessions can move from `neutral` to `created` with persisted project context during live work. |
| 6 | System and Config Workflows | Complete | Neutral system/config work remains prompt-neutral and unpromoted unless project context is explicitly set. |
| 7 | Tool Governance by Workspace Mode | Complete | Repo-oriented `build`/`explore` now refuse neutral sessions cleanly instead of assuming repo context. |
| 8 | Surface UX and Transparency | Complete | PWA workspace store, sidebar, and header now surface neutral vs project state directly. |
| 9 | API and Persistence Finalization | Complete | Session/chat APIs expose `project_dir` and `workspace_mode` canonically while retaining `working_dir` compatibility. |
| 10 | Validation and Competitive Audit | Complete | Full validation gate passed; live server probes verified neutral creation, project creation, promotion, and reload semantics. |

## Completed Deliveries
- Added schema migration 20 for `project_dir` and `workspace_mode`
- Split prompt/project identity from execution cwd across server, core, and PWA
- Added explicit neutral-mode system guidance
- Added `set_workspace_context` tool for explicit session promotion
- Made `build` and `explore` require explicit project context
- Surfaced workspace mode in PWA session/sidebar/header state
- Preserved compatibility by keeping `working_dir` as execution metadata while moving semantics to canonical fields

## Validation Evidence
- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cd apps/pwa/app && bun run check`
- `cd apps/pwa/app && bun run build`

## Live Backcheck Evidence
- `POST /api/sessions` with `workspace_mode=neutral` returns `working_dir=null`, `project_dir=null`
- `POST /api/sessions` with explicit project returns `working_dir=/path`, `project_dir=/path`, `workspace_mode=selected`
- `POST /api/chat` auto-create now returns a session that stays `workspace_mode=neutral`
- `PATCH /api/sessions/:id` promoting neutral -> created persists and survives fresh session listing/reload
- PWA build/check passed with workspace labels and neutral/project wiring intact
