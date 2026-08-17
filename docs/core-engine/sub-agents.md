# Sub-Agents and Teams

Mitsuro is not a single agent. When a task is large enough, ambiguous enough, or parallelizable enough, the main agent spawns sub-agents to handle parts of the work. These sub-agents are lightweight, policy-bounded, and disposable. They inherit explicit tool, permission, path, and turn-budget governance, but that governance is not an operating-system sandbox; any delegated Bash access still has the authority of the server account. They do their job, report back, and disappear. This document explains how that delegation works, what kinds of sub-agents exist, how teams coordinate, and what keeps everything safe.

## Why Sub-Agents Exist

A single agent loop works well for straightforward conversations. But some tasks don't fit that pattern. When you say "explore the entire codebase and tell me how the plugin system works," one agent reading files sequentially would be slow and expensive. When you say "build a game with separate modules for rendering, physics, and input," one agent writing all the code serially would be inefficient.

Sub-agents solve both problems. For exploration, Mitsuro fans out multiple read-only agents that each investigate a different part of the codebase in parallel, then merges their findings. For building, it assigns each component to a separate agent that writes its own files concurrently. The main agent acts as coordinator: it decides what to delegate, spawns the workers, collects results, and synthesizes a response.

## The Sub-Agent System

The session-level authority is `DelegationCoordinator`, defined in `crates/mitsuro-core/src/agent/delegation.rs`. Before execution begins it materializes one immutable delegation group and all logical tasks in SQLite. The group owns execution mode, completion/failure policy, inherited permissions, the delegated turn budget, the per-group parallelism ceiling, writer mode, task attempts, leases, synthesis ownership, and parent continuation identity. `SubAgentPool` remains the in-process worker launcher, but it no longer owns the operation lifecycle or global capacity policy.

Admission has two layers. `AgentScheduler` is the low-latency process-wide adaptive queue. A SQLite capacity authority is the hard cross-process ceiling for hosts that share the database. Capacity and cooldown are tracked by model domain, while workspace writer partitions prevent unsafe overlap. The current coordinator key is the resolved model identifier; refining it to provider, credential pool, endpoint, and model is a follow-up transport-contract change. There is no fixed four-agent product limit: host capacity starts from available parallelism, grows under healthy demand, and backs off under provider pressure. The immutable group contract applies the narrower per-operation ceiling.

Lease maintenance is shared rather than task-local. One renewal actor per normalized database path owns a reusable SQLite connection, coalesces due task/capacity/synthesis renewals into one immediate transaction, and cancels only the exact owner that loses its fence. A transient busy database is retried within the last-confirmed lease lifetime; an explicit owner mismatch cancels immediately. Registrations remain live through task or synthesis completion CAS, closing the window where completed side effects could otherwise be replayed after a lease expired.

Foreground delegation suspends the parent loop and returns the aggregate into that same run. Detached delegation persists one aggregate continuation identity; a terminal group may queue and promote it exactly once. Group and task transitions also append session-scoped events with a monotonic cursor. HTTP/SSE, the Rust client, Expo/Tauri, GPUI, TUI, CLI, and ACP all project the same durable snapshot and replay stream, so reconnect shows real queued/running/terminal parallel state rather than reconstructing it from prose or process-local progress. Event kinds are an extensible string protocol while group and task state machines remain closed.

Each sub-agent gets its own conversation with the AI model. It receives a system prompt, a task prompt, and a filtered set of tools. Its control flow has the same basic provider/tool continuation shape as the parent -- call the model, execute accepted tool requests, retain governed results, and continue -- but it is **not** the parent streaming orchestrator. It runs through the separately governed, non-streaming `execute_agent_loop` mini-kernel in `crates/mitsuro-core/src/agent/subagent/execution/runtime.rs`, parameterized over an `AgentConfig` trait for the different agent types.

That boundary is intentional and explicit. Delegated workers reuse the parent's exact `AiClient`, model identity, semantic `ProgressLedger`, history shaping, cancellation tree, process registry, and inherited permission/path/tool/turn ceiling. They do not consume `RunSpec` or emit a second provider-specific trace stream. The coordinator emits the canonical durable group/task event stream, while live progress is only a presentation optimization.

Detached Chat/Code tasks persist a versioned, bounded executor envelope containing reconstruction metadata and an objective digest, never the parent transcript or raw tool output. A new host can reacquire non-build tasks under task and synthesis leases using freshly resolved credentials. For an isolated Build batch, terminal successful task worktrees can be restored and synthesized idempotently under replay, synthesis, and repository-owner fences. An unfinished writer is never replayed in its possibly dirty retained worktree: that task fails closed, its partial edits remain available for inspection, and only terminal successful sibling patches are eligible for integration. Foreground tasks, legacy/malformed envelopes, ambiguous model identity, shared-writer builds, and mixed writer-mode builds fail closed.

