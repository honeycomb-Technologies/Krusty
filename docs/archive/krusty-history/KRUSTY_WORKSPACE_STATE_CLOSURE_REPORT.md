# Krusty Workspace State Closure Report

> Archived historical planning/audit document.
>
> This file is preserved for project history. It may reference the former `apps/pwa/app`, Svelte-era client files, or old validation commands when describing historical implementation state.


## Outcome
The workspace-state roadmap is complete.

Krusty now has a first-class neutral workspace model instead of inferring project identity from server launch cwd, fallback execution cwd, or hidden client heuristics.

## What Changed
- Sessions now persist `project_dir` and `workspace_mode` in addition to compatibility `working_dir`.
- Neutral sessions inject explicit system guidance that no project is currently selected.
- Project instructions and project-local skills only activate when `project_dir` exists.
- Neutral execution uses a canonical fallback policy that prefers user home when available, then server root, without treating that cwd as project meaning.
- Session promotion is explicit and persisted through canonical session updates and the new `set_workspace_context` tool.
- Repo-oriented tools now fail clearly in neutral mode instead of pretending a repo exists.
- PWA state, session/sidebar rendering, and session reload now consume and display canonical workspace semantics.

## Runtime Verification
Live probes against a fresh server instance confirmed:
- neutral session creation through `/api/sessions`
- neutral auto-create through `/api/chat`
- explicit project session creation through `/api/sessions`
- promotion of a neutral session into `workspace_mode=created`
- persistence and fresh reload/list visibility of the promoted state

## Validation
- `cargo fmt --all --check`
- `cargo test --workspace`
- `cargo clippy --workspace -- -D warnings`
- `cd apps/pwa/app && bun run check`
- `cd apps/pwa/app && bun run build`

## Result
Krusty now behaves correctly in the two modes that mattered most for this program:
- neutral shell/brainstorm/system work
- explicit or promoted project work

Project-aware behavior is now deliberate, not accidental.
