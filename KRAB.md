# krusty-public

## Tech Stack

- Rust

## Architecture

krusty: terminal-based AI coding CLI/TUI entrypoint with ACP server mode (crates/krusty-cli/src/main.rs)
krusty-core: core library for AI, storage, tools, extensions (crates/krusty-core/src/lib.rs)
krusty-core::ai: AI provider layer with multi-provider clients/streaming (crates/krusty-core/src/ai/mod.rs)
krusty-core::agent: agent system with event handling, hooks, sub-agents, pinch context (crates/krusty-core/src/agent/mod.rs)
krusty-core::acp: Agent Client Protocol server for editor integration (crates/krusty-core/src/acp/mod.rs)
krusty-core::mcp: Model Context Protocol client manager (crates/krusty-core/src/mcp/mod.rs)
krusty-core::tools: tool registry and built-in tool implementations (crates/krusty-core/src/tools/mod.rs)
krusty-core::storage: SQLite persistence for sessions, plans, preferences, credentials (crates/krusty-core/src/storage/mod.rs)
krusty-core::plan: database-backed planning system (crates/krusty-core/src/plan/mod.rs)
krusty-core::skills: filesystem-based skills system (crates/krusty-core/src/skills/mod.rs)
krusty-core::extensions: Zed-compatible WASM extension system (crates/krusty-core/src/extensions/mod.rs)
krusty-core::process: background process registry/management (crates/krusty-core/src/process/mod.rs)
krusty-core::auth: OAuth/auth flows and token storage helpers (crates/krusty-core/src/auth/mod.rs)
krusty-core::updater: auto-updater for dev/release modes (crates/krusty-core/src/updater/mod.rs)
krusty-cli::tui: terminal UI module (crates/krusty-cli/src/tui/mod.rs)
krusty-cli::tui::blocks: modular stream blocks for rendering chat/tool output (crates/krusty-cli/src/tui/blocks/mod.rs)
krusty-cli::tui::handlers: event handlers (crates/krusty-cli/src/tui/handlers/mod.rs)
krusty-cli::tui::state: centralized TUI state management (crates/krusty-cli/src/tui/state/mod.rs)
krusty-cli::tui::themes: theme system and registry (crates/krusty-cli/src/tui/themes/mod.rs)
krusty-cli::tui::plugins: trait-based plugin system (crates/krusty-cli/src/tui/plugins/mod.rs)

Design patterns:
Event Bus: AgentEventBus as central dispatcher (crates/krusty-core/src/agent/mod.rs)
Registry: ToolRegistry (crates/krusty-core/src/tools/mod.rs) and ThemeRegistry/THEME_REGISTRY (crates/krusty-cli/src/tui/themes/mod.rs)
Plugin architecture (trait-based): Plugin trait for dynamic plugins (crates/krusty-cli/src/tui/plugins/mod.rs)
Strategy/Polymorphism via traits: StreamBlock trait for renderable blocks (crates/krusty-cli/src/tui/blocks/mod.rs)
Manager pattern: McpManager, PlanManager, SkillsManager, SessionManager (crates/krusty-core/src/mcp/mod.rs, plan/mod.rs, skills/mod.rs, storage/mod.rs)

## Key Files

crates/krusty-cli/src/main.rs - CLI entry point that parses commands and starts ACP server or TUI with logging/setup.  
crates/krusty-core/src/lib.rs - Core crate module registry and re-exports for AI, tools, storage, MCP, etc.  
crates/krusty-cli/src/tui/app.rs - Main TUI application state and event loop with UI/runtime structures.  
crates/krusty-core/src/ai/mod.rs - AI provider layer module index for clients, formats, streaming, and providers.  
crates/krusty-core/src/ai/client/mod.rs - Provider-agnostic AI client module definitions and re-exports.  
crates/krusty-core/src/tools/mod.rs - Tool system module index and re-exports for registry and implementations.  
crates/krusty-core/src/tools/registry.rs - Tool registry and execution context with hooks, timeouts, and helpers.  
crates/krusty-core/src/storage/mod.rs - SQLite persistence layer exposing sessions, preferences, plans, credentials.  
crates/krusty-core/src/acp/server.rs - ACP server entry point for JSON-RPC stdio agent mode.  
crates/krusty-core/src/agent/mod.rs - Agent system module index for hooks, events, subagents, summarizer.

## Conventions

Error handling: anyhow + thiserror + custom error enum.
- anyhow::Result / anyhow::anyhow!: crates/krusty-cli/src/main.rs:8,50; crates/krusty-core/src/acp/server.rs:10,137; crates/krusty-core/src/process/mod.rs:11; crates/krusty-core/src/updater/checker.rs:3,99
- thiserror::Error derive on custom enum: crates/krusty-core/src/acp/error.rs:3-48
- custom error type AcpError: crates/krusty-core/src/acp/error.rs:6-86

