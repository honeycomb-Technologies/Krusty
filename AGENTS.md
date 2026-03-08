# AGENTS Guide: Krusty

## Purpose
Repository-level engineering guardrails for Krusty - an AI coding assistant CLI/TUI with ACP server mode.

## AGENTS Strategy
- Keep `AGENTS.md` only at architectural boundaries where local rules differ.
- Do not create AGENTS in every leaf folder.
- Add a new AGENTS file only when a directory has unique invariants, workflows, or integration risk.
- When a notable structural or behavioral change lands, update the nearest applicable AGENTS file in the same commit.

## Core Architecture
- `crates/krusty-cli`: Terminal client and TUI runtime. Entry point with command parsing.
  - `src/main.rs`: CLI entry point, parses commands, starts ACP server or TUI with logging/setup.
  - `src/tui/`: Terminal UI module with blocks, handlers, state, themes, plugins.
- `crates/krusty-core`: Shared runtime library.
  - `src/ai/`: AI provider layer with multi-provider clients, streaming support.
  - `src/agent/`: Agent system with event handling, hooks, sub-agents, pinch context.
  - `src/acp/`: Agent Client Protocol server for editor integration.
  - `src/mcp/`: Model Context Protocol client manager.
  - `src/tools/`: Tool registry and built-in tool implementations (read, write, edit, bash, grep, glob, etc.).
  - `src/storage/`: SQLite persistence for sessions, plans, preferences, credentials.
  - `src/plan/`: Database-backed planning system.
  - `src/skills/`: Filesystem-based skills system.
  - `src/extensions/`: Zed-compatible WASM extension system.
  - `src/process/`: Background process registry/management.
  - `src/auth/`: OAuth/auth flows and token storage helpers.
  - `src/updater/`: Auto-updater for dev/release modes.
- `crates/krusty-server`: Self-host API for external clients.
- `apps/pwa/app`: Primary installable web client (SvelteKit).
- `apps/desktop/shell`: Tauri wrapper around the PWA.
- `apps/marketing/site`: Static marketing/legal pages only.

## Design Patterns
- **Event Bus**: AgentEventBus as central dispatcher.
- **Registry**: ToolRegistry, ThemeRegistry for centralized management.
- **Plugin Architecture**: Trait-based plugins (e.g., StreamBlock trait for renderable blocks).
- **Strategy/Polymorphism via traits**: Different providers, tool implementations.
- **Manager Pattern**: McpManager, PlanManager, SkillsManager, SessionManager.

## Cross-Cutting Standards
- Prefer clear module boundaries over cross-layer coupling.
- Write code that is composable, testable, and explicit about failure modes.
- Keep changes small and reversible.
- Avoid hidden side effects and global state sprawl.
- Error handling: anyhow + thiserror + custom error enum.
- Logging: tracing + tracing_subscriber.
- Async: tokio (no async-std).

## Default Dev Workflow
- Build and run current local code only; do not require `git pull` for day-to-day refinement.
- **Rust backend**: `cargo run -p krusty` from repo root.
- **PWA dev server**: `cd apps/pwa/app && bun run dev` (default `http://localhost:5173`).
- Do active UI/PWA iteration at `http://localhost:5173` so HMR is enabled, with `/api` and `/ws` proxied to backend.
- Frontend edits hot-reload automatically; Rust backend edits require a restart.
- **ACP mode** (editor integration): `krusty acp`

## Required Validation
All code must pass before commit:
```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
```
PWA validation:
```bash
cd apps/pwa/app && bun run check && bun run build
```

## Dependencies
Key runtime dependencies (check `Cargo.toml` for versions):
- tokio (async runtime)
- anyhow, thiserror (error handling)
- serde, serde_json, serde_yaml, toml (serialization)
- tracing, tracing-subscriber (logging)
- ratatui (TUI framework)
- rusqlite (database)
- reqwest (HTTP client)
- wasmtime (WASM extensions)
