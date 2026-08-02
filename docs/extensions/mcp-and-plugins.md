# MCP, Plugins, Plans & Skills

Mitsuro has five cooperating extensibility layers. MCP connects the agent to
external capability servers. Plugin packages install and govern bundles. Agent
extensions add tools, slash commands, lifecycle events, and turn context. Plans
structure complex work. Skills provide deferred domain instructions. They can
be used independently or distributed together in one package.

This document explains what each one does, how it works internally, and how to configure it.

## MCP (Model Context Protocol)

MCP is an open standard that lets AI systems discover and use tools exposed by external servers. Instead of building every capability directly into Mitsuro, MCP lets you point Mitsuro at a server that provides tools, resources, and prompts over a standardized protocol. Mitsuro acts as an MCP client -- it connects to MCP servers, discovers what they offer, and makes those tools available to the agent just like built-in tools.

The implementation lives in `crates/mitsuro-core/src/mcp/`, built on the `rmcp` SDK.

### Transport

Mitsuro supports two transport modes for MCP servers.

**Stdio (local)** servers run as child processes on your machine. Mitsuro spawns the process, and communication happens over stdin/stdout using newline-delimited JSON-RPC. This is the most common setup -- you point Mitsuro at a command like `npx @modelcontextprotocol/server-filesystem` and it handles the rest. The working directory, arguments, and environment variables are all configurable per server. Child processes start from a cleared environment: only host `PATH`/`HOME` plus values explicitly resolved from the server declaration are supplied, so package/project servers do not inherit ambient credentials. Each inbound JSON-RPC record is limited to 8 MiB; an invalid or oversized record closes the connection and terminates the server's process tree rather than allowing unbounded buffering or leaving descendants behind.

**Streamable HTTP (remote)** servers run somewhere else -- a cloud service, a
team server, or a SaaS tool. Mitsuro requires HTTPS, with a narrow HTTP exception
for loopback development endpoints, and supports optional Bearer or OAuth
authentication. Mitsuro retains connector-ready remote descriptors for future
provider integrations, but current MCP calls are routed through Mitsuro; no
provider request path consumes those descriptors today.

The HTTP transport is a custom `StreamableHttpClient` implementation handling POST/GET/DELETE operations, SSE event streams, session management, and content-type negotiation. Redirects are disabled so credentials cannot be forwarded to a redirect target. Decompressed JSON responses and individual SSE events have hard byte limits and fail closed when exceeded.

### Configuration

