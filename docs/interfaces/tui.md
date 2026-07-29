# The Terminal UI

> **Legacy surface:** the terminal UI remains functional but is intentionally excluded from the Mitsuro v1 visual conversion. Its current mascot, ASCII, color, and theme behavior are documented here as-is until the planned ground-up TUI rebuild.

Mitsuro's terminal interface is built on Ratatui and crossterm. It runs inside any modern terminal emulator, renders at 60fps when animations are active, and supports everything from mouse-driven scrolling to gamepad input. This document walks through how the interface is structured, how it processes events, and how the various subsystems fit together.

## Architecture

The TUI is organized around a single `App` struct that owns three top-level state groups:

- **`AppUi`** holds everything related to what the user sees: the current view (start menu or chat), which popup is open, the active theme, the input editor, the autocomplete and file search popups, the scroll system, block UI states, the markdown cache, and toast notifications. A `needs_redraw` flag tracks whether the screen actually needs to be repainted.

- **`AppRuntime`** holds everything related to the AI session and background work: the chat state (messages and conversation history), the current model and provider, the AI client, the agent event bus and cancellation token, the block manager that owns all visual blocks, the tool result cache, background process tracking, git status, and the various async channels that connect the TUI to the orchestrator.

- **`AppServices`** holds long-lived external systems: the tool registry, the session and plan managers, the credential store, the model registry, the MCP manager, the WASM extension host, the plugin manager, and the skills manager.

This three-way split keeps rendering logic separate from AI orchestration logic, and both separate from the services they depend on. The `App` struct itself lives on the main thread and is never shared across threads. All async work happens through channels.

### The Event Loop

The main event loop runs inside a tokio runtime. Each iteration:

1. Polls crossterm for terminal events (keyboard, mouse, paste, resize).
2. Drains pending orchestrator events from the `LoopEvent` channel.
3. Polls background process output channels (bash, explore, build).
4. Polls git status and plugin catalog on a 2-second interval.
5. Ticks all block animations.
6. Renders the frame if anything changed.

The loop uses `needs_redraw` tracking to avoid unnecessary renders. When the UI is idle and nothing is streaming, the loop sleeps efficiently on the crossterm event stream instead of busy-polling.

### Views

The application has two top-level views:

- **StartMenu** -- the landing screen with the Mitsuro logo, an input bar, and animated menu items. This is where you pick up an existing session or start a new one.
- **Chat** -- the main working view with the toolbar at the top, a scrollable message area in the center, the input editor at the bottom, and the status bar below that. The plan sidebar and plugin window can appear alongside the message area when active.

Switching between views is managed through `AppUi::view` and an optional `pending_view_change` that gets applied at the end of the event loop tick to avoid mid-frame state inconsistencies.

## The Block System

Every piece of non-text content in the chat stream -- bash output, file diffs, read previews, thinking traces, web search results, explore progress -- is rendered by a "block." Blocks implement the `StreamBlock` trait:

```rust
pub trait StreamBlock: Send + Sync {
    fn height(&self, width: u16, theme: &Theme) -> u16;
    fn render(&self, area: Rect, buf: &mut Buffer, theme: &Theme, focused: bool, clip: Option<ClipContext>);
    fn handle_event(&mut self, event: &Event, area: Rect, clip: Option<ClipContext>) -> EventResult;
    fn tick(&mut self) -> bool;
    fn is_streaming(&self) -> bool;
}
```

Each block calculates its own height, renders itself into a buffer region, handles mouse and keyboard events within its bounds, and reports whether it needs animation ticks. The `ClipContext` parameter tells a block when it is partially scrolled off-screen so it can skip drawing the clipped border.

### Block Types

