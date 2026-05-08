```
▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌
█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌
▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪
▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.
·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ •
```

## Overview

Krusty is a multi-platform AI coding assistant. A single Rust binary bundles a terminal TUI, a web server with an embedded frontend, editor integration via ACP, and an autonomous agent mode (Mako). The frontend is built with Expo (React Native) and shared across mobile (iOS/Android), web (embedded in the server binary), and desktop (Tauri wrapper).

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
curl -fsSL https://raw.githubusercontent.com/honeycomb-Technologies/Krusty/main/install.sh | sh
```

Or from source:

```bash
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libudev-dev
git clone https://github.com/honeycomb-Technologies/Krusty.git
cd Krusty
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
| `krusty mako run "task"` | Submit a task to the autonomous agent |
| `krusty mako status` | Show Mako session status |
| `krusty mako attach <id>` | Attach to a running Mako session |

`krusty serve` bundles everything — API server, agent runtime, and the embedded Expo web build — into a single process. On first run it walks you through provider and API key setup. If Tailscale is installed, it auto-configures remote HTTPS access.

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
Modular instruction sets for domain-specific tasks. Add custom skills in `~/.krusty/skills/` or project `.krusty/skills/`. Browse with `/skills`.

### Plugins
Extensible plugin system with install, enable/disable, and reload support. Manage with `/plugins`.

### Hooks
Pre and post-tool execution hooks for custom workflows. Configure with `/hooks`.

### Permission Modes
- **Supervised** (default) - Requires approval for write operations
- **Autonomous** - Auto-executes all tools

Switch with `/permissions`.

### Sessions
All conversations are saved locally in SQLite. Resume any session with `/load` (filtered by current directory).

### Themes
31 built-in themes including krusty (default), tokyo_night, dracula, catppuccin_mocha, gruvbox_dark, nord, one_dark, solarized_dark, synthwave_84, monokai, rosepine, and more. Switch with `/theme`.

### Auto-Updates
Krusty checks for updates and can self-update via `/update`.

## Configuration

Data stored in `~/.krusty/`:

```
~/.krusty/
├── credentials.json  # API keys (encrypted)
├── preferences.json  # Settings (theme, model, recent models)
├── extensions/       # Zed WASM LSP extensions
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
- **[Interfaces](docs/interfaces/)** - TUI, web server/API, ACP editor integration, Mako autonomous mode
- **[Frontends](docs/frontends/)** - Mobile app (Expo), desktop app (Tauri), shared packages
- **[Extensions](docs/extensions/)** - WASM extension system, MCP, plugins, plans, skills
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
| **Extensions** | WebAssembly (Wasmtime, Zed-compatible WIT) |
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