MCP configuration is layered. Package-provided fragments are defaults, `~/.mitsuro/mcp.json` overrides packages, and `<project>/.mcp.json` has the highest precedence. The format uses `mcpServers` as the top-level key, with each server defined by name:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"],
      "env": {},
      "cwd": "./tools",
      "enabled": true,
      "required": false,
      "startupTimeoutMs": 15000,
      "toolTimeoutMs": 60000,
      "tools": {
        "allow": ["read_*", "search"],
        "deny": ["read_private"],
        "approval": { "search": "allow", "read_*": "inherit" }
      }
    },
    "remote-api": {
      "type": "url",
      "url": "https://mcp.example.com/mcp",
      "oauth": {
        "scopes": ["repo:read", "repo:write"],
        "clientName": "Mitsuro"
      },
      "headers": { "X-Client": "mitsuro" },
      "envHeaders": { "X-Team-Key": "MY_TEAM_KEY" }
    }
  }
}
```

Local servers need a `command` and accept optional `args`, `env`, and `cwd`.
Remote servers need `type: "url"` and a `url`; they support static headers,
environment-backed headers, interactive `oauth`, `bearerTokenEnvVar`, and the
backward-compatible `authorization_token` field. Host-environment expansion,
environment-backed headers, and credential-store fallback are available only
for user-owned global declarations. Project and package references remain
literal and cannot read host secrets. An explicit bearer token takes precedence
over OAuth, which makes externally managed service tokens a deterministic
override.

Remote OAuth follows MCP's OAuth 2.1 flow: Mitsuro discovers protected-resource and authorization-server metadata, uses S256 PKCE and one-time CSRF state, and dynamically registers a public client. Servers without dynamic registration can set `oauth.clientId`; servers implementing URL-based client metadata can set `oauth.clientMetadataUrl`. Explicit `oauth.scopes` are requested as written, while an empty list lets server metadata choose. Resource and redirect URLs require HTTPS, except for localhost/loopback HTTP callbacks during local development.

Discovered authorization, registration, and token endpoints are independently validated with the same HTTPS/loopback rule before client registration, authorization URL generation, token exchange, or refresh. OAuth and MCP HTTP clients do not follow redirects.

`enabled: false` keeps a server visible without connecting it. A failed
`required: true` server makes startup/reload report an error. User-global stdio
and remote servers auto-connect by default. Package and project declarations,
including remote URLs, always require an explicit connect so installing a
package or opening a repository cannot silently launch a process or make a
network request; setting `autoConnect: true` in untrusted configuration cannot
override this boundary.

Authority is transport-specific. A package stdio declaration is connectable only when the exact installed plugin descriptor has a current `process` grant; a remote declaration requires its current `network` grant. A network-only grant never enables stdio. Project declarations receive no ambient authority and the explicit connect/OAuth action grants only the configured transport. Internal reconnects reuse an already recorded project decision but cannot create one.

Tool rules use shell-style globs. Deny always wins; a non-empty allow list becomes an allowlist. Approval classifications are `inherit`, `prompt`, and `allow`. `prompt` blocks autonomous execution and requires a supervised, approved call. `allow` is metadata for the central policy layer and does not weaken Mitsuro's conservative treatment of unknown remote tools.

### Tool Registration

When Mitsuro starts, the `McpManager` loads the configuration, connects all
enabled auto-connect-eligible servers in parallel, and queries each connected
server for its available tools via `list_tools()`. Each MCP tool is then wrapped
in an `McpTool` struct that implements Mitsuro's standard `Tool` trait, making it
indistinguishable from built-in tools as far as the agent is concerned.

MCP tools are registered in the global `ToolRegistry` with a namespaced name: `mcp__{server}_{tool}`. So a tool called `search` on a server named `filesystem` becomes `mcp__filesystem_search`. The tool's JSON Schema is sanitized during registration to ensure it conforms to the strict schema requirements that AI providers expect -- adding missing `properties` and `additionalProperties` fields, filtering invalid `required` entries, and normalizing nested schemas.

When the agent calls an MCP tool, the `McpTool` wrapper routes the call through the `McpManager` to the correct server and returns a structured `ToolResult`. Text, image, audio, embedded text/blob resources, resource links, annotations, result metadata, and `structuredContent` are preserved. If the session has a scoped local access root, the result warns that the remote server applies its own access policy.

### Lifecycle

The `McpManager` holds connections behind concurrent locks, applies bounded startup/request timeouts, and serializes reconnects per server. Configuration reload/revocation takes an exclusive lifecycle guard while connection startup holds a shared guard, and a generation/config check occurs immediately before client insertion. A stale in-flight client therefore cannot become live after reload, disable, revoke, or uninstall. Streamable HTTP transparently reinitializes expired sessions. A failed read-only resource/prompt request reconnects and retries once. A failed tool call reconnects for future work but is never replayed automatically because its side effects may already have occurred.

Tool, resource, resource-template, and prompt discovery uses bounded pagination. Each catalog has hard item, page, cursor, and aggregate serialized-byte limits; repeated cursors and over-limit pages fail closed instead of growing memory indefinitely.

Servers can be connected and disconnected individually. Server instructions, implementation/capability metadata, configuration source, required/enabled state, tool schemas, annotations, and approval classification are exposed through the server API. `tools/list_changed` notifications invalidate the cache and the API refresh path re-synchronizes the AI tool registry.

Beyond server-specific tools, the agent receives `mcp__list_tools` plus a conservative `mcp__call_tool` dispatcher, so a catalog change is usable before every UI has re-registered named wrappers. Read-only `mcp__list_resources`, `mcp__list_resource_templates`, `mcp__read_resource`, `mcp__list_prompts`, and `mcp__get_prompt` wrappers expose the rest of the protocol. Equivalent server endpoints expose tools, resources, resource templates, and prompts to web/mobile clients.

Mitsuro supports both externally provisioned Bearer tokens and interactive OAuth without placing secrets in project config. OAuth credentials, refresh tokens, and dynamically registered client identity are serialized by rmcp inside Mitsuro's shared atomic owner-only credential store; status and server responses never contain token material. The live HTTP transport asks rmcp for a token on every request, so near-expiry access tokens are refreshed before use. Tokens are keyed by a SHA-256 fingerprint of the normalized MCP resource URL, preventing a same-named server from receiving credentials issued for a different audience.

Web/mobile authorization uses the MCP management API:

- `GET /api/mcp/:name/oauth/status` returns `disabled`, `authorization-required`, `pending`, or `authenticated` plus scopes, never secrets.
- `POST /api/mcp/:name/oauth/start` with `{ "redirectUri": "..." }` returns the authorization URL and flow expiry.
- `GET` or `POST /api/mcp/:name/oauth/callback` validates `code` and `state`, stores credentials, and connects the server.
- `POST /api/mcp/:name/oauth/logout` atomically removes credentials and disconnects the server.

MCP configuration and OAuth credentials belong to the process-wide local
administrator. The MCP, plugin, extension, and skill management route groups
reject tenant-scoped requests until Mitsuro has per-tenant manager instances;
they never silently reuse a tenant identity against the shared credential
store.

Pending browser state expires after ten minutes and is intentionally process-local; durable tokens survive restarts, while an interrupted pre-token browser flow must be started again.

## Plugins

Plugins are installable bundles that can contribute TUI components, agent
extensions, skills, MCP configuration, hooks, and assets. Standalone release
artifacts are publisher-signed; npm and explicitly selected local packages have
separate, visible unsigned trust levels.

The plugin system lives in `crates/mitsuro-core/src/plugins/`.

### The Plugin Manifest

Every plugin is defined by a manifest (a TOML or JSON file) that declares its identity and requirements:

```toml
manifest_version = 1
id = "com.example.my-plugin"
name = "My Plugin"
version = "1.2.0"
publisher = "example-team"
description = "Does something useful in the TUI"
runtime = "js"
entry_component = "plugin.js" # omit for bundle-only packages
skills = ["skills/example/SKILL.md"]
agent_extensions = ["extensions/example.ts"]
mcp_servers = "mcp/servers.json"
hooks = ["hooks/hooks.json"]
assets = "assets"

