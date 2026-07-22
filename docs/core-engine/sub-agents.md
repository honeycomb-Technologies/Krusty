# Sub-Agents and Teams

Krusty is not a single agent. When a task is large enough, ambiguous enough, or parallelizable enough, the main agent spawns sub-agents to handle parts of the work. These sub-agents are lightweight, policy-bounded, and disposable. They inherit explicit tool, permission, path, and turn-budget governance, but that governance is not an operating-system sandbox; any delegated Bash access still has the authority of the server account. They do their job, report back, and disappear. This document explains how that delegation works, what kinds of sub-agents exist, how teams coordinate, and what keeps everything safe.

## Why Sub-Agents Exist

A single agent loop works well for straightforward conversations. But some tasks don't fit that pattern. When you say "explore the entire codebase and tell me how the plugin system works," one agent reading files sequentially would be slow and expensive. When you say "build a game with separate modules for rendering, physics, and input," one agent writing all the code serially would be inefficient.

Sub-agents solve both problems. For exploration, Krusty fans out multiple read-only agents that each investigate a different part of the codebase in parallel, then merges their findings. For building, it assigns each component to a separate agent that writes its own files concurrently. The main agent acts as coordinator: it decides what to delegate, spawns the workers, collects results, and synthesizes a response.

## The Sub-Agent System

At the core of delegation is `SubAgentPool`, defined in `crates/krusty-core/src/agent/subagent/mod.rs`. A pool manages concurrent execution of multiple sub-agent tasks through `AgentScheduler`, an actor-owned adaptive queue. It takes an AI client, a cancellation token, an optional user concurrency ceiling, and a stagger delay, then spawns tasks as independent tokio tasks. There is no product-wide fixed four-agent limit: the scheduler derives an initial target from host parallelism, grows under healthy backlog, and backs off under provider pressure.

Each sub-agent gets its own conversation with the AI model. It receives a system prompt, a task prompt, and a filtered set of tools. It runs the same agentic loop as the main agent -- call the model, execute any tool requests, feed results back, repeat -- but in a governed scope. The loop lives in `crates/krusty-core/src/agent/subagent/execution/runtime.rs` as `execute_agent_loop`, a generic function parameterized over an `AgentConfig` trait that abstracts the differences between agent types.

A sub-agent task is described by `SubAgentTask`: a struct carrying a semantic task ID and name, an optional `AgentIdentity`, the task prompt, a working directory, an optional delegation policy, and an optional turn budget. Identity deliberately separates the canonical runtime path from the playful display name. The root is `Krusty the Krab`; children receive deterministic creature names such as `Horseshoe Crab`, `Mantis Shrimp`, or `Nautilus` while retaining task labels such as `Honey audit`. The task does not specify which model to use -- that's resolved by the pool based on the user's current model selection, making the system provider-agnostic.

Results come back as `SubAgentResult`, which includes whether the task succeeded, the agent's final output, a list of files it examined, how many turns it took, wall-clock duration, any errors, and any policy violations it triggered.

## The Four Agent Types

The unified `agent` tool (`crates/krusty-core/src/tools/implementations/agent/mod.rs`) is the primary way the main agent spawns sub-agents. It accepts an `agent_type` parameter that selects one of four flavors:

**Explore** agents investigate the codebase. They are read-only -- they can use glob, grep, read, and list, but cannot write files, run shell commands, or modify anything. They get a focused system prompt instructing them to gather evidence, follow references across modules, and report findings in a structured format with specific file paths and line references. Explore agents use a fast, inexpensive model when available (Haiku on Anthropic, GPT-4.1 mini on OpenAI) to keep costs low. They also inherit context from the parent conversation, so they understand what the user has been working on.

**Plan** agents generate implementation plans. Like explore agents, they are read-only, but they use the user's full model (not a downgraded one) and receive a fresh context without the parent conversation. Their job is to produce step-by-step plans with critical files, trade-offs, and dependency analysis.

**Verify** agents run tests, builds, linters, and other validation commands. They are read-only for file access but have bash access enabled, so they can execute shell commands like `cargo test` or `npm run lint`. They output a structured verdict: PASS, FAIL, or PARTIAL, with details.