A sub-agent task is described by `SubAgentTask`: a struct carrying a semantic task ID and name, an optional `AgentIdentity`, the task prompt, a working directory, an optional delegation policy, and an optional turn budget. Identity separates the canonical runtime path from the display name. The root identity is Agent; children receive deterministic names such as `Hive Worker 01` while retaining task labels such as `Honey audit`. The task does not specify which model to use -- that is resolved by the pool from the user's current model selection, keeping the system provider-agnostic.

Results come back as `SubAgentResult`, which includes whether the task succeeded, the agent's final output, a list of files it examined, how many turns it took, wall-clock duration, any errors, and any policy violations it triggered. The `agent` tool also records delegated-run lifecycle metadata and the final structured artifact when session storage is available.

## The Four Agent Types

The unified `agent` tool (`crates/mitsuro-core/src/tools/implementations/agent/mod.rs`) is the primary way the main agent spawns sub-agents. It accepts an `agent_type` parameter that selects one of four flavors:

**Explore** agents investigate the codebase. They are read-only -- they can use glob, grep, read, and list, but cannot write files, run shell commands, or modify anything. They get a focused system prompt instructing them to gather evidence, follow references across modules, and report findings in a structured format with specific file paths and line references. Explore agents inherit the parent run's exact resolved model and client. A future fast-model substitution must resolve a separate exact provider, authentication, and API runtime; it cannot change only the model slug. They also inherit context from the parent conversation, so they understand what the user has been working on.

**Plan** agents generate implementation plans. Like explore agents, they are read-only, but they use the user's full model (not a downgraded one) and receive a fresh context without the parent conversation. Their job is to produce step-by-step plans with critical files, trade-offs, and dependency analysis.

**Verify** agents run tests, builds, linters, and other validation commands. They are read-only for file access but have bash access enabled, so they can execute shell commands like `cargo test` or `npm run lint`. They output a structured verdict: PASS, FAIL, or PARTIAL, with details.

**Build** agents write code. They get the full suite of tools -- glob, grep, read, write, edit, and bash -- plus a special `register_interface` tool for cross-agent coordination. Build agents are the only sub-agent type that can modify the filesystem. Parallel builders execute in per-attempt Git worktrees rooted in a UID-private guarded batch directory. They never concurrently edit the authoritative workspace.

## Explore: Codebase Investigation

The unified agent tool's `explore` mode launches a focused read-only investigator for a scoped prompt such as "explore the auth module." Broader investigations are expressed as multiple independent `agent` calls, which the parent orchestrator may issue concurrently. Each run remains separately governed and returns structured evidence to the parent instead of relying on an obsolete standalone explore implementation.

## Build: Parallel Code Implementation

When the agent tool receives `agent_type: "build"` with a `components` array, it creates one builder agent per component and runs them through a `SubAgentPool`. Each builder gets a detailed prompt explaining which component it owns, what the overall goal is, who the other builders are, and how to coordinate.

Builders share a `SharedBuildContext` for advisory coordination inside the batch:

1. **File ownership metadata.** Builders advertise the files and interfaces they are changing. This improves coordination, but filesystem safety comes from isolated worktrees rather than an in-memory lock.

2. **Interface registration.** After creating a module, a builder can call `register_interface` to advertise its exports -- function names, class names, file paths, and a description. Other builders see these interfaces in their system prompt (which is refreshed every turn) and can import from them.

3. **Line tracking.** The build context records lines added and removed across all builders, providing aggregate statistics when the build completes.

Build agents are submitted eagerly to the coordinator. `max_concurrency` is an optional user ceiling, not a hidden default cap. After every task settles, one synthesis lease owns integration. The worktree diffs are bounded, checked as one deterministic combined patch, and applied atomically. A conflict leaves the authoritative workspace unchanged and retains the recovery worktrees.

## Hive Delegation

Hive does not use the obsolete rigid `TeamManager`/`TeammateRole` prototype under `agent/autonomy/team`; that module is intentionally not compiled. The live path is Honey's Hive runner → `TickEngine` → the shared `AgenticOrchestrator` and tool registry → the unified `agent` tool. Hive therefore receives the same coordinator, immutable governance, scheduler, lifecycle, and isolated build behavior as Chat and Code instead of maintaining a shadow worker architecture.

Hive batching is model-directed: an autonomous tick requests an explicit multi-component build when work is genuinely separable. There is no second scheduler over the legacy `autonomous_tasks` prototype. Server startup recovery can adopt ordinary detached Chat/Code executor envelopes and can finish terminal isolated-Build synthesis, but Hive groups remain owned by the live Hive runtime. Unfinished writer execution cannot resume until recovery has a distinct durable per-attempt worktree and execution fence; it remains fail-closed with the retained worktree available for inspection.

## The Auto-Classifier

