# AGENTS Guide: Krusty

## Purpose
Repository-level engineering guardrails for Krusty - an AI coding assistant with CLI/TUI, web server, ACP editor integration, and autonomous agent modes.

## AGENTS Strategy
This is the **only** AGENTS file in the repository. All module-specific invariants live in sections below rather than scattered across subdirectories.

## Core Architecture
- `crates/krusty-cli`: Terminal client and TUI runtime. Entry point with command parsing.
  - `src/main.rs`: CLI entry point, parses commands, starts ACP server or TUI with logging/setup.
  - `src/tui/`: Terminal UI module with blocks, handlers, state, themes, plugins.
- `crates/krusty-core`: Shared runtime library.
  - `src/ai/`: AI provider layer with multi-provider clients, streaming support.
  - `src/agent/`: Agent system with event handling, hooks, sub-agents, in-place compaction.
  - `src/acp/`: Agent Client Protocol server for editor integration.
  - `src/mcp/`: Model Context Protocol client manager.
  - `src/tools/`: Tool registry and built-in tool implementations (read, write, edit, bash, grep, glob, etc.).
  - `src/storage/`: SQLite persistence for sessions, plans, preferences, credentials.
  - `src/plan/`: Database-backed planning system.
  - `src/skills/`: Filesystem-based skills system.
  - `src/extensions/`: Zed-compatible WASM extension system.
  - `src/process/`: Background process registry/management.
  - `src/auth/`: OAuth/auth flows and token storage helpers.
  - `src/updater/`: Auto-updater for dev/release modes.
- `crates/krusty-server`: Self-host API plus embedded web bundle for external clients.
- `apps/mobile`: Expo app that serves as the primary mobile client and React-based web surface.
- `apps/desktop/shell`: Tauri wrapper around the Expo web build.

## Design Patterns
- **Event Bus**: AgentEventBus as central dispatcher.
- **Registry**: ToolRegistry, ThemeRegistry for centralized management.
- **Plugin Architecture**: Trait-based plugins (e.g., StreamBlock trait for renderable blocks).
- **Strategy/Polymorphism via traits**: Different providers, tool implementations.
- **Manager Pattern**: McpManager, PlanManager, SkillsManager, SessionManager.

## Cross-Cutting Standards
- Prefer clear module boundaries over cross-layer coupling.
- Write code that is composable, testable, and explicit about failure modes.
- Keep changes small and reversible.
- Avoid hidden side effects and global state sprawl.
- Error handling: anyhow + thiserror + custom error enum.
- Logging: tracing + tracing_subscriber.
- Async: tokio (no async-std).

## Crate Boundaries
- `krusty-cli`: terminal UX only. Do not re-implement core runtime logic here.
- `krusty-core`: shared runtime. All shared business logic lives here.
- `krusty-server`: HTTP API. Keep route handlers thin; push shared logic into core.
- Move shared logic to `krusty-core`; avoid duplication across CLI/server.

## Module-Specific Invariants

### AI Provider Layer (`crates/krusty-core/src/ai/`)
- Keep provider-specific quirks isolated from shared response models.
- Keep model-family prompt behavior in shared profiles; streaming and simple/conversation calls must build the same instruction layers.
- Keep provider request/stream normalization in the shared AI transform layer; avoid scattering provider patches across individual transport call-sites.
- Streaming behavior must be robust to partial/malformed provider events.
- Parser changes must preserve existing tool/thinking/message semantics.
- Keep curated direct-provider model catalogs aligned with product-supported IDs; when a provider adds fast or effort variants, update static fallbacks and dynamic filtering together.

### Tool System (`crates/krusty-core/src/tools/`)
- Tool argument parsing and error surfaces are user-facing contracts.
- Keep permission/approval semantics explicit and conservative.
- Avoid hidden filesystem/network side effects in tool implementations.
- Prefer structured tool result envelopes (`ok`, `data`, `error`, optional `warnings`/`metadata`) over ad-hoc plain strings.
- Delegated tool surfaces (subagents, remote MCP wrappers) must carry inherited governance metadata and enforce parent permission constraints.
- Keep filesystem path policy owned by `ToolContext`/registry logic; do not reintroduce duplicate standalone path-validation helpers.

### Storage (`crates/krusty-core/src/storage/`)
- Migration safety first: schema changes must be forward-only and tested.
- Keep read/write behavior explicit and transaction-aware.
- Keep interrupted-turn recovery state separate from canonical conversation history.
- Keep runtime traces compact and structured; persist summarized diagnostic payloads rather than raw stream dumps unless exact replay fidelity is required by design.
- Linked-session persistence must preserve parent ownership metadata so pinch/continuation flows do not escape multi-tenant boundaries.
- For push reliability changes, keep `database.rs`, `push_subscriptions.rs`, and `push_delivery_attempts.rs` aligned.
- Never log sensitive credentials.

