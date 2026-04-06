# MCP, Plugins, Plans & Skills

Krusty has four extensibility layers, each solving a different problem. MCP connects the agent to external tool servers. Plugins add dynamic capabilities to the TUI. Plans structure complex work into phased task decomposition. Skills inject domain-specific knowledge into the agent's context. They're independent systems -- you can use any combination of them, or none at all.

This document explains what each one does, how it works internally, and how to configure it.

## MCP (Model Context Protocol)

MCP is an open standard that lets AI systems discover and use tools exposed by external servers. Instead of building every capability directly into Krusty, MCP lets you point Krusty at a server that provides tools, resources, and prompts over a standardized protocol. Krusty acts as an MCP client -- it connects to MCP servers, discovers what they offer, and makes those tools available to the agent just like built-in tools.

The implementation lives in `crates/krusty-core/src/mcp/`, built on the `rmcp` SDK.

### Transport

Krusty supports two transport modes for MCP servers.

**Stdio (local)** servers run as child processes on your machine. Krusty spawns the process, and communication happens over stdin/stdout using JSON-RPC. This is the most common setup -- you point Krusty at a command like `npx @modelcontextprotocol/server-filesystem` and it handles the rest. The working directory, arguments, and environment variables are all configurable per server.

**HTTP/SSE (remote)** servers run somewhere else -- a cloud service, a team server, a SaaS tool. Krusty connects via Streamable HTTP transport, with optional Bearer token authentication. Remote servers can also be passed through to the Anthropic API's MCP Connector feature, letting the API call the server directly rather than routing every call through Krusty.

The HTTP transport is a custom `StreamableHttpClient` implementation handling POST/GET/DELETE operations, SSE event streams, session management, and content-type negotiation.

### Configuration

MCP servers are declared in a `.mcp.json` file at the project root. The format uses `mcpServers` as the top-level key, with each server defined by name:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"],
      "env": {}
    },
    "remote-api": {
      "type": "url",
      "url": "https://mcp.example.com/sse",
      "authorization_token": "${MY_API_KEY}"
    }
  }
}
```

Local servers need a `command` and optional `args` and `env`. Remote servers need `type: "url"` and a `url`, with an optional `authorization_token`. Environment variables in the config support `${VAR}` expansion -- Krusty checks the process environment first, then falls back to its internal credential store at `~/.krusty/tokens/credentials.json`.

Krusty ships with one built-in MCP server (minimax) that gets merged with your `.mcp.json` configuration. User-defined servers override built-ins by name.

### Tool Registration

When Krusty starts, the `McpManager` loads the configuration, connects to all declared servers in parallel, and queries each one for its available tools via `list_tools()`. Each MCP tool is then wrapped in an `McpTool` struct that implements Krusty's standard `Tool` trait, making it indistinguishable from built-in tools as far as the agent is concerned.

MCP tools are registered in the global `ToolRegistry` with a namespaced name: `mcp__{server}_{tool}`. So a tool called `search` on a server named `filesystem` becomes `mcp__filesystem_search`. The tool's JSON Schema is sanitized during registration to ensure it conforms to the strict schema requirements that AI providers expect -- adding missing `properties` and `additionalProperties` fields, filtering invalid `required` entries, and normalizing nested schemas.

When the agent calls an MCP tool, the `McpTool` wrapper routes the call through the `McpManager` to the correct server, converts the result (which can be text, images, or embedded resources), and returns it as a standard `ToolResult`. If the session is running in sandboxed mode, a warning is logged because MCP tools execute on external servers and bypass Krusty's local sandbox restrictions.

### Lifecycle

The `McpManager` holds connections to all servers behind `RwLock<HashMap<String, Arc<McpClient>>>`, supporting concurrent access from the agent loop. Each `McpClient` wraps an rmcp `RunningService` and maintains a tool cache. Liveness is checked by sending a lightweight `list_tools` probe -- if the connection has died, the status flips to `Error` and the UI reflects this.

Servers can be connected and disconnected individually. The manager also exposes server information for the UI: name, transport type, connection status, tool count, and the full tool list with descriptions and schemas.

Beyond tools, MCP servers can also expose resources (data the agent can read) and prompts (parameterized prompt templates). Krusty supports both through `list_resources`, `read_resource`, `list_prompts`, and `get_prompt` methods on the client.

## Plugins

Plugins are installable, signed packages that extend the TUI with dynamic capabilities. Where MCP extends the agent's tool access, plugins extend the user interface -- adding visual components, interactive modes, or integrations that the terminal client renders. Think gamepad input overlays, image rendering via the Kitty graphics protocol, or retro-style terminal effects.

The plugin system lives in `crates/krusty-core/src/plugins/`.

### The Plugin Manifest

Every plugin is defined by a manifest (a TOML or JSON file) that declares its identity and requirements:

```toml
manifest_version = 1
id = "com.example.my-plugin"
name = "My Plugin"
version = "1.2.0"
publisher = "example-team"
description = "Does something useful in the TUI"
entry_component = "plugin.wasm"