**Build** agents write code. They get the full suite of tools -- glob, grep, read, write, edit, and bash -- plus a special `register_interface` tool for cross-agent coordination. Build agents are the only sub-agent type that can modify the filesystem. When multiple builders run in parallel, they use a shared build context with file-level locking to prevent conflicts.

## Explore: Codebase Investigation

The unified agent tool's `explore` mode launches a focused read-only investigator for a scoped prompt such as "explore the auth module." Broader investigations are expressed as multiple independent `agent` calls, which the parent orchestrator may issue concurrently. Each run remains separately governed and returns structured evidence to the parent instead of relying on an obsolete standalone explore implementation.

## Build: Parallel Code Implementation

When the agent tool receives `agent_type: "build"` with a `components` array, it creates one builder agent per component and runs them through a `SubAgentPool`. Each builder gets a detailed prompt explaining which component it owns, what the overall goal is, who the other builders are, and how to coordinate.

Builders share a `SharedBuildContext` that provides three coordination mechanisms:

1. **File locking.** Before writing or editing a file, a builder must acquire a lock through an RAII guard (`FileLockGuard`). If another builder holds the lock, it retries with exponential backoff (50ms, 100ms, 200ms, up to 10 attempts). The guard automatically releases the lock when dropped, preventing leaks from early returns or panics.

2. **Interface registration.** After creating a module, a builder can call `register_interface` to advertise its exports -- function names, class names, file paths, and a description. Other builders see these interfaces in their system prompt (which is refreshed every turn) and can import from them.

3. **Line tracking.** The build context records lines added and removed across all builders, providing aggregate statistics when the build completes.

Build agents are submitted eagerly to the adaptive scheduler. `max_concurrency` is an optional user ceiling, not a hidden default cap. Omitting it lets the scheduler choose a host-aware starting target, grow when queued work completes healthily, and reduce pressure when the provider returns rate-limit, overload, service-unavailable, or timeout signals.

## The Team System

Beyond the agent tool's one-shot delegation, Krusty has a persistent team system for longer-running coordination. The `TeamManager` (`crates/krusty-core/src/agent/autonomy/team/manager.rs`) maintains a pool of named teammates that run as background loops, polling a SQLite task queue for work.

Each teammate is defined by a `TeammateConfig` with a semantic name, a role, and an optional turn budget. `TeamManager` assigns a deterministic creature identity for display and keeps the semantic name for task ownership and cancellation. There are three roles:

- **Builder** -- Can write files but cannot run shell commands. Gets `SubagentBuild` delegation surface.
- **Reviewer** -- Read-only file access with bash enabled. Gets `SubagentVerify` delegation surface. Intended for code review.
- **Tester** -- Same permissions as Reviewer. Intended for running test suites and validation.

Teammates run according to the parent session's permission mode. The manager spawns them with `spawn_teammate`, and each one starts a background loop that polls the `autonomous_tasks` SQLite table every 5 seconds for unclaimed work. When a task appears, the teammate claims it atomically (using a SQL UPDATE ... RETURNING pattern to avoid races), executes it through the standard sub-agent loop, and records the result back to the database. A teammate may tighten the inherited turn budget but cannot relax it, and a supervised parent can never produce an autonomous child. If no tasks arrive for 30 seconds, the teammate exits gracefully.

The manager provides lifecycle controls: `list_teammates` to check status, `cancel_teammate` to stop one, and `cancel_all` to shut everything down. Teammates carry their own `CancellationToken`, so cancellation is cooperative and immediate. The `Drop` implementation on `TeamManager` ensures all teammates are cancelled if the manager is dropped unexpectedly.

## The Auto-Classifier

When Krusty operates in autonomous mode (Mako), there is no human in the loop to approve tool calls. The auto-classifier (`crates/krusty-core/src/agent/autonomy/auto_classifier.rs`) fills that gap. It is a `PreToolHook` that runs before every tool execution when the permission mode is `Autonomous`.

