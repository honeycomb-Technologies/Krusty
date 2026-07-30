<p align="center">
  <img src="assets/branding/mitsuro/mitsuro-lockup-horizontal.svg" alt="mitsuro" width="360">
</p>

## Overview

Mitsuro is a multi-platform AI coding product from Honeycomb Technologies. Agent is the interactive coding experience; Hive is the persistent autonomous mode that can keep working across threads and restarts. The existing `krusty` binary and `krusty-mako` service names are compatibility identifiers and remain stable for v1.

The repository contains the Rust runtime and CLI, an Expo client shared across mobile and web, a Tauri desktop host, and ACP editor integration. The terminal UI is still functional but is intentionally frozen at its legacy design while a ground-up Mitsuro TUI is developed.

## Repository Layout

```
crates/
  krusty-cli/     Terminal UI + CLI entry point (the main binary)
  krusty-core/    Shared AI, tools, storage, runtime (library)
  krusty-server/  API server with embedded web frontend (library)
apps/
  mobile/         Expo app — iOS, Android, and web (React Native)
  desktop/        Tauri desktop wrapper around the Expo web build
packages/
  api/            TypeScript API client (shared between mobile + desktop)
  state/          Zustand state management (shared between mobile + desktop)
  ui/             Design tokens and theme definitions (shared between mobile + desktop)
wit/              WebAssembly Interface Types for the extension system
```

## Quick Start

### Install

```bash
curl -fsSLO https://raw.githubusercontent.com/honeycomb-Technologies/Krusty/main/install.sh
sh install.sh
```

The installer requires the release archive checksum published alongside each GitHub release asset before it installs the binary.