- **BashBlock** -- terminal-style command output. Shows the command in the header, streams output as it arrives, displays a blinking cursor while running, detects progress bars, and reports the exit code and duration when complete. Scrollable, collapsible after completion.
- **EditBlock** -- GitHub-style diffs. Shows file edits in either unified or side-by-side view with line numbers, red/green gutter markers, and context lines. An animated symbol toggles while the edit is in flight. The diff mode is global and toggleable.
- **WriteBlock** -- similar to EditBlock but for new file creation.
- **ReadBlock** -- file content preview with syntax highlighting.
- **ThinkingBlock** -- extended thinking traces from reasoning models. Collapsible, with signature verification support.
- **ExploreBlock** -- progress display for the explore/init agent, showing a consortium of animated crab workers.
- **BuildBlock** -- build process tracking tied to plan execution.
- **ToolResultBlock** -- generic display for tool outputs that do not have a dedicated block type.
- **WebSearchBlock** -- search results with titles and URLs.
- **TerminalPane** -- embedded interactive terminal (PTY) with full keyboard forwarding.

All block instances are managed by the `BlockManager`, which holds typed vectors for each block type. The block manager provides `tick_all()` to advance animations, `poll_terminals()` to collect PTY output, and index-based access for hit-testing and event routing.

### Block Scrolling

Blocks use two scrolling strategies depending on their content:

- **SimpleScrollable** for blocks with a fixed line count (tool results, web search results). Total lines are known without a render width.
- **WidthScrollable** for blocks with width-dependent wrapping (read, write, thinking blocks). These must recalculate their wrapped lines whenever the terminal width changes.

Both traits provide `scroll_up()`, `scroll_down()`, `max_scroll()`, and `needs_scrollbar()` methods.

### Block UI State

Visual state like collapsed/expanded and scroll offset is stored separately from the blocks themselves in `BlockUiStates`, a HashMap keyed by stable IDs (tool_use_id or content hash). This decoupling means that when blocks are reconstructed -- for example, when reloading a session -- their UI state survives. The states are exportable to the database for session persistence and importable on restore.

## Input System

The input editor is a multi-line text editor with word wrapping, cursor management, and clipboard support.

### The Multi-Line Editor

`MultiLineInput` stores the raw text as a `String` with a byte-offset cursor position. It maintains a visual cursor position (line, column) that accounts for word wrapping at the current terminal width. The editor supports:

- **Standard editing**: character insertion, backspace, delete, word-delete (Ctrl+W), kill-to-end (Ctrl+K), clear-line (Ctrl+U).
- **Navigation**: arrow keys move through wrapped lines, Home/End jump within a line, Ctrl+A/Ctrl+E move to line start/end.
- **Multi-line input**: Shift+Enter or Alt+Enter inserts a newline, plain Enter submits.
- **Clipboard**: Ctrl+V pastes text or images. Image paste uses arboard and inserts a `[clipboard:uuid]` placeholder that is resolved later into a base64 image attachment.
- **Scrolling**: when the input grows taller than its allocated area, the editor scrolls to keep the cursor visible.

The editor uses a wrapped-line cache that is invalidated whenever the content or terminal width changes.

### Autocomplete

When you type a `/` at the start of the input, the autocomplete popup activates. It uses fuzzy matching against all registered slash commands, scoring exact matches highest, then prefix matches, then substring matches, then character-by-character fuzzy hits. The popup renders up to 7 suggestions and navigates with Tab/Up/Down. Enter selects the highlighted command.

### File Search

Typing `@` triggers the file search popup, which has two modes toggled by Ctrl+F:

- **Fuzzy mode** -- searches file names in the working directory.
- **Tree mode** -- navigates the directory tree with arrow keys (left goes up, right enters a directory).

Selecting a file inserts a `[path]` reference into the input. These bracketed paths are recognized by the image parser and by click-detection for file previews.

### Image Attachment

The input parser (`image_parser`) scans for image references in the text: bracketed file paths pointing to supported image formats (PNG, JPG, GIF, WebP, PDF), clipboard placeholders from Ctrl+V paste, and pasted paths that are auto-wrapped in brackets. These are resolved into base64-encoded image content and sent as multimodal message parts to the AI provider.