Logging: tracing + tracing_subscriber; some eprintln!/println! (mostly error fallback/docs).
- tracing_subscriber setup + tracing::info!: crates/krusty-cli/src/main.rs:93-116
- tracing::{debug, error, info, warn}: crates/krusty-core/src/acp/agent.rs:20; crates/krusty-core/src/acp/server.rs:15
- eprintln! fallback: crates/krusty-cli/src/main.rs:62-84
- println! in docs/tests: crates/krusty-core/src/skills/mod.rs:35; crates/krusty-core/src/skills/manager.rs:111,131

Async: tokio.
- #[tokio::main] async fn main(): crates/krusty-cli/src/main.rs:50
- tokio::spawn / tokio::sync / tokio::process: crates/krusty-core/src/process/mod.rs:11-13,172; crates/krusty-core/src/acp/server.rs:11-13,116-126
- no async-std matches in Rust code.

Testing: inline unit tests in src with #[cfg(test)] mod tests; uses built-in #[test] and #[tokio::test] (tokio test runtime).
- #[cfg(test)] mod tests + #[tokio::test]: crates/krusty-core/src/acp/agent.rs:793-809
- #[cfg(test)] mod tests + #[tokio::test]: crates/krusty-core/src/mcp/config.rs:279-333
- #[cfg(test)] mod tests + #[test]: crates/krusty-cli/src/tui/plugins/kitty_graphics.rs:354-440
- #[cfg(test)] mod tests + #[test]: crates/krusty-core/src/storage/sessions.rs:468-520

Naming: Rust conventions observed.
- Modules/files snake_case: crates/krusty-core/src/acp/model_manager.rs; crates/krusty-cli/src/tui/handlers/commands.rs
- Structs/Enums CamelCase: KrustyAgent, ModelConfig, ProcessRegistry, ProcessStatus (crates/krusty-core/src/acp/agent.rs; crates/krusty-core/src/process/mod.rs)
- Constants SCREAMING_SNAKE_CASE: DEFAULT_USER (crates/krusty-core/src/process/mod.rs:78)
- Functions snake_case: detect_available_models, set_model, spawn_for_user (crates/krusty-core/src/acp/agent.rs; crates/krusty-core/src/process/mod.rs)

Key files examined:
- crates/krusty-cli/src/main.rs
- crates/krusty-core/src/lib.rs
- crates/krusty-core/src/acp/error.rs
- crates/krusty-core/src/acp/agent.rs
- crates/krusty-core/src/acp/server.rs
- crates/krusty-core/src/process/mod.rs
- crates/krusty-core/src/storage/sessions.rs
- crates/krusty-core/src/mcp/config.rs
- crates/krusty-core/src/acp/model_manager.rs
- crates/krusty-cli/src/tui/plugins/kitty_graphics.rs
- crates/krusty-cli/src/tui/handlers/commands.rs

## Build & Run

```bash
curl -fsSL https://raw.githubusercontent.com/BurgessTG/Krusty/main/install.sh | sh  # quick install (README.md)
brew tap BurgessTG/tap  # homebrew tap (README.md)
brew install krusty  # homebrew install (README.md)
git clone https://github.com/BurgessTG/Krusty.git  # clone repo (README.md)
cd Krusty  # enter repo (README.md)
cargo build --release  # release build (README.md, KRAB.md)
./target/release/krusty  # run built binary (README.md)
cargo build --workspace  # workspace build (CLAUDE.md)
cargo test --workspace  # workspace tests (CLAUDE.md)
cargo fmt --all  # format (CLAUDE.md)
cargo clippy --workspace -- -D warnings  # lints (CLAUDE.md)
cargo build  # debug build (KRAB.md)
cargo check  # type check (KRAB.md)
cargo test  # run tests (KRAB.md)
cargo clippy  # run lints (KRAB.md)
cargo clippy -- -D warnings  # lints as errors (KRAB.md)
cargo fmt  # format code (KRAB.md)
cargo tree -p krusty -i  # dependency tree (KRAB.md)
cargo update -p <package>  # update dependency (KRAB.md)
krusty acp  # ACP mode (CLAUDE.md)
```

Key dependencies:
krusty-core
tokio
anyhow
thiserror
serde
serde_json
serde_yaml
toml
tracing
tracing-subscriber
async-trait
futures
tokio-stream
tokio-util
reqwest
ratatui
ratatui-image
crossterm
unicode-width
textwrap
arboard
image
palette
vt100
portable-pty
rusqlite
wasmtime
wasmtime-wasi
wasmparser
semver
sha2
hmac
base64
rand
dirs
url
webbrowser
open
tiny_http
html2md
scraper
which
chrono
uuid
regex
once_cell
clap
glob
walkdir
similar
shell-words
scopeguard
bytes
flate2
tar
zip
xz2
dashmap
libc
libloading
parking_lot
moka
ignore
git2
pulldown-cmark
syntect
gilrs
agent-client-protocol
tempfile

## Notes for AI

<!-- Add project-specific instructions here -->