Or from source:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libudev-dev
git clone https://github.com/honeycomb-Technologies/Mitsuro.git
cd Mitsuro
cargo build --release
./target/release/krusty
```

### Commands

| Command | Description |
|---------|-------------|
| `krusty` | Launch the interactive TUI |
| `krusty serve` | Start the web server with embedded web UI (default port 3000) |
| `krusty serve --port 8080` | Start on a custom port |
| `krusty acp` | Run as ACP server for editor integration |
| `krusty hive run "task"` | Start a persistent Hive run |
| `krusty hive status` | Show Hive run status |
| `krusty hive attach <id>` | Attach to a Hive run |

`krusty serve` bundles the Mitsuro API, Agent runtime, and embedded Expo web build into a single process. On first run it walks you through provider and API key setup. If Tailscale is installed, it can configure private HTTPS access.

If `krusty serve` reports Tailscale permission denied, run this once:

```bash
sudo tailscale set --operator=$USER
```

## Supported Providers

Configure providers via `/auth` in the TUI or on first run of `krusty serve`. Anthropic and OpenAI support OAuth browser login in addition to API keys.

| Provider | Models |
|----------|--------|
| **MiniMax** | MiniMax M2.5 |
| **Anthropic** | Claude Opus 4.6, Claude Haiku 4.5 |
| **OpenAI** | GPT-5.5, GPT-5.5 Mini, GPT-5.4, GPT-5.4 Mini, GPT-5.3 Codex |
| **OpenRouter** | 100+ models (Claude, GPT, Gemini, Llama, DeepSeek, Qwen) |
| **Z.ai** | GLM-5 |

Switch providers and models anytime with `/model`.

## TUI Controls

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | New line in input |
| `Esc` | Interrupt AI response / Close popup |
| `Ctrl+Q` | Quit application |
| `Ctrl+G` | Toggle BUILD/PLAN mode |
| `Ctrl+T` | Toggle plan sidebar |
| `Ctrl+B` | Open process list |
| `Ctrl+P` | Toggle plugin window |
| `Ctrl+F` | Toggle fuzzy/tree file search mode |
| `Tab` | Cycle thinking level (Off/Low/Medium/High/XHigh) |
| `@` | Search and attach files |
| `PgUp/PgDn` | Scroll messages |

### Slash Commands

| Command | Description |
|---------|-------------|
| `/home` | Return to start menu |
| `/load` | Load previous session (filtered by directory) |
| `/model` | Select AI model and provider |
| `/auth` | Manage API keys for providers |
| `/theme` | Change color theme |
| `/clear` | Clear current conversation |
| `/pinch` | Compress context to new session |
| `/plan` | View and manage active plan |
| `/mcp` | Manage MCP servers |
| `/skills` | Browse available skills |
| `/plugins` | Manage plugins |
| `/hooks` | Manage pre/post-tool hooks |
| `/permissions` | Switch between Supervised and Autonomous mode |
| `/ps` | View background processes |
| `/terminal` | Open interactive terminal (aliases: `/term`, `/shell`) |
| `/init` | Generate project context file |
| `/update` | Check for updates |
| `/cmd` | Show command help popup |

### Mouse

- Click and drag to select text
- Scroll wheel to navigate
- Click links to open in browser

## Features

### Multi-Provider AI
Configure multiple providers and switch between them seamlessly. Your conversation continues even when switching models.

### Tool Execution
- **Read/Write/Edit/MultiEdit** - File operations with syntax highlighting
- **Bash** - Run shell commands with streaming output
- **Glob/Grep/List** - Search files and content (ripgrep-powered)
- **Explore** - Spawn parallel sub-agents for codebase analysis
- **Build** - Spawn parallel builder agents for complex operations
- **Apply Patch** - Multi-file patch application
- **Ask User** - Interactive prompts with multi-choice or custom input

### Plan/Build Mode
Toggle between structured planning and execution modes with `Ctrl+G`:
- **Plan Mode** - Restricts write operations, focuses on task planning with phases and tasks
- **Build Mode** - Enables all tools for execution of approved plans

Plans are stored as markdown in `~/.krusty/plans/` and can be managed with `/plan`.

### Terminal Integration
Open an interactive terminal session with `/terminal` for direct shell access within the TUI.

### Context Compression
Use `/pinch` to compress long conversations into a new session with summarized context, preserving essential information while reducing token usage.

### Skills
Agent Skills-compatible, progressively disclosed instruction sets. Mitsuro
discovers native, Agent Skills, Pi, OpenCode, Claude, Codex, project, and plugin
package roots with deterministic precedence and local allow/ask/deny policy.
Browse and manage them with `/skills`.

### Plugins
Transactional plugin bundles can contribute TUI components, agent extensions,
skills, MCP configuration, hooks, and assets. Installs distinguish signed,
npm-unsigned, and local-unsigned trust; lifecycle scripts are off by default;
permissions, pinning, updates, reconciliation, and uninstall are explicit.
Manage them with `/plugins`.

### Agent Extensions
Trusted local JavaScript/TypeScript workers can register tools, slash commands,
canonical lifecycle observers, persistent state, and bounded turn context.
Global, project, and permissioned package roots hot-reload with last-known-good
recovery.

### Hooks
Pre and post-tool execution hooks for custom workflows. Configure with `/hooks`.

### Permission Modes
- **Supervised** (default) - Requires approval for write operations
- **Autonomous** - Auto-executes all tools

Switch with `/permissions`.

### Sessions
All conversations are saved locally in SQLite. Resume any session with `/load` (filtered by current directory).

### Themes
The legacy TUI currently retains its existing theme catalog, including the compatibility theme key `krusty`. This surface is intentionally excluded from the v1 visual conversion pending its rebuild.

### Auto-Updates
Mitsuro checks for updates via `/update`. Windows can apply the standalone
binary update in place after verifying the release SHA-256 manifest.
Hive-capable Unix releases distribute the compatibility `krusty` binary,
supervised `krusty-mako` service, and service units as one set, so Mitsuro fails
closed instead of updating only one binary. Use a complete package channel or the checksum-verifying
installer after that release is published; until then, build the complete set
from source.

## Configuration

Data stored in `~/.krusty/`:

```
~/.krusty/
├── credentials.json  # API keys (encrypted)
├── preferences.json  # Settings (theme, model, recent models)
├── extensions/       # Zed WASM plus executable agent extensions
├── plugins/          # Immutable package snapshots, lockfile, trust, grants
├── bin/              # Auto-downloaded LSP binaries
├── skills/           # Custom global skills
├── plans/            # Markdown plan files
├── tokens/           # LSP and MCP authentication
├── mcp_keys.json     # MCP server credentials
└── logs/             # Application logs
```

### Project Configuration

Add a `CLAUDE.md` file to your project root for project-specific instructions that are automatically included in context. Generate one with `/init`.

Project-level skills in `.krusty/skills/` override global skills.

## Documentation

Detailed project documentation lives in [`docs/`](docs/README.md):

- **[Architecture](docs/architecture/)** - System overview, data flow, and design decisions
- **[Core Engine](docs/core-engine/)** - Agent orchestrator, AI providers, tools, context management
- **[Storage](docs/storage/)** - SQLite persistence layer
- **[Interfaces](docs/interfaces/)** - TUI, web server/API, ACP editor integration, and Hive autonomous mode
- **[Frontends](docs/frontends/)** - Mobile app (Expo), desktop app (Tauri), shared packages
- **[Extensions](docs/extensions/)** - Plugin packages, agent extensions, WASM, MCP, plans, and skills
- **[Operations](docs/operations/)** - Build, CI/CD, packaging, and deployment

## Tech Stack

| Layer | Technology |
|-------|-----------|
| **Backend** | Rust (tokio, axum, ratatui, rusqlite, wasmtime) |
| **Mobile** | Expo / React Native (iOS, Android) |
| **Web** | Expo web export (embedded in Rust binary via rust-embed) |
| **Desktop** | Tauri v2 (wraps Expo web build) |
| **Shared Frontend** | TypeScript, React 19, Zustand |
| **Package Manager** | Cargo (Rust), Bun (TypeScript) |
| **Database** | SQLite (embedded, local-first) |
| **Extensions** | Signed/npm/local bundles; Bun agent workers; WebAssembly (Wasmtime, Zed-compatible WIT) |
| **Protocols** | ACP (editor integration), MCP (tool discovery) |

## Development

### Rust backend

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace
```

### Expo frontend (mobile + web)

```bash
cd apps/mobile
bun install
bun run start              # dev server (all platforms)
bun run web                # web only
npx expo export --platform web  # production web build
```

### Desktop (Tauri)

```bash
cd apps/desktop/shell
bun install
bun run dev                # Tauri dev with Expo web hot-reload
bun run build              # production build (DEB, RPM)
```

## License

MIT License - see [LICENSE](LICENSE) for details.
