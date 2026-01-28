# Krusty

Terminal-based AI coding assistant with multi-provider AI support and Zed's WASM extension system.

## Tech Stack

- Rust

## Architecture

---

## CRATES

**krusty-core**: Shared library containing AI, storage, tools, LSP, extensions, and agent systems

**krusty-cli**: Terminal UI application with ratatui-based chat interface, Zed extension management, and theming system

---

## KEY MODULES IN KRUSTY-CORE

- **agent**: Event bus, state tracking, hooks system, sub-agents, build context coordination (Octopod swarm)
- **ai**: Multi-provider AI client with streaming, SSE parsing, title generation, OpenRouter support
- **auth**: (deprecated - API keys now stored in storage/credentials)
- **extensions**: Zed-compatible WASM extension host (wasmtime), manifest parsing, GitHub integration
- **lsp**: Language server protocol client with JSON-RPC transport, diagnostics, and downloader for Zed extensions
- **tools**: Tool registry with pre/post-execution hooks, implementations (read, write, edit, bash, grep, glob, processes, explore, build, skills, ask_user)
- **skills**: Modular filesystem-based resources with YAML frontmatter for domain-specific instructions
- **storage**: SQLite persistence for sessions, plans, preferences, credentials, file activity tracking
- **plan**: Plan management with session linkage and task tracking
- **process**: Background process registry with status tracking
- **paths**: Configuration directory resolution

---

## DESIGN PATTERNS USED

1. **Trait-based Architecture** - Async traits (`#[async_trait]`) for extensibility: `PreToolHook`, `PostToolHook`, `Tool`, `WorktreeDelegate`, `SseParser`

2. **Hook Pattern** - Pre/post execution hooks for logging, validation, and safety interception on all tools

3. **Registry Pattern** - `ToolRegistry`, `SkillsManager`, theme/extension registries for plugin-like extensibility

4. **Event Bus Pattern** - `AgentEventBus` for centralized event dispatch and logging

5. **Builder Swarm (Octopod)** - `SharedBuildContext` coordinates concurrent Opus agents with file locks, conventions, and diff tracking

6. **Sub-agent Pool** - Lightweight parallel agents for codebase exploration with limited tool access

7. **Cache Pattern** - `SharedExploreCache` and moka caches for expensive operations

8. **Streaming/SSE** - Server-sent events parsing with accumulators for thinking, tool calls, and tokens

9. **Shared State Pattern** - Arc/RwLock/DashMap for thread-safe concurrent state in process registry, credentials, models

10. **Context Object Pattern** - `ToolContext` passed through tool execution with optional channels for streaming output, progress updates

11. **Configuration Loading** - Multi-source config (API keys, preferences) with fallbacks

12. **Middleware/Pipeline** - Axum tower-http for CORS, tracing, routing in server

13. **WASM Component Model** - Zed-compatible extension loading via wasmtime with component WIT interfaces

14. **Pinch Context Pattern** - Session continuation state for resuming conversations with summarization

15. **CLI Subcommand Pattern** - Clap derive for modular commands (chat, lsp, auth, themes)

## Key Files

- `crates/krusty-cli/src/main.rs` - CLI entry point with chat, theme, LSP, and authentication subcommands
- `crates/krusty-cli/src/tui/app.rs` - Terminal UI application logic and state management
- `crates/krusty-core/src/lib.rs` - Core library exposing agent, AI, auth, storage, tools, extensions modules
- `crates/krusty-core/src/tools/registry.rs` - Tool registry managing execution, hooks, timeouts, and context
- `crates/krusty-core/src/agent/mod.rs` - Agent system with event bus, state tracking, hooks, and sub-agents
- `crates/krusty-core/src/ai/client/core.rs` - AI API client with streaming and tool execution
- `crates/krusty-core/src/storage/database.rs` - SQLite database wrapper with versioned migrations and schema management
- `crates/krusty-core/src/extensions/mod.rs` - Zed-compatible WASM extension system for language servers
- `crates/krusty-core/src/lsp/manager.rs` - LSP manager coordinating multiple language servers and diagnostics
- `crates/krusty-core/src/skills/manager.rs` - Skills manager for discovery and loading of global and project skills
- `crates/krusty-core/src/storage/sessions.rs` - Session CRUD operations with database persistence
- `crates/krusty-core/src/plan/manager.rs` - Plan manager with SQLite-backed storage and session linkage

## Conventions

**Error Handling: anyhow**
- Primary: `anyhow::Result` and `anyhow::anyhow!()` for most functions
- Secondary: `thiserror` in dependencies (2.0) for structured errors

**Logging: tracing**
- Uses `tracing` crate (0.1) exclusively, no `log` or `println`
- Levels: `tracing::info!()`, `tracing::warn!()`, `tracing::debug!()`
- Setup: `tracing_subscriber` with env-filter in main.rs
- Exception: User-facing CLI output uses `println!()` and `eprintln!()` (not logging)

**Async: tokio**
- Runtime: `tokio` 1.40 with "full" features across all crates
- Entry: `#[tokio::main]` attribute on main functions
- Traits: `#[async_trait]` from async-trait crate for trait methods

**Testing: Inline with #[cfg(test)]**
- Location: `#[cfg(test)] mod tests { }` blocks at end of source files
- Framework: Standard Rust test framework (`#[test]` attribute)
- Dependencies: `tempfile` in dev-dependencies for temporary directory tests

**Naming Conventions:**
- Module naming: lowercase snake_case (`tools`, `extensions`, `lsp`)
- Struct naming: PascalCase (`SkillsManager`, `PlanManager`, `ToolRegistry`, `AiClient`)
- Function naming: snake_case (`parse_frontmatter`, `build_tree`, `create_session`)
- Enum naming: PascalCase variants (`SkillSource::Global`)
- Tool implementation files: Named after tool function (bash.rs, read.rs, write.rs, glob.rs, grep.rs, explore.rs, edit.rs)

## Build & Run

```bash
cargo check                          # Quick compilation check
cargo build                          # Debug build
cargo build --release                # Release build (LTO enabled)
cargo run                            # Run TUI
cargo run -- lsp list                # List installed extensions
cargo run -- lsp install <name>      # Install Zed extension
cargo run -- lsp remove <name>       # Remove extension
cargo run -- lsp status              # Show extension details
cargo run -- themes                  # List available themes
cargo run -- -t <theme>              # Run with specific theme
cargo clippy                         # Lint check (zero warnings required)
cargo test                           # Run tests
```

## Key Dependencies

- tokio (async runtime)
- reqwest (HTTP client)
- ratatui (TUI framework)
- crossterm (terminal control)
- portable-pty (PTY/terminal emulation)
- vt100 (terminal parsing)
- rusqlite (SQLite database)
- wasmtime, wasmtime-wasi (WASM runtime for extensions)
- lsp-types (LSP support)
- git2 (version control)
- image (image processing)
- serde, serde_json, toml, serde_yaml (serialization)
- tracing (logging)
- anyhow, thiserror (error handling)
- clap (CLI argument parsing)
- uuid, chrono, regex, glob, walkdir (utilities)
- flate2, tar, zip, xz2 (archive handling)
- dashmap, parking_lot, moka (concurrency)
- ignore (file system)
- syntect (syntax highlighting)