[release]
url = "https://example.com/releases/my-plugin-1.2.0.wasm"
sha256 = "abc123..."
signature = "base64-encoded-ed25519-signature"
signing_key_id = "publisher-key-2024"
```

The manifest carries the entry component path (what to load), render capabilities (text or frame-based rendering), requested permissions (filesystem read/write, network, process spawning), and compatibility constraints (minimum/maximum Krusty versions). Permissions are declared upfront and enforced at runtime -- a plugin can't access the network unless it declared `network = true` in its manifest and the user granted that permission.

### Signature Verification

Plugin trust is enforced through ed25519 cryptographic signatures. Before a plugin is installed, Krusty verifies two things:

1. **Publisher allowlist.** The plugin's publisher must appear in the trust policy (`~/.krusty/plugins/trust/allowlist.toml`). If the publisher isn't trusted, installation is rejected with a message to add them first.

2. **Artifact integrity.** The downloaded artifact's SHA-256 hash must match the manifest declaration, and the artifact's ed25519 signature must verify against a trusted public key registered in the trust policy. The signing key ID from the manifest is looked up in the policy's key map, and the signature is verified against the raw artifact bytes.

This chain of trust means you control exactly which publishers can ship plugins to your system, and every artifact is verified as both untampered (hash check) and authentically signed (signature check).

### Installation and Lifecycle

The `PluginManager` manages the full plugin lifecycle under `~/.krusty/plugins/`:

```
~/.krusty/plugins/
  installed/     # Plugin files, organized by id/version
  active/        # Currently active plugin state
  state/         # Persistent plugin state
  index/         # Plugin source registries
  trust/         # Publisher allowlists and signing keys
  plugins.lock   # Lockfile pinning installed versions
```

Installation starts from a manifest reference -- a URL, a local file path, or a `file://` URI. The manager fetches the manifest, validates it, checks the trust policy, downloads the artifact, verifies its integrity and signature, unpacks it into the install directory, and writes a lock entry. The lockfile tracks every installed plugin's ID, version, enabled status, and pin status.

Plugins can be enabled or disabled without uninstalling them. The lockfile records this state, and the TUI checks it when deciding what to load. Sources (registries where plugin manifests are published) are managed separately in `index/sources.toml`, allowing you to add third-party plugin repositories.

### Render Capabilities

Plugins declare whether they render as `text` (standard terminal output) or `frame` (full screen region, like a canvas). If no capability is declared, text is the default. This lets the TUI allocate the right kind of rendering surface for each plugin.

## Plans

Plans are Krusty's answer to complex, multi-step tasks. When a task is too large to tackle in a single pass -- a feature that touches multiple files across several subsystems, a refactor that needs to happen in stages, or an investigation that branches into several directions -- plans break the work into phases, each phase into numbered tasks, and each task into something the agent can execute and check off.

The plan system lives in `crates/krusty-core/src/plan/`.

### Plan Mode vs. Build Mode

Krusty sessions have a work mode: either **plan** or **build**. In plan mode, the agent focuses on decomposing the problem -- reading code, analyzing dependencies, and producing a structured plan. Editing tools are restricted during planning to prevent the agent from jumping into implementation before the plan is ready. Once the plan is approved, the session transitions to build mode, where the agent picks up tasks in order and starts executing.

The lifecycle module in `lifecycle.rs` handles this transition intelligently. If a session's persisted mode says "plan" but the plan already has completed or in-progress tasks, the effective mode is automatically repaired to "build". This prevents sessions from getting stuck in plan mode after work has already started.

### The Plan File Format

Plans are structured as markdown and stored in SQLite (with legacy file-based storage still supported for migration). The format is human-readable:

```markdown
# Plan: Refactor Authentication System

Created: 2025-03-15 14:30 UTC
Session: abc123
Working Directory: /home/user/project
Status: in_progress

---

## Phase 1: Audit Existing Code

- [x] Task 1.1: Map all authentication entry points
  > Result [2025-03-15 15:00]: Found 6 entry points across 3 modules
- [>] Task 1.2: Document token validation flow
- [ ] Task 1.3: Identify deprecated auth methods

## Phase 2: Implement New Token System

- [ ] Task 2.1: Design token schema
- [ ] Task 2.2: Build token generation service
  > Blocked-By: 1.3
- [ ] Task 2.3: Write migration for existing tokens
```

Tasks use checkbox notation to show status: `[ ]` for pending, `[x]` for completed, `[>]` for in progress, and `[~]` for blocked. Each task has a dotted ID (phase.task, like `2.1`) and can carry context notes, blocking relationships, completion results with timestamps, and priority levels. Tasks can also have subtasks (IDs like `1.1.1`) with parent references, creating a hierarchy within phases.