## Slash Commands

The command system maps input prefixed with `/` to application actions. Commands are handled synchronously in `handle_slash_command()`:

| Command | Action |
|---------|--------|
| `/home` | Return to start menu, clear session |
| `/load` | Open session list popup |
| `/model` | Open model selector |
| `/fast` | Toggle the selected model's advertised Fast mode |
| `/auth` | Manage API providers |
| `/init` | Analyze codebase and generate KRAB.md |
| `/theme` | Open theme picker |
| `/clear` | Clear chat messages and blocks |
| `/pinch` | Continue in new session with summarized context |
| `/cmd` | Show all keyboard controls |
| `/terminal` | Open interactive terminal pane |
| `/ps` | View background processes |
| `/skills` | Browse and manage skills |
| `/plugins` | Browse and manage installable plugins |
| `/plan` | View or manage active plan |
| `/mcp` | Browse MCP servers |
| `/hooks` | Configure tool execution hooks |
| `/permissions` | Toggle supervised/autonomous mode |
| `/update` | Check for updates; Unix upgrades use the package manager or verified installer so Mitsuro and Hive remain aligned |

Unknown commands produce a system message. Each command either manipulates state directly (e.g., `/clear`), opens a popup (e.g., `/model`), or kicks off an async operation (e.g., `/init`).

## Markdown Rendering

AI assistant responses are rendered as rich markdown. The pipeline has two stages.

### Parsing

The parser uses `pulldown-cmark` with tables, strikethrough, and task list extensions enabled. Raw markdown text is converted into a tree of `MarkdownElement` values: paragraphs, headings (with level), code blocks (with optional language), block quotes, ordered and unordered lists (with task list support), tables, and thematic breaks. Inline content is parsed into its own model: plain text, bold, italic, strikethrough, inline code, links (with URL detection via regex for bare URLs), and images.

### Rendering

The renderer walks the element tree and produces Ratatui `Line` and `Span` values styled according to the active theme. Key rendering behaviors:

- **Code blocks** get box-drawn borders (rounded corners), a language label in the header, and syntax highlighting via tree-sitter. The border string is pre-computed and cached to avoid per-frame allocations.
- **Block quotes** are prefixed with a styled vertical bar and indented.
- **Tables** are rendered with box-drawing characters for headers, separators, and cell borders, with column widths calculated from content.
- **Headings** are bold and use the title color.
- **Links** are tracked with position metadata so they can receive OSC 8 hyperlink sequences and hover highlighting after the buffer is rendered.
- **Lists** render with bullets or numbers, properly indented for nesting.

A `MarkdownCache` avoids re-rendering unchanged content. The cache is keyed by the raw text, and when the underlying text for a message changes (during streaming, for example), the cache entry is invalidated and re-rendered on the next frame.

## Theme System

Mitsuro ships with 31 themes. Each theme is a `Theme` struct with over 70 named color fields organized into groups: core colors, mode colors, special colors (warning, error, code background), UI element colors, message role colors, syntax highlighting colors, diff colors, and display colors (scrollbar, logo, animation, token usage, link).

### Theme Builder

Themes are created using a `ThemeBuilder`. You set seven core colors and optionally override any extended field. Unset fields are derived from the core colors -- `input_bg_color` defaults to `code_bg_color`, syntax colors derive from mode colors, and diff backgrounds are calculated from success/error colors at 1/6 intensity.

### Contrast Balancing

After building, every theme goes through `balance_theme_contrast()`, which enforces minimum WCAG contrast ratios between foreground and background pairs. Text gets 4.5:1, dim text 2.4:1, borders 1.35:1, accent colors 2.5:1. Surface colors are normalized to a contrast band against the main background. The enforcement uses binary search over color mixing to find the minimal adjustment needed.

