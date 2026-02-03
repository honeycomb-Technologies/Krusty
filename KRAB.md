# krusty-public

## Tech Stack

- Rust

## Key Files

crates/krusty-cli/src/main.rs - CLI entrypoint that initializes logging, handles ACP mode vs TUI, and runs the app.
crates/krusty-cli/src/tui/app.rs - Main TUI application state, services wiring, and event loop definitions.
crates/krusty-core/src/lib.rs - Core crate module exports and re-exports for AI, storage, tools, indexing, and ACP.
crates/krusty-core/src/agent/mod.rs - Agent system module wiring for state, events, hooks, dual-mind, and sub-agents.
crates/krusty-core/src/ai/client/core.rs - Core AI client implementation with system prompt and request building.
crates/krusty-core/src/acp/processor.rs - ACP prompt processing loop connecting AI client, tools, and streaming updates.
crates/krusty-core/src/index/mod.rs - Codebase indexing subsystem modules and public API exports.
crates/krusty-core/src/storage/database.rs - SQLite database wrapper with migrations and shared connection handling.
crates/krusty-core/src/tools/implementations/mod.rs - Registers built-in tool implementations and ACP-specific tool set.

## Conventions

Error handling: anyhow used (Cargo.toml deps in crates/krusty-core and crates/krusty-cli; `use anyhow::Result` in `crates/krusty-cli/src/main.rs:8`, `crates/krusty-core/src/ai/client/core.rs:6`, `crates/krusty-core/src/storage/database.rs:3`, `crates/krusty-core/src/ai/parsers/openai.rs:3`). thiserror dependency present (Cargo.toml in both crates) but no `derive(Error)`/`thiserror::Error` seen in sampled files.

Logging: tracing used (`use tracing::{error, info}` in `crates/krusty-core/src/ai/client/core.rs:8`, `tracing::info!` in `crates/krusty-cli/src/main.rs:88+`, `tracing::debug!/info!` in `crates/krusty-core/src/ai/parsers/openai.rs:25+`, `use tracing::info` + `tracing::warn!` in `crates/krusty-core/src/storage/database.rs:6/51`). tracing-subscriber setup in `crates/krusty-cli/src/main.rs:70-85`. println/eprintln used for early logging failures in `crates/krusty-cli/src/main.rs:58-76`.

Async: tokio used (`tokio` dependency in both Cargo.toml; `#[tokio::main]` in `crates/krusty-cli/src/main.rs:53`; async fns in core, e.g., `handle_error_response` in `crates/krusty-core/src/ai/client/core.rs:164` and async usage in TUI app via `tokio::sync::RwLock` in `crates/krusty-cli/src/tui/app.rs:18`).

Testing: dev-dependency `tempfile` in both Cargo.toml; no `#[test]`/`#[tokio::test]` found in scanned files (no explicit test locations observed).

Naming: Rust conventions (snake_case modules/functions, CamelCase structs/enums, SCREAMING_SNAKE constants). Examples: `pub struct AiClient` (`crates/krusty-core/src/ai/client/core.rs:68`), `pub enum WorkMode` (`crates/krusty-cli/src/tui/app.rs:74`), constant `SCHEMA_VERSION` (`crates/krusty-core/src/storage/database.rs:9`), modules like `ai`, `storage`, `tools` (`crates/krusty-core/src/lib.rs:10-23`).

## Build & Run

Build commands:
```bash
curl -fsSL https://raw.githubusercontent.com/BurgessTG/Krusty/main/install.sh | sh  # quick install (README.md)
brew tap BurgessTG/tap  # homebrew tap (README.md)
brew install krusty  # homebrew install (README.md)
git clone https://github.com/BurgessTG/Krusty.git  # clone (README.md)
cd Krusty  # enter repo (README.md)
cargo build --release  # release build (README.md)
./target/release/krusty  # run built binary (README.md)
cargo fmt --all  # format (CLAUDE.md)
cargo clippy --workspace -- -D warnings  # lints (CLAUDE.md)
cargo build --workspace  # workspace build (CLAUDE.md)
cargo test --workspace  # workspace tests (CLAUDE.md)
cargo build  # debug build (KRAB.md)
cargo build --release  # release build (KRAB.md)
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
- krusty-core
- tokio
- anyhow
- thiserror
- serde
- serde_json
- serde_yaml
- toml
- tracing
- tracing-subscriber
- async-trait
- futures
- tokio-stream
- tokio-util
- reqwest
- ratatui
- ratatui-image
- crossterm
- unicode-width
- textwrap
- arboard
- image
- palette
- vt100
- portable-pty
- rusqlite
- lsp-types
- wasmtime
- wasmtime-wasi
- wasmparser
- semver
- sha2
- hmac
- base64
- rand
- dirs
- url
- webbrowser
- open
- tiny_http
- html2md
- scraper
- which
- chrono
- uuid
- regex
- once_cell
- clap
- glob
- walkdir
- similar
- shell-words
- scopeguard
- bytes
- flate2
- tar
- zip
- xz2
- dashmap
- libc
- libloading
- parking_lot
- moka
- ignore
- git2
- pulldown-cmark
- syntect
- gilrs
- tree-sitter
- tree-sitter-rust
- streaming-iterator
- fastembed
- agent-client-protocol
- tempfile

## Notes for AI

<!-- Add project-specific instructions here -->