The plan system recognizes task completion through multiple patterns in the agent's responses -- not just checkbox toggling but also natural language like "completed task 1.1", "that completes 2.3", checkmark emoji, and other variations. Eleven regex patterns handle different phrasings so the agent doesn't need to use a specific format to mark work as done.

### Storage and Persistence

Plans have a strict 1:1 relationship with sessions. Each session can have at most one plan, and the plan is stored in SQLite with a foreign key to the session. When a session is deleted, its plan is automatically removed via CASCADE. The `PlanManager` provides the full CRUD interface: create, load, update, abandon, and query.

Legacy plans that were stored as markdown files in `~/.krusty/plans/` are automatically migrated to the database on first access.

Plans track progress at multiple levels. Each phase knows how many of its tasks are complete. The plan itself reports total progress and auto-detects when all tasks are finished. A completed plan is no longer considered "active" and won't be injected into the agent's context.

### The /plan Command

Users interact with plans through the `/plan` slash command. It triggers plan mode, where the agent analyzes the request, breaks it down into phases and tasks, and presents the plan for review. Once approved, the session switches to build mode and the agent starts working through tasks sequentially, checking each one off as it goes. The plan state is injected into the agent's context on every turn, so the agent always knows where it stands and what's next.

## Skills

Skills are the simplest extensibility layer -- they're just files on disk that get loaded into the agent's context when invoked. A skill is a collection of domain-specific instructions, best practices, workflows, and examples written in Markdown with YAML frontmatter. When a skill is active, its content becomes part of what the agent knows and follows.

The skills system lives in `crates/krusty-core/src/skills/`.

### Skill Format

Each skill is a directory containing a `SKILL.md` file:

```
~/.krusty/skills/
  rust-patterns/
    SKILL.md
  git-workflow/
    SKILL.md
    templates/
      commit-template.md
```

The `SKILL.md` file starts with YAML frontmatter declaring the skill's identity:

```yaml
---
name: rust-patterns
description: Idiomatic Rust patterns and best practices
version: 1.0.0
author: krusty
tags:
  - rust
  - patterns
---

# Rust Patterns

## Error Handling

Always use `anyhow::Result` with `.context()` for functions that can fail...
```

The frontmatter requires `name` and `description`. Names must be lowercase letters, numbers, and hyphens only. Version, author, and tags are optional. Everything after the closing `---` is the skill's content, which gets injected into the agent's context verbatim when the skill is activated.

Skills can include additional files beyond `SKILL.md`. The `load_skill_file` function lets you load any file within a skill's directory, with path traversal protection to prevent reading outside the skill boundary.

### Discovery and Loading

The `SkillsManager` scans two directories:

1. **Global skills** at `~/.krusty/skills/` -- available in every session, every project.
2. **Project skills** at `.krusty/skills/` relative to the working directory -- scoped to a specific project.

Project skills take precedence over global skills with the same name. If you have a global `rust-patterns` skill and a project-specific one, the project version wins. This lets teams ship project-specific skills in version control that override your personal defaults.

The manager uses lazy loading with an in-memory cache. Skills are loaded on first access and cached until explicitly refreshed. The `refresh()` method rescans both directories, and `reload_skill()` reloads a single skill after editing without clearing the full cache.

### Context Injection

When skills are available, their metadata (name, description, tags) is included in the agent's system prompt on every turn, so the agent always knows what skills exist. The `/skills` browser in the TUI lets you browse available skills, see their descriptions, and activate them for the current session.

When a skill is activated, `load_skill_content()` returns the SKILL.md body (with frontmatter stripped), and this content is injected into the agent's context. The agent then follows the skill's instructions as if they were part of its base system prompt. This is how you teach Krusty project-specific workflows, coding standards, deployment procedures, or domain knowledge without modifying any code.

### Creating Skills

You can create new skills through the `SkillsManager`:

```rust
manager.create_skill("my-workflow", "Custom deployment workflow")?;
```

This scaffolds a new skill directory with a template `SKILL.md` containing the frontmatter and placeholder sections for quick start, usage, and examples. Skills can also be deleted (global only -- project skills should be managed through version control) and reloaded after editing.

## How They Fit Together

These four layers serve different audiences and different moments in a Krusty session:

- **MCP** extends what tools the agent can call. It's about capability -- connecting to databases, APIs, file systems, or any service that speaks the MCP protocol.
- **Plugins** extend what the TUI can render and interact with. They're about the user interface -- adding visual capabilities that the terminal client wouldn't have otherwise.
- **Plans** extend how work is organized. They're about structure -- breaking large tasks into trackable phases and tasks so nothing gets lost in a long session.
- **Skills** extend what the agent knows. They're about knowledge -- injecting domain expertise, coding standards, and workflows so the agent works the way you want it to.

Each layer does its part without stepping on the others.