Themes span terminal-native colors, popular editor palettes (Tokyo Night, Dracula, Catppuccin, Gruvbox, Nord, and more), and novelty options (Sith Lord, Matrix, Cyberpunk, Retro Wave).

## Components

### Toolbar

The toolbar sits at the top of the chat view. It shows the current work mode (BUILD or PLAN) as a colored badge, the session title (click to edit), and a plan progress indicator when a plan is active. During streaming, an animated shark swims back and forth across the toolbar -- 26 frames at 120ms per frame.

### Status Bar

The status bar at the bottom displays the working directory, the active model name (shortened), the git branch name and dirty file count, the token usage with a color gradient (green at low usage, yellow at moderate, pink at high, red at critical), and the count of running background processes with their elapsed time.

### Plan Sidebar

When a plan is active, Ctrl+T toggles a sidebar on the right side of the screen (76 columns wide, only shown when the terminal is at least 140 columns). It displays the plan's phases and tasks with status icons: a checkmark for completed, a spinner for in-progress, and a bullet for pending. The sidebar supports smooth expand/collapse animation and scrolling for long plans. It uses content caching keyed by a hash of the plan data to avoid rebuilding every frame.

### Decision Prompts

When the agent needs user input -- tool approval, plan confirmation, or custom questions via the AskUserQuestion tool -- a decision prompt appears inline. Options can be selected by number keys, arrow navigation, or typing a custom response. Multi-select is supported with space to toggle. Backspace goes back to the previous question, and Escape dismisses the prompt.

### Scrollbars

The message area and individual blocks render scrollbar tracks on the right edge. The scrollbar supports click-to-jump, drag-to-scroll, and hover highlighting. Mouse position is tracked to show which scrollbar the cursor is near.

### Toast Notifications

Transient messages (errors, confirmations) appear as toasts that render above the input area and auto-dismiss after a timeout.

## Event Handling

### Keyboard

Keyboard input goes through a dispatch chain. First, if the title is being edited, keys route to the title editor. If a popup is open, keys route to the popup's handler. If the plugin window is focused, keys go to the active plugin (except Delete to unfocus and Ctrl+Q to quit). If a terminal pane is focused, keys go to the PTY (except Escape to unfocus). Otherwise, keys are processed by the main handler.

Global shortcuts available in all contexts:
- **Ctrl+Q** -- quit
- **Ctrl+B** -- open process list
- **Ctrl+T** -- toggle plan sidebar
- **Ctrl+P** -- toggle plugin window
- **Ctrl+G** -- toggle BUILD/PLAN mode
- **Tab** -- cycle thinking level (Off, Low, Medium, High, XHigh)
- **PageUp/PageDown** -- scroll the message area
- **Escape** -- interrupt the active AI task (in chat view)

The input editor handles its own subset: Enter submits, Shift+Enter adds a newline, Ctrl+V pastes, Ctrl+W deletes a word, Ctrl+U clears the line, Ctrl+C interrupts the current task.

### Mouse

Mouse events handle scrolling (both the message area and individual blocks), left-click (collapse/expand blocks, click scrollbar tracks, click links, select text, focus terminal panes), drag (scrollbar dragging, text selection with edge-scroll), and hover (link highlighting, scrollbar glow). The mouse handler performs hit-testing against all rendered block areas to determine which block (if any) a click or scroll targets.

### Paste

Bracketed paste events are routed to the focused context: terminal pane, auth popup API key input, pinch popup text input, or the main input editor. When pasting into the main editor, file paths that point to existing supported files are automatically wrapped in brackets for preview and attachment support.

## Streaming

When a message is sent to the AI, the orchestrator runs in a background task and communicates with the TUI through a `LoopEvent` channel. The TUI does not manage the AI stream directly -- it consumes events that describe what happened.

### Adaptive Stream Drain

Events arrive faster than the TUI can render frames. The `StreamDrainState` manages a queue with three operating modes:

