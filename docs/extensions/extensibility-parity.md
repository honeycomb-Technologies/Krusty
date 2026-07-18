# Extensibility parity: OpenCode, Pi, Codex, and Krusty

This audit compares the plugin/package, Agent Skills, hook/extension, and MCP
surfaces that are useful to a coding-agent harness. It is based on the current
official documentation for [OpenCode plugins](https://opencode.ai/docs/plugins/),
[OpenCode skills](https://opencode.ai/docs/skills/),
[OpenCode MCP](https://opencode.ai/docs/mcp-servers/),
[Pi extensions](https://pi.dev/docs/latest/extensions),
[Pi packages](https://pi.dev/docs/latest/packages),
[Pi skills](https://pi.dev/docs/latest/skills),
[Pi usage and MCP stance](https://pi.dev/docs/latest/usage), and the official Codex
documentation for [plugins](https://learn.chatgpt.com/docs/build-plugins),
[skills](https://learn.chatgpt.com/docs/build-skills),
[hooks](https://learn.chatgpt.com/docs/hooks), and
[MCP](https://learn.chatgpt.com/docs/extend/mcp).

## Result

Krusty reaches **10/10 functional coverage on Unix, Windows, and other supported
platforms** on the rubric below. That
means each major extensibility contract has an implemented, governed,
inspectable path through the shared runtime. It does not mean that Krusty's
public package catalog is already as large as the older ecosystems, or that
every competitor API method has a one-for-one alias.

Cross-platform distribution uses publisher-signed single-component artifacts
or authenticated multi-resource ZIP bundles. Unsigned local/npm package
snapshots remain a Unix-only convenience: non-Unix builds reject that path
before staging because the stable filesystem API cannot enforce the same
no-follow, stable-identity, and hard-link guarantees as the Unix snapshot
implementation. Signed ZIP bundles retain the complete multi-resource
distribution contract on those platforms without weakening snapshot safety.

| # | Required capability | Krusty evidence | Result |
|---|---|---|---|
| 1 | Installable, multi-resource distribution unit | One manifest can contribute TUI code, agent extensions, skills, MCP fragments, declarative hooks, and assets; Unix also supports unsigned npm/local snapshots, while signed ZIP releases and catalogs provide the full cross-platform path | Pass |
| 2 | Complete package lifecycle | Transactional staging, immutable managed snapshots, atomic lockfile replacement, enable/disable, pin/unpin, update, uninstall, and interrupted-install reconciliation | Pass |
| 3 | Executable agent extension API | Persistent JS/TS workers register tools, slash commands, state, context, and lifecycle observers; Pi aliases and OpenCode-style returned hook objects are accepted | Pass |
| 4 | Hooks on the canonical execution path | Agent interceptors normalize or block before classification, and any effective arguments are exactly what central approval displays; package command hooks support Codex/Claude-compatible `PreToolUse` and `PostToolUse` schemas | Pass |
| 5 | Standards-compatible skills | Agent Skills frontmatter validation, progressive disclosure, upward repo discovery, global/project/package roots, cross-harness paths, recursive resources, diagnostics, and `allow`/`ask`/`deny` policy | Pass |
| 6 | Full MCP connection model | Layered package/global/project config, stdio and Streamable HTTP, environment/header injection, timeouts, required/disabled state, reconnects, and safe non-replay of failed writes | Pass |
| 7 | MCP protocol and authentication depth | Tools, instructions, resources, templates, prompts, structured/multimodal results, list-change refresh, bearer tokens, OAuth 2.1 discovery, S256 PKCE, dynamic registration, refresh, logout, and secret-safe status | Pass |
| 8 | Explicit governance and trust | Full release-envelope signatures, HTTPS/loopback transport policy, path containment, script consent, exact-descriptor package grants, user-owned project trust, monotonic skill policy, exact effective-argument approvals, and fail-closed revocation | Pass |
| 9 | Operator UX and diagnostics | TUI catalog/install/update/permission flows, dynamic command autocomplete, skill diagnostics, server management APIs, contribution refresh, OAuth endpoints, and last-known-good extension reload | Pass |
| 10 | Integrated proof and maintainability | Unit coverage plus `extensibility_bundle` installs one real immutable package, loads its skill and MCP declaration, executes its hook, and exercises its command/tool leg when Bun is available; focused manager/client tests cover live MCP transport and capabilities | Pass |

The score is deliberately binary: a row passes only when the capability is
wired into the production control path, governed, documented, and covered by a
focused test. Merely parsing a manifest field does not count.

## Capability comparison

| Area | OpenCode | Pi | Codex | Krusty |
|---|---|---|---|---|
| Distribution | Local JS/TS and npm plugins installed with Bun | npm, git, and local packages bundle extensions, skills, prompts, and themes | Marketplace plugin with `.codex-plugin/plugin.json`; may bundle skills, hooks, apps, MCP, and assets | Cross-platform publisher-signed ZIP bundles and catalogs plus Unix npm/local snapshots; one immutable snapshot can bundle TUI, agent, skill, hook, MCP, and asset resources |
| Executable API | JS/TS hook object, events, custom tools, SDK client, Bun shell | Broad TypeScript API for tools, events, commands, sessions, providers, shortcuts, and custom TUI | Skills and declarative lifecycle hooks are the portable executable/workflow surfaces; MCP/apps add external actions | Persistent JS/TS agent runtime plus executable native/JS TUI runtimes; tools, commands, events, state, context, and before/after interception. Installable WASM TUI entries are descriptor-only today |
| Skills | On-demand Agent Skills with upward and cross-harness discovery plus wildcard policy | On-demand Agent Skills; deliberately lenient validation and cross-harness locations | Agent Skills with progressive disclosure, implicit/explicit activation, scripts, references, and plugin distribution | Strict Agent Skills validation, progressive disclosure, upward and cross-harness discovery, recursive resources, package roots, diagnostics, and per-skill policy |
| Hooks | Rich in-process event and dot-hook surface | Rich in-process event interception with UI access | Declarative command hooks from managed, user, project, session, and plugin sources | Persistent worker-hosted agent observers/interceptors plus bounded declarative package command hooks on the shared tool pipeline |
| MCP | Built-in local/remote servers, OAuth, enable/disable, and tool policy | Intentionally no built-in MCP; packages can add an implementation | Built-in stdio/HTTP, bearer/OAuth, server instructions, shared host configuration, and plugin MCP | Built-in stdio/HTTP, layered packages/config, tool policy, complete context surface, bearer/OAuth, server instructions, and management API |
| Security posture | General tool/skill permissions; plugins execute trusted local code | Project trust; packages explicitly run with full system access | Sandboxing/approval plus hook trust and workspace/plugin administration | User-owned project trust, release-envelope signatures, immutable installs, path/source validation, explicit script consent and grants, exact approvals, and a separate isolated Zed-compatible WASM editor/language host |
| Failure behavior | Sequential hooks and normal startup loading | Hot reload and extension error handling | Managed configuration and trusted hook loading | Transaction rollback, reconciliation, fail-closed hook replacement, process-tree cleanup, bounded output, per-server timeouts/reconnect, and last-known-good workers |

## Deliberate differences

Krusty targets equivalent outcomes without copying every API shape:

- Pi exposes its entire interactive UI, model/provider registry, and session
  tree through one very broad TypeScript API. Krusty splits those concerns:
  agent behavior lives in JS/TS workers, installable render extensions use
  native/JS hosts, the separate Zed-compatible editor/language ABI uses WASM,
  and provider/session policy remains in typed core contracts. The
  current JS agent API is therefore intentionally smaller than Pi's complete
  UI SDK.
- Codex plugins can point at hosted app connectors administered by a ChatGPT
  workspace. Krusty's analogous external-action boundary is MCP and its
  self-host server API; it does not reproduce the hosted ChatGPT marketplace
  or its installed ecosystem.
- OpenCode and Pi already have visible community package ecosystems. Krusty's
  catalog and source format are implemented, but catalog population and
  publisher adoption are product/ecosystem work rather than missing runtime
  contracts.
- Pi intentionally omits built-in MCP. Krusty keeps MCP in core because shared
  CLI, server, mobile, and desktop clients need one typed connection and
  governance boundary.
- Unsigned local/npm snapshots currently require Unix. Non-Unix installs reject
  that convenience path before staging while retaining both signed
  single-component manifests and signed multi-resource ZIP bundles, instead of
  weakening link and identity checks.

These are not hidden parity claims: **10/10 is the cross-platform engineering
feature-coverage score, not an ecosystem-size score and not a claim of
byte-compatible APIs.** Unix has the additional convenience of unsigned
local/npm snapshots; other platforms use authenticated release bundles for the
same multi-resource distribution outcome.

## Verification gate

Run the focused end-to-end proof first:

```bash
cargo test -p krusty-core --test extensibility_bundle -- --nocapture
```

Install Bun when running this proof to exercise the package's executable
command/tool leg. Without Bun, the same test still verifies immutable install,
skill loading, hook execution, and fail-closed MCP configuration loading; live
MCP behavior is covered by the focused manager/client tests in the workspace
suite.

Then run the repository release gate:

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all -- --check
cd apps/mobile && npx expo export --platform web
```

The score should be treated as regressed if the bundle fixture or any required
workspace gate fails.