[requested_permissions]
network = true
process = true

```

All component paths are containment-checked inside an immutable installed
snapshot. Host-mediated capabilities are declared upfront and enforced through
durable, request-bound grants. Granting `process` to a native or JavaScript
component authorizes trusted local code with the user's OS authority; the
`fs_*` and `network` declarations are auditable host permissions, not a kernel
sandbox around that code. Installable WASM TUI entries are currently managed
descriptors and do not execute package code. Mitsuro's isolated
Zed-compatible WASM editor/language host uses a separate ABI and is not a
drop-in sandbox for package TUI or agent-extension code.
Bundles can be distributed through npm, an explicitly selected local package,
or a publisher-signed ZIP release. A signed single-component artifact declares
its `entry_component`; a signed ZIP can declare any combination of the bundle
resources above and may omit `entry_component`. Both use `[release]` metadata
containing `url`, `sha256`, `signature`, `signing_key_id`, and the mandatory
`signature_scheme = "manifest-envelope-v1"`; ZIP releases additionally set
`artifact_kind = "zip-bundle"`. The artifact kind and every component path are
inside the signature envelope. Legacy artifact-only signatures are never
inferred: publishers must select the scheme and re-sign, while unknown schemes
fail closed.

### Signature Verification

Plugin trust is enforced through ed25519 cryptographic signatures. Before a plugin is installed, Mitsuro verifies two things:

1. **Publisher allowlist.** The plugin's publisher must appear in the trust policy (`~/.mitsuro/plugins/trust/allowlist.toml`). If the publisher isn't trusted, installation is rejected with a message to add them first.

2. **Artifact integrity and release-envelope binding.** The downloaded
   artifact's SHA-256 hash must match the manifest declaration. Its Ed25519
   signature covers a domain-separated canonical envelope containing the full
   immutable manifest: identity, version, publisher, runtime, component paths,
   requested permissions, compatibility bounds, release URL, artifact digest,
   and signing-key ID. That key ID must also be explicitly bound to the declared
   publisher.

This chain of trust means you control exactly which publishers can ship plugins
to your system, and every signed single-component or ZIP artifact is verified at
installation as both untampered (hash check) and authentically signed (signature check). The
installed manifest and canonical source retain the inputs for later
activation-time verification. Persisted `source_trust` is provenance, not proof
that current on-disk bytes were rechecked, so it does not alone produce a
`cryptographically_verified` API claim.

### Installation and Lifecycle

The `PluginManager` manages the full plugin lifecycle under `~/.mitsuro/plugins/`:

```
~/.mitsuro/plugins/
  installed/
    .staging/     # Incomplete transactions; safe to reconcile
    .managed/     # Immutable, manager-owned package snapshots
  active/        # Currently active plugin state
  state/         # Persistent plugin state
  index/         # Plugin source registries
  trust/         # Publisher keys, allowlists, and permission grants
  plugins.lock   # Lockfile pinning installed versions