- **Smooth** -- processes a small batch per frame (the default).
- **Moderate** -- larger batch when the queue grows past a threshold or the oldest event exceeds a time limit.
- **CatchUp** -- drains aggressively when the backlog is severe.

Adjacent text deltas and thinking deltas are coalesced in the queue -- two consecutive `TextDelta` events merge into one with concatenated text. When the queue hits its hard limit, low-priority events (text deltas, thinking deltas, tool executing notifications) are dropped to keep tool results and structural events flowing.

### Event Types

The orchestrator emits events covering the full lifecycle: text and thinking deltas, tool start/complete/execute/result, approval and input requests, server-side tool results, plan and mode changes, token usage and context compaction, session titles, turn completion, errors, background agent coordination, and Hive teammate events.

Each event is translated into visual state: text deltas append to the current assistant message, tool events create or update blocks, approval events show decision prompts, and plan events update the sidebar.

When the orchestrator finishes, the TUI reloads the full conversation from the database to ensure it matches what was persisted, then clears all streaming state and checks whether auto-pinch (context compaction into a new session) should be triggered.

## Plugins

The plugin system provides a trait-based architecture for hosting custom content in a dedicated window panel.

### The Plugin Trait

Plugins implement the `Plugin` trait with methods for rendering (text or pixel), event handling, animation ticks, and lifecycle hooks (activate/deactivate). Two render modes are supported:

- **Text** -- standard Ratatui widgets rendered into a buffer region.
- **KittyGraphics** -- pixel-level rendering at up to 60fps using the Kitty graphics protocol.

### Kitty Graphics

The `KittyGraphics` handler transmits pixel data to supporting terminals (Ghostty, Kitty, WezTerm) via escape sequences. It uses zlib compression (80-95% size reduction), RGB mode when alpha is not needed (25% less data), and double-buffering for flicker-free updates. Frames are shared between the plugin and the graphics system using `Arc<Vec<u8>>` for zero-copy transfer.

### Gamepad Input

On Unix platforms, the `GamepadHandler` uses gilrs for controller support. It polls for button events, handles hotplugging, and maps physical buttons to libretro joypad IDs. This powers the RetroArch plugin, which can run retro game emulation inside the terminal using the Kitty graphics protocol for display and gamepad or keyboard for input.

### Managed Plugins

The `ManagedPlugin` type represents installable plugins loaded from `~/.krusty/plugins`. Each has a descriptor with ID, name, version, publisher, description, and render mode. The plugin manager handles installation, updates, and enable/disable state. A global registry tracks installed plugin descriptors, and the plugin catalog is polled every 2 seconds for state changes.

### The Plugin Window

The plugin window is toggled with Ctrl+P. When focused, all keyboard input routes to the active plugin (except Delete to unfocus and Ctrl+Q to quit). The window remembers the user's preferred plugin from preferences and restores it on open.

## Popups

Popups are modal overlays that handle specific interactions. Each has its own state struct and key handler. The active popup is tracked by the `Popup` enum on `AppUi`:

- **Auth** -- provider selection, API key input with browser-based OAuth support.
- **ModelSelect** -- searchable model picker organized by provider, with recent models at the top.
- **ThemeSelect** -- scrollable theme list with live preview.
- **Help** -- keyboard shortcut reference.
- **SessionList** -- previous sessions for the current directory, with title and timestamp.
- **McpBrowser** -- MCP server status and tool listing.
- **ProcessList** -- background process monitoring.
- **PluginsBrowser** -- install, update, enable, and disable plugins.
- **Pinch** -- context compaction wizard with preservation and direction inputs.
- **FilePreview** -- read-only file viewer triggered by clicking bracketed paths.
- **SkillsBrowser** -- browse and manage skills (global and project-level).
- **Hooks** -- configure pre/post tool execution hooks.

All popups follow a consistent visual pattern: rounded borders, a title bar, a separator, scrollable content, and a footer with available actions. They are dismissed with Escape.
