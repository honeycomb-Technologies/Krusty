# Krusty Documentation

Welcome to the Krusty project documentation. These documents explain how every part of the system works, why specific design choices were made, and how everything connects together.

## How to Read These Docs

**If you're an investor or non-technical reader**, start with:
1. [What Is Krusty?](architecture/overview.md) - The big picture in plain language
2. [Design Decisions](architecture/design-decisions.md) - Why we built it this way

**If you're an engineer evaluating the codebase**, start with:
1. [What Is Krusty?](architecture/overview.md) - System overview
2. [How a Message Flows Through the System](architecture/data-flow.md) - End-to-end trace
3. Then dive into whichever subsystem interests you

**If you're a contributor**, start with:
1. [Data Flow](architecture/data-flow.md) - Understand the full pipeline
2. [Agent Orchestrator](core-engine/agent-orchestrator.md) - The core loop
3. Then read the docs for whichever area you'll be working in

---

## Architecture

High-level system design and the reasoning behind it.

| Document | Description |
|----------|-------------|
| [What Is Krusty?](architecture/overview.md) | Plain-language overview of the entire system, its four interfaces, and what problems it solves |
| [Data Flow](architecture/data-flow.md) | Trace a single user message from input through every subsystem to response |
| [Design Decisions](architecture/design-decisions.md) | The 12 major architectural choices, what alternatives existed, and why we chose what we did |

## Core Engine

The heart of the system — the agent loop, AI providers, tools, and context management.

| Document | Description |
|----------|-------------|
| [Agent Orchestrator](core-engine/agent-orchestrator.md) | The agentic loop that drives everything: streaming, tool calls, failure detection, hooks |
| [AI Providers](core-engine/ai-providers.md) | Multi-provider LLM abstraction: how we talk to Anthropic, OpenAI, and others through one interface |
| [Tool System](core-engine/tool-system.md) | 50+ built-in tools, the registry, execution lifecycle, permissions, and hooks |
| [Context & Memory](core-engine/context-and-memory.md) | How the agent builds context, manages conversation history, and handles summarization |
| [Agent Efficiency](core-engine/agent-efficiency.md) | Compact tool exposure, stable prompt/cache layers, rendered-request budgeting, compaction, and usage regression gates |
| [Sub-Agents & Teams](core-engine/sub-agents.md) | Parallel agent delegation, the team system, and auto-classification |

## Storage

Data persistence and state management.

| Document | Description |
|----------|-------------|
| [Persistence Layer](storage/persistence-layer.md) | SQLite architecture, 15+ table managers, session lifecycle, and credential storage |

## Interfaces

The four ways users interact with Krusty.

| Document | Description |
|----------|-------------|
| [Terminal UI (TUI)](interfaces/tui.md) | Ratatui-based terminal interface: blocks, themes, input system, markdown rendering |
| [Web Server & API](interfaces/server-api.md) | Axum HTTP/WebSocket server, REST endpoints, SSE streaming, push notifications |
| [ACP Editor Integration](interfaces/acp-editor-integration.md) | How editors (Zed, Neovim, JetBrains) connect via the Agent Client Protocol |
| [Mako Autonomous Mode](interfaces/mako-autonomous-mode.md) | Background autonomous agent: tick engine, swarm execution, CLI controls |

## Mako Engineering

Implemented runtime architecture, operations, and data-handling guidance for
the Mako surface. Superseded product plans live in the documentation archive.

| Document | Description |
|----------|-------------|
| [Mako Engineering Index](mako/README.md) | Current runtime architecture, autonomous-mode behavior, operations, and privacy guidance |

## Frontends

Client applications that connect to the Krusty server.

| Document | Description |
|----------|-------------|
| [Mobile App](frontends/mobile-app.md) | Expo/React Native app: platform abstraction, components, widgets, state management |
| [Conversation Workstream](frontends/conversation-workstream.md) | Shared mobile/web transcript contract: streaming deltas, tool blocks, activity states, errors, and interaction behavior |
| [Desktop App](frontends/desktop-app.md) | Tauri wrapper around the Expo web build for native desktop distribution |
| [Shared Packages](frontends/shared-packages.md) | TypeScript monorepo packages shared between mobile and desktop: API client, state, UI |

## Extensions

How Krusty is extended beyond its built-in capabilities.

| Document | Description |
|----------|-------------|
| [WASM Extensions](extensions/wasm-extensions.md) | Zed-compatible WebAssembly extension system, WIT contracts, runtime hosting |
| [Agent Extensions](extensions/agent-extensions.md) | JavaScript/TypeScript agent tools, slash commands, lifecycle events, and turn context |
| [MCP, Plugins, Plans & Skills](extensions/mcp-and-plugins.md) | Five cooperating layers: MCP, packages, agent extensions, planning, and skills |
| [Plugin Packages](extensions/plugin-packages.md) | Unified bundles, lifecycle, trust, permissions, and package contributions |
| [Extensibility Parity](extensions/extensibility-parity.md) | Evidence-based OpenCode, Pi, Codex, and Krusty comparison with the 10-point completion rubric |

## Operations

Building, testing, deploying, and packaging.

| Document | Description |
|----------|-------------|
| [Build, CI/CD & Packaging](operations/build-and-deploy.md) | Cargo workspace, GitHub Actions, AUR/Homebrew packaging, Expo/Tauri builds |

## Archive

Historical audit, roadmap, tracker, and closure documents that are preserved for reference but no longer represent the main documentation set.

| Document | Description |
|----------|-------------|
| [Documentation Archive](archive/README.md) | Historical planning, audit, competitive research, tracker, and closure documents |