```

Installation stages and validates a complete snapshot before publishing it and
atomically swapping the lockfile. Remote manifests, artifacts, and catalogs
must use HTTPS. npm lifecycle and build scripts are blocked unless the user
passes an explicit script-consent option. The lockfile records source, trust
boundary, script consent, pinning, and the manager-owned root.

Plugins can be enabled, disabled, pinned, unpinned, updated, reconciled, and
uninstalled. Uninstall revokes permission grants before removing lock state and
only recursively deletes a validated manager-owned root after its final
reference is gone. Sources are managed separately in `index/sources.toml` and
may be HTTPS URLs or explicit local paths.

### Render Capabilities

Plugin manifests accept `text` and `frame` render-capability metadata, with
`text` as the default. Current installable native-v1 and JavaScript hosts
execute text rendering only; installable WASM and frame rendering are not wired
to an executable package host yet.

## Agent Extensions

JavaScript and TypeScript agent extensions run in persistent Bun workers and
can register tools, slash commands, canonical loop-event observers, persistent
state, and bounded per-turn context. Project definitions are disabled until a
user-owned project trust grant is recorded; repository settings can only narrow
that grant. Trusted project definitions override global and package definitions,
failed reloads keep the last-known-good worker, and before-tool argument rewrites
are classified and displayed for approval before execution. See
[Agent Extensions](agent-extensions.md) for the manifest, API, policy, and
security boundary.

## Plans

Plans are Mitsuro's answer to complex, multi-step tasks. When a task is too large to tackle in a single pass -- a feature that touches multiple files across several subsystems, a refactor that needs to happen in stages, or an investigation that branches into several directions -- plans break the work into phases, each phase into numbered tasks, and each task into something the agent can execute and check off.

The plan system lives in `crates/mitsuro-core/src/plan/`.

### Plan Mode vs. Build Mode

Mitsuro sessions have a work mode: either **plan** or **build**. In plan mode, the agent focuses on decomposing the problem -- reading code, analyzing dependencies, and producing a structured plan. Editing tools are restricted during planning to prevent the agent from jumping into implementation before the plan is ready. Once the plan is approved, the session transitions to build mode, where the agent picks up tasks in order and starts executing.

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

Legacy plans that were stored as markdown files in `~/.mitsuro/plans/` are automatically migrated to the database on first access.

Plans track progress at multiple levels. Each phase knows how many of its tasks are complete. The plan itself reports total progress and auto-detects when all tasks are finished. A completed plan is no longer considered "active" and won't be injected into the agent's context.

### The /plan Command

Users interact with plans through the `/plan` slash command. It triggers plan mode, where the agent analyzes the request, breaks it down into phases and tasks, and presents the plan for review. Once approved, the session switches to build mode and the agent starts working through tasks sequentially, checking each one off as it goes. The plan state is injected into the agent's context on every turn, so the agent always knows where it stands and what's next.

## Skills

Skills are the simplest extensibility layer -- they're just files on disk that get loaded into the agent's context when invoked. A skill is a collection of domain-specific instructions, best practices, workflows, and examples written in Markdown with YAML frontmatter. When a skill is active, its content becomes part of what the agent knows and follows.

The skills system lives in `crates/mitsuro-core/src/skills/`.

### Skill Format

Each skill is a directory containing a `SKILL.md` file:

```
~/.mitsuro/skills/
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
author: mitsuro
tags:
  - rust
  - patterns
---

# Rust Patterns

## Error Handling

