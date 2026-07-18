# Agent Extensions

Agent extensions are executable JavaScript or TypeScript modules that extend
Krusty's agent runtime. They can register model-callable tools, slash commands,
lifecycle observers, and bounded context for each turn. This is Krusty's
counterpart to Pi's extension API and OpenCode's plugin hooks; it is separate from Krusty's
Zed-compatible WASM extension ABI, which targets editor and language features.

## Discovery and precedence

Krusty loads extensions from three scopes:

1. package roots contributed by enabled plugin bundles;
2. the global `~/.krusty/extensions/agent/` root;
3. the project's `.krusty/extensions/` root.

Later scopes override an extension with the same ID, so trusted project code
has the highest precedence. Project roots are fail-closed until the user grants
trust from outside the repository:

```text
/extensions status
/extensions trust
/extensions revoke
```

The grant is keyed to the canonical project path and stored in the owner-only
global runtime trust store. A repository cannot authorize itself by changing
`.krusty/settings.json`; that file can only narrow an existing user grant.

A root may contain standalone `.js`/`.ts` files or
directories with `krusty-extension.json`. Invalid extensions produce structured
diagnostics without preventing other extensions from loading. When an edited
extension fails validation or startup, its last-known-good worker and tools
remain active.

After trust is granted, optional project restrictions live in
`.krusty/settings.json`:

```json
{
  "agentExtensions": {
    "enabled": true,
    "allow": ["team-*"],
    "deny": ["team-experimental"]
  }
}
```

Package extensions and hooks require a current explicit `process` permission
grant before their roots are activated.

## Manifest

```json
{
  "manifest_version": 1,
  "id": "release-tools",
  "name": "Release Tools",
  "version": "1.0.0",
  "description": "Release checks and commands",
  "entry": "index.ts",
  "enabled": true,
  "timeout_ms": 30000,
  "permissions": {
    "filesystem_read": true,
    "filesystem_write": false,
    "network": false,
    "process": false,
    "env": ["GITHUB_TOKEN"]
  }
}
```

The entry must remain inside its extension directory. Worker environments are
cleared, then receive only `PATH`, `HOME`, Krusty's scoped runtime variables,
and explicitly declared uppercase environment variables.

Agent extensions run as trusted local code in persistent Bun workers. The
manifest, user-owned project trust, and package grants make authority visible
and reviewable, but they are not represented as a JavaScript sandbox. The
isolated Zed-compatible WASM host exposes a separate editor/language ABI; it is
not a sandboxed drop-in for the agent-extension API.

## API

An extension exports a default setup function:

```ts
export default function setup(krusty) {
  krusty.registerTool({
    name: "release_status",
    description: "Inspect the current release state",
    parameters: {
      type: "object",
      properties: {
        channel: { type: "string" }
      },
      required: ["channel"]
    },
    async execute(args, context) {
      return { channel: args.channel, directory: context.working_dir };
    }
  });

  krusty.registerCommand("release", {
    description: "Prepare a release",
    async handler(argument, context) {
      return `Preparing ${argument || "the next release"}`;
    }
  });

  krusty.on("turn_complete", async (event, context) => {
    await krusty.state.set("lastTurn", event);
  });

  krusty.on("tool.execute.before", async (input, output) => {
    if (input.tool === "bash" && output.args.command.includes("deploy")) {
      return { block: true, reason: "Use the reviewed deploy command" };
    }
  });

  krusty.addContext(async (context) =>
    `Release worktree: ${context.working_dir}`
  );
}
```

Pi-style aliases (`tool`, `command`, `context`) are supported. An extension may
also return an OpenCode-style object containing `tools`, `commands`, `hooks`,
`events`, or dot-named hook functions. Tool names are preserved when available;
collisions are namespaced as `ext__<extension-id>__<tool-name>`. Tools still pass
through the central permission and approval policy. Slash commands appear in
TUI autocomplete and execute asynchronously.

Pi-style `tool_call` and OpenCode-style `tool.execute.before` interceptors can
block a call or normalize its arguments. Krusty's central safety and approval
hooks then evaluate the effective arguments, so an extension cannot bypass the
host policy by rewriting a call. `tool_result` and `tool.execute.after` are
observational and cannot rewrite the canonical result retained in history.

The context object deliberately contains stable, non-secret session metadata:
working/project directory, session ID, model, permission mode, and plan mode.
Context contributions are capped at 16 values and 32 KiB per turn. Lifecycle
observers receive canonical `LoopEvent` names and cannot create a second,
drift-prone event stream.

## Management API

The server exposes:

- `GET /api/extensions` for loaded status and diagnostics;
- `POST /api/extensions/reload` for a safe refresh;
- `GET /api/extensions/commands` for contributed slash commands.

These endpoints administer a process-wide runtime and are local
single-tenant-admin only. Tenant-scoped requests fail closed with `403` rather
than mutating or inspecting another trust domain's extension manager.

Plugin enable/disable, permission, update, and uninstall operations rebuild the
complete package-root snapshot, so removed contributions disappear without a
process restart.