### Agent Core (`crates/krusty-core/src/`)
- Keep subsystem contracts explicit between AI, tools, storage, plugins, and protocols.
- Prefer typed boundaries over ad-hoc JSON passing.
- Keep live in-place compaction as the default overflow path; `/pinch` and the pinch API route trigger manual compaction in the same session, not a session fork.
- Flush durable notes to the memory store before compaction summarization; persist compaction checkpoints/segments for `search_compaction_segments` recovery.
- On provider context-overflow/413 errors, compact once with `CompactionTrigger::Overflow` and retry the turn before surfacing a terminal provider error.
- Keep loop budgets and streaming timeouts explicit and shared across callers; do not hide behavioral caps inside transport layers.
- Keep UI-facing tool output separate from model-facing history retention; long raw tool output should not be preserved in conversation history unless the exact payload is still needed for the next turn.
- Keep agent tool approval/retry/result policy centralized in the agent control layer rather than embedding it ad hoc in transport or tool implementation code.
- Keep delegated execution (subagents, MCP, extensions, skills) on explicit inherited governance contracts for permission mode and turn budget; delegated paths must not silently bypass parent policy.
- Resolve delegated turn budgets and permission mode from the shared contract at execution time; do not duplicate drift-prone defaults across subagent or route surfaces.
- Keep plan lifecycle state canonical in core helpers; active-vs-archived plan resolution and effective work mode must not be re-derived independently in UI or server layers.
- Capture runtime observability from the canonical `LoopEvent` boundary; do not add drift-prone provider/tool/UI-specific trace streams for the same execution path.

### Server Routes (`crates/krusty-server/src/routes/`)
- Keep request/response shapes synchronized with CLI, web, and mobile clients.
- Validate and sanitize all user inputs before side effects.
- Preserve streaming route stability and backpressure behavior.
- Chat routes must honor persisted session model unless an explicit per-request override is provided.
- Session routes must keep `working_dir` as runtime source-of-truth and treat `target_branch` as optional session intent metadata.
- Session creation, read, pinch, and approval routes must preserve multi-tenant ownership end-to-end.
- Tool approval routes must stay contract-aligned with mobile/web notification surfaces by accepting explicit session-targeted approvals and surfacing delivery failures.
- Session presence routes must stay ownership-checked, server-authored, and stale-aware.
- Tool execution routes must pass the same governance context as orchestrated runs (permission mode, delegated turn budget, and extensibility managers).
- Direct tool execution must keep `working_dir` scoped to the same allowed workspace root as the rest of the server file/path surfaces.
- Knowledge routes must keep reports and memories on shared typed contracts with project/user scoping preserved end-to-end; avoid shadow stores or client-only promotion state.
- Chat streaming must keep a bounded queue with explicit lag signaling; never let a slow SSE client silently stall or redefine core loop semantics.
- Auth and credential routes for dynamic-model providers should refresh shared model catalogs eagerly.
- Push endpoints (`/push/*`) must stay aligned with mobile/web diagnostics and test-send flows.
- Port proxy endpoints (`/ports/*`) must remain localhost-scoped and deny recursive self-proxy loops.

### TUI (`crates/krusty-cli/src/tui/`)
- Protect frame-time performance and input responsiveness. Avoid heavy allocations in render/event hot paths.
- Keep streaming updates idempotent and visually stable.
- Keep stream backpressure policy, queue telemetry, and interruption recovery messaging explicit in shared TUI state/handlers.
- Handle partial stream events safely; never panic on malformed chunks.
- Drain bursty stream output incrementally; do not let a single stream monopolize a frame and starve input/render.
- Plan/task UI state must come from persisted plan lifecycle or explicit loop events, not heuristic parsing of assistant prose.
- Keep contrast/readability strong in both dense and sparse views. Theme additions must update registry wiring and defaults intentionally. Avoid hardcoding colors outside theme primitives.

### TUI Handlers (`crates/krusty-cli/src/tui/handlers/`)
- Keep keyboard/mouse/render handling deterministic.
- Keep session/tool side effects explicit and traceable.
- Keep model selection and quick-toggle flows on a shared handler path so persistence, auth rebinds, and recent-model state do not drift.