When Mitsuro operates in autonomous mode (Hive), there is no human in the loop to approve tool calls. The auto-classifier (`crates/mitsuro-core/src/agent/autonomy/auto_classifier.rs`) fills that gap. It is a `PreToolHook` that runs before every tool execution when the permission mode is `Autonomous`.

The classifier works in two stages:

First, it checks an allowlist of inherently safe tools -- read, grep, glob, list, memory, and others that cannot modify the system. If the tool is on the list, it passes immediately without any AI call.

Deterministic local policy handles ordinary reads, in-workspace edit tools, common project build/test commands, and delegated calls that inherit the parent's governance contract. Obvious unsafe payloads are blocked locally. Only an operation that remains ambiguous after those checks invokes the fast AI classifier (stage 1) with a 64-token budget.

Stage 2 uses a larger token budget (4096 tokens) only after an ambiguous or blocking stage-1 verdict. If stage 2 says ALLOW, the operation proceeds. If it says BLOCK, remains ambiguous, or fails, the operation defaults to deny.

Every decision is emitted as a `ClassifierDecision` event with the tool name, the verdict, the reason, and which stage made the call. This provides a full audit trail.

## Governance: Permission Inheritance

Sub-agents do not get to decide their own permissions. Every sub-agent inherits a `DelegationPolicy` from its parent context. This policy specifies:

- **Surface** -- What kind of delegation this is (explore, build, plan, verify). This determines which tools are available.
- **Permission mode** -- Whether the sub-agent runs supervised or autonomous. Inherited from the parent session.
- **Turn budget** -- An optional explicit ceiling on conversation turns. The budget cascades from the parent's `subagent_max_turns`; durable unified-agent groups resolve an absent setting to 20 and apply that same value at execution time. A non-session compatibility caller has no durable contract and may remain unbounded. Semantic progress guards, cancellation, permissions, and provider limits still govern the run.
- **Read-only flag** -- Whether the agent can only read or can also write. Explore and plan agents are always read-only.
- **Bash access** -- Whether shell commands are available. Only verify agents and testers get this by default.
- **Exact tool ceiling** -- If the parent run has an explicit execution allowlist, the child receives its intersection with the agent-type surface. `None` means ordinary governed defaults; an explicit empty set remains tool-free. A wrapper such as `tool_search` must have both the wrapper and its effective target in scope.

The policy is enforced at tool execution time. Before a sub-agent runs any tool, `policy.authorize_tool()` checks whether the requested tool is permitted. If not, the call returns an error rather than executing.

## Resource Management

Concurrent agents are bounded at multiple levels:

**Adaptive queued concurrency.** `AgentScheduler` owns low-latency admission within one process. The SQLite capacity authority fences the same host/model/writer limits across processes. The default target is based on host parallelism and begins above four on ordinary development machines. Sustained healthy backlog ramps the target gradually. HTTP 429/503/529, overload, timeout, and `Retry-After` signals reduce the affected model domain and start a bounded cooldown without pausing unrelated domains. A reserved control lane keeps the root coordinator responsive, weighted session selection prevents one swarm from monopolizing starts, writer partitions avoid conflicts, and cancellation releases capacity. An explicit `max_concurrency` remains available as a user safety ceiling.

**Eager materialization, governed admission.** Logical tasks are created and shown to clients immediately. Provider calls start only after process and durable admission succeed, so the UI can display genuine parallel queued/running state without relying on artificial spawn delays.

**Turn budgets.** Durable unified-agent paths resolve an omitted request ceiling to the persisted 20-turn default, and the runtime task applies that exact same value. An explicit parent/task ceiling replaces it but may never exceed inherited governance. Legacy non-session callers can remain unbounded only when no durable group contract exists. Budget exhaustion is typed separately from semantic loop detection.

**Cancellation tokens.** Every sub-agent receives a child cancellation token from its parent. If the parent is cancelled (Ctrl+C, Hive tick interrupted, team manager shutdown), cancellation propagates to all children immediately. Each turn of the agent loop checks the token before proceeding.

**Panic recovery.** Sub-agents run as tokio tasks. If a task panics, the `JoinHandle` catches it and converts it to a failed `SubAgentResult` with the panic message. The pool continues collecting results from the remaining agents.

**Message pruning.** Long-running agents accumulate conversation history. To prevent context window exhaustion, the agent loop prunes messages when they exceed 100 entries, keeping the system prompt and the most recent exchanges while compressing older tool results.

**Cleanup hooks.** The `AgentConfig` trait includes a `cleanup()` method called when any agent exits, whether normally or due to cancellation. For builders, this releases all file locks held by that agent, ensuring no stale locks persist.

Together, these mechanisms ensure that sub-agents are adaptively bounded by current capacity, bounded by their persisted resource policy, governed by their parent, and cleaned up reliably when they finish or fail. The remaining mini-kernel, unfinished-writer replay, shared-writer recovery, and Hive recovery boundaries above must stay visible in design and release claims.
