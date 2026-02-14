```
▄ •▄ ▄▄▄  ▄• ▄▌.▄▄ · ▄▄▄▄▄ ▄· ▄▌
█▌▄▌▪▀▄ █·█▪██▌▐█ ▀. •██  ▐█▪██▌
▐▀▀▄·▐▀▀▄ █▌▐█▌▄▀▀▀█▄ ▐█.▪▐█▌▐█▪
▐█.█▌▐█•█▌▐█▄█▌▐█▄▪▐█ ▐█▌· ▐█▀·.
·▀  ▀.▀  ▀ ▀▀▀  ▀▀▀▀  ▀▀▀   ▀ •
```
The Fun AI coding assistant. 

## Installation

### Quick Install (Linux/macOS)

```bash
curl -fsSL https://raw.githubusercontent.com/BurgessTG/Krusty/main/install.sh | sh
```

### Homebrew (macOS/Linux)

```bash
brew tap BurgessTG/tap
brew install krusty
```

### From Source

```bash
git clone https://github.com/BurgessTG/Krusty.git
cd Krusty
cargo build --release
./target/release/krusty
```

### GitHub Releases

Download prebuilt binaries from [Releases](https://github.com/BurgessTG/Krusty/releases):
- Linux (x86_64, ARM64)
- macOS (Intel, Apple Silicon)
- Windows (x86_64)

## Supported Providers

Krusty supports multiple AI providers. Add API keys via `/auth` in the TUI.

| Provider | Models |
|----------|--------|
| **MiniMax** | MiniMax M2.1 Lightning, MiniMax M2.1, MiniMax M2 |
| **OpenAI** | GPT 5.2 Codex, GPT 5.2 |
| **OpenRouter** | 100+ Frontier and OSS models |
| **Z.ai** | GLM-4.7, GLM-4.5-Air |

Switch providers and models anytime with `/model` or `Ctrl+M`.

## Controls

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | New line in input |
| `Ctrl+C` | Cancel current generation |
| `Ctrl+L` | Clear screen |
| `Ctrl+M` | Open model selector |
| `Ctrl+N` | New session |
| `Ctrl+P` | View background processes |
| `Ctrl+K` | Open command palette |
| `Ctrl+G` | Toggle BUILD/PLAN mode |
| `Ctrl+T` | Toggle plan sidebar |
| `Ctrl+P` | Open plugin window |
| `Ctrl+Q` | Quit application |
| `Ctrl+V` | Paste text or image |
| `Ctrl+W` | Delete word |
| `Tab` | Toggle extended thinking |
| `Esc` | Close popup / Cancel |
| `@` | Search and attach files |
| `↑/↓` | Scroll / Navigate history |
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
| `/lsp` | Browse and install language servers |
| `/mcp` | Manage MCP servers |
| `/skills` | Browse available skills |
| `/ps` | View background processes |
| `/terminal` | Open interactive terminal |
| `/init` | Generate KRAB.md project context file |
| `/cmd` | Show command help popup |

### Mouse

- Click to select text
- Scroll wheel to navigate
- Click links to open in browser
- Click code blocks to copy

## Features

### Multi-Provider AI
Configure multiple providers and switch between them seamlessly. Your conversation continues even when switching models.

### Language Server Protocol (LSP)
Install language servers from Zed's extension marketplace for 100+ languages:

```bash
krusty lsp install rust
krusty lsp install python
krusty lsp install typescript
```

Or use `/lsp` in the TUI to browse and install interactively.

### Tool Execution
Krusty can execute tools on your behalf:
- **Read/Write/Edit** - File operations with syntax highlighting
- **Bash** - Run shell commands with streaming output
- **Glob/Grep** - Search files and content (ripgrep-powered)
- **Explore** - Spawn parallel sub-agents for codebase analysis
- **Build** - Parallel task execution for complex operations
- **Web Search/Fetch** - Search and fetch web content (Anthropic models)

### Plan/Build Mode
Toggle between structured planning and execution modes with `Ctrl+B`:
- **Plan Mode** - Restricts write operations, focuses on task planning with phases and tasks
- **Build Mode** - Enables all tools for execution of approved plans

Plans are stored as markdown in `~/.krusty/plans/` and can be managed with `/plan`.

### Terminal Integration
Open an interactive terminal session with `/terminal` (or `/term`, `/shell`) for direct shell access within the TUI.

### Context Compression
Use `/pinch` to compress long conversations into a new session with summarized context, preserving essential information while reducing token usage.

### Skills
Modular instruction sets for domain-specific tasks. Add custom skills in `~/.krusty/skills/` or project `.krusty/skills/`.

### Sessions
All conversations are saved locally in SQLite. Resume any session with `/load` (filtered by current directory).

### Themes
31 built-in themes including krusty (default), tokyo_night, dracula, catppuccin_mocha, gruvbox_dark, nord, one_dark, solarized_dark, synthwave_84, monokai, rosepine, and more. Switch with `/theme` or:

### Auto-Updates
Krusty checks for updates and can self-update.

## Configuration

Data stored in `~/.krusty/`:

```
~/.krusty/
├── credentials.json  # API keys (encrypted)
├── preferences.json  # Settings (theme, model, recent models)
├── extensions/       # Zed WASM LSP extensions
├── bin/             # Auto-downloaded LSP binaries
├── skills/          # Custom global skills
├── plans/           # Markdown plan files
├── tokens/          # LSP and MCP authentication
├── mcp_keys.json    # MCP server credentials
└── logs/            # Application logs
```

### Project Configuration

Add a `KRAB.md`, or `CLAUDE.md` file to your project root for project-specific instructions that are automatically included in context. Generate one with `/init`.

Project-level skills in `.krusty/skills/` override global skills.

## License

MIT License - see [LICENSE](LICENSE) for details.