The classifier works in two stages:

First, it checks an allowlist of inherently safe tools -- read, grep, glob, list, memory, and others that cannot modify the system. If the tool is on the list, it passes immediately without any AI call.

Deterministic local policy handles ordinary reads, in-workspace edit tools, common project build/test commands, and delegated calls that inherit the parent's governance contract. Obvious unsafe payloads are blocked locally. Only an operation that remains ambiguous after those checks invokes the fast AI classifier (stage 1) with a 64-token budget.

Stage 2 uses a larger token budget (4096 tokens) only after an ambiguous or blocking stage-1 verdict. If stage 2 says ALLOW, the operation proceeds. If it says BLOCK, remains ambiguous, or fails, the operation defaults to deny.

Every decision is emitted as a `ClassifierDecision` event with the tool name, the verdict, the reason, and which stage made the call. This provides a full audit trail.

## Governance: Permission Inheritance

Sub-agents do not get to decide their own permissions. Every sub-agent inherits a `DelegationPolicy` from its parent context. This policy specifies:

- **Surface** -- What kind of delegation this is (explore, build, plan, verify). This determines which tools are available.
- **Permission mode** -- Whether the sub-agent runs supervised or autonomous. Inherited from the parent session.
- **Turn budget** -- An optional explicit ceiling on conversation turns. The budget cascades: a parent's `subagent_max_turns` setting flows into each task's policy, but an absent setting remains unlimited. Semantic progress guards, cancellation, permissions, and provider limits still govern the run.
- **Read-only flag** -- Whether the agent can only read or can also write. Explore and plan agents are always read-only.
- **Bash access** -- Whether shell commands are available. Only verify agents and testers get this by default.

The policy is enforced at tool execution time. Before a sub-agent runs any tool, `policy.authorize_tool()` checks whether the requested tool is permitted. If not, the call returns an error rather than executing.

## Resource Management

Concurrent agents are bounded at multiple levels:

**Adaptive queued concurrency.** `AgentScheduler` owns admission state and queues excess work. The default target is based on host parallelism and begins above four on ordinary development machines. Sustained healthy backlog ramps the target gradually. HTTP 429/503/529, overload, timeout, and `Retry-After` signals halve the target, pause new starts for a bounded cooldown, and then permit gradual recovery. A reserved control lane keeps the root coordinator responsive, weighted session selection prevents one swarm from monopolizing starts, shared-write partitions avoid conflicting admission, and cancellation immediately releases capacity. An explicit `max_concurrency` remains available as a user safety ceiling.

**Staggered spawning.** Agents are not all launched simultaneously. There is a configurable delay between spawns (default 100ms, higher for rate-sensitive providers like MiniMax at 600ms). This prevents burst traffic that could trigger provider rate limits.

**Turn budgets.** Sub-agents are unlimited by default. A parent or task may set an explicit finite ceiling when it is a real resource policy; when that ceiling is exhausted, the agent stops with a typed budget-exhaustion reason. Loop detection is handled separately by the semantic progress ledger.

**Cancellation tokens.** Every sub-agent receives a child cancellation token from its parent. If the parent is cancelled (Ctrl+C, Mako tick interrupted, team manager shutdown), cancellation propagates to all children immediately. Each turn of the agent loop checks the token before proceeding.

**Panic recovery.** Sub-agents run as tokio tasks. If a task panics, the `JoinHandle` catches it and converts it to a failed `SubAgentResult` with the panic message. The pool continues collecting results from the remaining agents.

**Message pruning.** Long-running agents accumulate conversation history. To prevent context window exhaustion, the agent loop prunes messages when they exceed 100 entries, keeping the system prompt and the most recent exchanges while compressing older tool results.

**Cleanup hooks.** The `AgentConfig` trait includes a `cleanup()` method called when any agent exits, whether normally or due to cancellation. For builders, this releases all file locks held by that agent, ensuring no stale locks persist.

Together, these mechanisms ensure that sub-agents are adaptively bounded by current capacity, bounded in duration and context size, governed by their parent, and cleaned up reliably when they finish or fail.