Always use `anyhow::Result` with `.context()` for functions that can fail...
```

The frontmatter follows the [Agent Skills](https://agentskills.io) contract. `name` and `description` are required. Names are 1–64 lowercase ASCII characters with single hyphen separators, cannot start/end with a hyphen, and must exactly match the directory name. Descriptions are 1–1024 characters. Standard optional fields (`license`, `compatibility`, `metadata`, `allowed-tools`, and `disable-model-invocation`) are supported; Mitsuro's existing `version`, `author`, and `tags` catalog fields remain compatible. `allowed-tools` is advisory and never bypasses Mitsuro's runtime tool governance.

Skills can include additional files beyond `SKILL.md`. The `load_skill_file` function lets you load any file within a skill's directory, with path traversal protection to prevent reading outside the skill boundary.

### Discovery and Loading

The `SkillsManager` scans compatible user roots (`~/.mitsuro/skills`, `~/.agents/skills`, `~/.pi/agent/skills`, `~/.claude/skills`, `~/.codex/skills`, and `~/.config/opencode/skills`) plus matching project roots (`.mitsuro`, `.agents`, `.pi`, `.claude`, `.codex`, and `.opencode`). Project discovery walks upward from the working directory through the git worktree boundary (or filesystem root outside a worktree). Pi roots additionally accept direct Markdown skills; package roots are scanned recursively. Package lifecycle code supplies its complete enabled snapshot through `set_package_roots`, so disable, update, and uninstall remove stale contributions immediately.

Precedence is deterministic: nearest project definitions override farther project definitions, project overrides user roots, user roots override packages, and native Mitsuro roots win ties within the same scope. Every rejected definition, invalid policy, and shadowed duplicate appears in the diagnostics catalog instead of disappearing into debug logs.

The manager keeps an in-memory catalog but fingerprints definitions and policy files on normal reads. Edits, additions, removals, and policy changes are detected without requiring a restart; `refresh()` remains available as an explicit force-rescan.

Per-skill policy is stored in `.mitsuro/skills-policy.json`. The nearest project
file wins field-by-field among project files, but project policy composes
monotonically with the user policy: a repository may change `allow` to `ask` or
`deny`, and may disable a skill, but it cannot re-enable or loosen a user-level
restriction:

```json
{
  "skills": {
    "rust-patterns": { "enabled": true, "permission": "allow" },
    "production-deploy": { "permission": "ask" }
  }
}
```

`allow` permits normal on-demand loading, `ask` permits model loading only in a supervised parent session (or direct user `/skill:name` invocation), and `deny` blocks loading. Disabled/denied skills remain visible in management UIs but are not advertised to the model. Loading a skill never changes the inherited permission mode for tools used afterward.

### Context Injection

When skills are available, bounded metadata (name, description, tags, origin, and policy) is included in the agent's system prompt. Full instructions remain deferred. The `/skills` TUI browser supports search, refresh, persistent enable/disable, policy cycling, and Enter-to-prepare an explicit `/skill:name` invocation. `/skill:name [request]` loads the selected instructions as an explicit user action, including user-only skills marked `disable-model-invocation`.

When a skill is model-activated, the governed skill tool returns the full
SKILL.md instructions as tool output. An explicit `/skill:name` invocation
embeds those instructions in the user's invocation. Only bounded skill metadata
is advertised in the system prompt. This is how you teach Mitsuro
project-specific workflows, coding standards, deployment procedures, or domain
knowledge without loading every instruction body up front.

### Creating Skills

You can create new skills through the `SkillsManager`:

```rust
manager.create_skill("my-workflow", "Custom deployment workflow")?;
```

This scaffolds a new skill directory with a template `SKILL.md` containing the frontmatter and placeholder sections for quick start, usage, and examples. Skills can also be deleted (global only -- project skills should be managed through version control) and reloaded after editing.

## How They Fit Together

These five layers serve different audiences and different moments in a Mitsuro session:

- **MCP** extends what tools the agent can call. It's about capability -- connecting to databases, APIs, file systems, or any service that speaks the MCP protocol.
- **Plugin packages** install, verify, update, permission, and remove complete bundles of capabilities.
- **Agent extensions** add worker-hosted agent behavior: tools, commands, events, and bounded context.
- **Plans** extend how work is organized. They're about structure -- breaking large tasks into trackable phases and tasks so nothing gets lost in a long session.
- **Skills** extend what the agent knows. They're about knowledge -- injecting domain expertise, coding standards, and workflows so the agent works the way you want it to.

Each layer does its part without stepping on the others.
