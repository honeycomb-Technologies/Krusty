```
▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌
█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌
▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪
▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.
·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ •
```

## Overview

Krusty is a single binary that bundles a terminal TUI, a web server with an embedded PWA frontend, and editor integration via ACP — all in one.

## Repository Layout

```
crates/
  krusty-cli/     Terminal UI + CLI entry point
  krusty-core/    Shared AI, tools, storage, runtime
  krusty-server/  API server (library, embedded in CLI)
apps/
  pwa/            SvelteKit PWA frontend (embedded at compile time)
  desktop/        Tauri desktop wrapper
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
| `krusty serve` | Start the web server with embedded PWA (default port 3000) |
| `krusty serve --port 8080` | Start on a custom port |
| `krusty acp` | Run as ACP server for editor integration |

`krusty serve` bundles everything — API server, agent runtime, and PWA frontend — into a single process. On first run it walks you through provider and API key setup. If Tailscale is installed, it auto-configures remote HTTPS access.

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
| **OpenAI** | GPT-5.3 Codex |
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
| `/init` | Generate KRAB.md project context file |
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

Add a `KRAB.md` or `CLAUDE.md` file to your project root for project-specific instructions that are automatically included in context. Generate one with `/init`.

Project-level skills in `.krusty/skills/` override global skills.

## Development

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace
```

PWA frontend (requires [bun](https://bun.sh)):

```bash
cd apps/pwa/app
bun install
bun run check
bun run build
```

## License

MIT License - see [LICENSE](LICENSE) for details.