### Extensions & WIT (`crates/krusty-core/src/extensions/`, `wit/`)
- Treat WIT and extension host changes as ABI-sensitive.
- Keep manifest parsing strict and error messages actionable.
- Preserve compatibility rules across extension API versions.
- Version contract changes intentionally; avoid silent breaking renames.
- Keep generated/runtime expectations synchronized across crates.

### Plugins (`crates/krusty-core/src/plugins/`)
- Treat plugin install/update flows as security-sensitive.
- Verify trust and signature requirements before writing plugin artifacts.
- Keep lockfile and on-disk state transitions atomic and recoverable.
- Error messages must clearly distinguish trust failures from IO failures.

### Apps (`apps/`)
- Preserve strict separation between app surfaces and core runtime internals.
- Do not duplicate business logic that already exists in `krusty-core` or `krusty-server`.
- Keep desktop and Expo web behavior aligned where features overlap.
- Keep model-speed and reasoning controls driven by shared client state or server contracts rather than ad-hoc component-local mappings.
- Notification and Live Activity actions that mutate session state must carry explicit session context; never assume the currently focused chat is the correct target.
- Keep knowledge surfaces server-backed and shared across modes; reports and memories should behave like one project knowledge substrate rather than separate client-local feature stacks.
- Desktop shell is a host for the Expo web build, not a separate product surface. Keep desktop-specific code focused on windowing, permissions, startup wiring, and packaging.
- Treat Tauri permissions, deep links, and updater config as security-sensitive.

### CI/CD (`.github/`)
- Treat workflow changes as production-impacting.
- Keep CI reproducible and aligned with local developer commands.
- Avoid secret leakage in logs and artifact names.
- Keep release workflows backward-compatible for existing tags and packaging paths.

### Packaging (`aur/`)
- Keep PKGBUILD/install scripts aligned with released artifacts.
- Avoid distro-specific assumptions in core runtime logic.
- Any packaging change should preserve clean install/upgrade paths.

### Git Hooks (`.githooks/`)
- Keep hooks deterministic and quick.
- Prefer local checks; avoid network-dependent commands.
- Fail with clear, actionable error messages.
- Any new hook must not duplicate CI logic unnecessarily.

## Default Dev Workflow
- Build and run current local code only; do not require `git pull` for day-to-day refinement.
- Rust builds inherit `TMPDIR` from `.cargo/config.toml`, pointing rustc temp files at the workspace `target/` directory instead of `/tmp`.
- **Rust backend**: `cargo run -p krusty` from repo root.
- **Expo web dev server**: `cd apps/mobile && npx expo start --web --port 5173`.
- Do active UI/web iteration at `http://localhost:5173` while the backend runs separately.
- Frontend edits hot-reload automatically; Rust backend edits require a restart.
- **ACP mode** (editor integration): `krusty acp`

## Release Integration Workflow
- Keep `main` as the last accepted release baseline. Cross-surface work intended for the next coordinated release belongs on the current dated `codex/release-staging-YYYYMMDD` branch.
- At the start of any Krusty conversation that may change or release the product, inspect the active branch, every registered worktree, staged and unstaged changes, and the current release-staging branch before editing.
- Treat each worktree as an independent Git index. Commit a logical change on its source branch, then deliberately cherry-pick or otherwise reconcile it into release staging; files cannot be staged across worktrees through one shared index.
- Do not merge preservation, archive, or old rollup branches wholesale. Audit their commits against release staging and integrate only changes that are genuinely missing and still intended.
- Preserve concurrent dirty work. When a mixed dirty tree must be captured, place it on release staging as an explicit staging snapshot before reorganizing or squashing it for release.
- Build private Honey previews from the exact release-staging commit. A detached preview mirror is not a source branch and must not become the release authority.
- Do not push, merge to `main`, tag, publish artifacts, restart the production service, or issue a public release until release staging passes the required validation below and the user explicitly approves the release.
- One coordinated release may contain multiple logical commits. Prefer an auditable staging history over one unreviewable cross-worktree commit; squash only at the final release boundary when requested.

## Required Validation
All code must pass before commit:
```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
cargo fmt --all
```
Web/mobile validation:
```bash
cd apps/mobile && npx expo export --platform web
```

## Dependencies
Key runtime dependencies (check `Cargo.toml` for versions):
- tokio (async runtime)
- anyhow, thiserror (error handling)
- serde, serde_json, serde_yaml, toml (serialization)
- tracing, tracing-subscriber (logging)
- ratatui (TUI framework)
- rusqlite (database)
- reqwest (HTTP client)
- wasmtime (WASM extensions)
