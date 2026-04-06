# Sub-Agents and Teams

Krusty is not a single agent. When a task is large enough, ambiguous enough, or parallelizable enough, the main agent spawns sub-agents to handle parts of the work. These sub-agents are lightweight, sandboxed, and disposable. They do their job, report back, and disappear. This document explains how that delegation works, what kinds of sub-agents exist, how teams coordinate, and what keeps everything safe.

## Why Sub-Agents Exist

A single agent loop works well for straightforward conversations. But some tasks don't fit that pattern. When you say "explore the entire codebase and tell me how the plugin system works," one agent reading files sequentially would be slow and expensive. When you say "build a game with separate modules for rendering, physics, and input," one agent writing all the code serially would be inefficient.

Sub-agents solve both problems. For exploration, Krusty fans out multiple read-only agents that each investigate a different part of the codebase in parallel, then merges their findings. For building, it assigns each component to a separate agent that writes its own files concurrently. The main agent acts as coordinator: it decides what to delegate, spawns the workers, collects results, and synthesizes a response.

## The Sub-Agent System

At the core of delegation is `SubAgentPool`, defined in `crates/krusty-core/src/agent/subagent/mod.rs`. A pool manages concurrent execution of multiple sub-agent tasks. It takes an AI client, a cancellation token, and configuration for concurrency limits and stagger delays, then spawns tasks as independent tokio tasks.

Each sub-agent gets its own conversation with the AI model. It receives a system prompt, a task prompt, and a filtered set of tools. It runs the same agentic loop as the main agent -- call the model, execute any tool requests, feed results back, repeat -- but in a confined scope. The loop lives in `execution.rs` as `execute_agent_loop`, a generic function parameterized over an `AgentConfig` trait that abstracts the differences between agent types.

A sub-agent task is described by `SubAgentTask`: a struct carrying an ID, a display name, the task prompt, a working directory, an optional delegation policy, and an optional turn budget. The task does not specify which model to use -- that's resolved by the pool based on the user's current model selection, making the system provider-agnostic.

Results come back as `SubAgentResult`, which includes whether the task succeeded, the agent's final output, a list of files it examined, how many turns it took, wall-clock duration, any errors, and any policy violations it triggered.

## The Four Agent Types

The unified `agent` tool (`crates/krusty-core/src/tools/implementations/agent.rs`) is the primary way the main agent spawns sub-agents. It accepts an `agent_type` parameter that selects one of four flavors:

**Explore** agents investigate the codebase. They are read-only -- they can use glob, grep, read, and list, but cannot write files, run shell commands, or modify anything. They get a focused system prompt instructing them to gather evidence, follow references across modules, and report findings in a structured format with specific file paths and line references. Explore agents use a fast, inexpensive model when available (Haiku on Anthropic, GPT-4.1 mini on OpenAI) to keep costs low. They also inherit context from the parent conversation, so they understand what the user has been working on.

**Plan** agents generate implementation plans. Like explore agents, they are read-only, but they use the user's full model (not a downgraded one) and receive a fresh context without the parent conversation. Their job is to produce step-by-step plans with critical files, trade-offs, and dependency analysis.

**Verify** agents run tests, builds, linters, and other validation commands. They are read-only for file access but have bash access enabled, so they can execute shell commands like `cargo test` or `npm run lint`. They output a structured verdict: PASS, FAIL, or PARTIAL, with details.

**Build** agents write code. They get the full suite of tools -- glob, grep, read, write, edit, and bash -- plus a special `register_interface` tool for cross-agent coordination. Build agents are the only sub-agent type that can modify the filesystem. When multiple builders run in parallel, they use a shared build context with file-level locking to prevent conflicts.

## Explore: Parallel Codebase Investigation

The `explore` tool (`crates/krusty-core/src/tools/implementations/explore.rs`) is the older, specialized entry point for spawning parallel read-only agents. It takes a prompt and optionally a list of directories or files, then creates one sub-agent per directory (or file, or a single agent for general exploration).

For example, if you pass `directories: ["src/tui", "src/agent", "src/tools", "src/ai"]`, the explore tool spawns four agents that each focus on their assigned directory. They run concurrently, bounded by a configurable concurrency limit (default 10, but provider-aware -- MiniMax gets capped at 3 to avoid rate limits). Results are aggregated: findings are merged, files examined are deduplicated, and the whole package is returned to the main agent.

The unified agent tool's explore mode works similarly but as a single focused agent rather than a fan-out -- better for scoped investigation ("explore the auth module") than broad sweeps.

## Build: Parallel Code Implementation

When the agent tool receives `agent_type: "build"` with a `components` array, it creates one builder agent per component and runs them through a `SubAgentPool`. Each builder gets a detailed prompt explaining which component it owns, what the overall goal is, who the other builders are, and how to coordinate.

Builders share a `SharedBuildContext` that provides three coordination mechanisms:

1. **File locking.** Before writing or editing a file, a builder must acquire a lock through an RAII guard (`FileLockGuard`). If another builder holds the lock, it retries with exponential backoff (50ms, 100ms, 200ms, up to 10 attempts). The guard automatically releases the lock when dropped, preventing leaks from early returns or panics.

2. **Interface registration.** After creating a module, a builder can call `register_interface` to advertise its exports -- function names, class names, file paths, and a description. Other builders see these interfaces in their system prompt (which is refreshed every turn) and can import from them.

3. **Line tracking.** The build context records lines added and removed across all builders, providing aggregate statistics when the build completes.

Concurrency for build agents defaults to the number of components, clamped between 2 and 10. You can override this with `max_concurrency` -- lower values (2-3) for tightly coupled code, higher values (5-10) for independent modules.

## The Team System

Beyond the agent tool's one-shot delegation, Krusty has a persistent team system for longer-running coordination. The `TeamManager` (`crates/krusty-core/src/agent/team/manager.rs`) maintains a pool of named teammates that run as background loops, polling a SQLite task queue for work.

Each teammate is defined by a `TeammateConfig` with a name, a role, and an optional turn budget. There are three roles:

- **Builder** -- Can write files but cannot run shell commands. Gets `SubagentBuild` delegation surface.
- **Reviewer** -- Read-only file access with bash enabled. Gets `SubagentVerify` delegation surface. Intended for code review.
- **Tester** -- Same permissions as Reviewer. Intended for running test suites and validation.

Teammates operate autonomously. The manager spawns them with `spawn_teammate`, and each one starts a background loop that polls the `autonomous_tasks` SQLite table every 5 seconds for unclaimed work. When a task appears, the teammate claims it atomically (using a SQL UPDATE ... RETURNING pattern to avoid races), executes it through the standard sub-agent loop, and records the result back to the database. If no tasks arrive for 30 seconds, the teammate exits gracefully.

The manager provides lifecycle controls: `list_teammates` to check status, `cancel_teammate` to stop one, and `cancel_all` to shut everything down. Teammates carry their own `CancellationToken`, so cancellation is cooperative and immediate. The `Drop` implementation on `TeamManager` ensures all teammates are cancelled if the manager is dropped unexpectedly.

## The Auto-Classifier

When Krusty operates in autonomous mode (Mako), there is no human in the loop to approve tool calls. The auto-classifier (`crates/krusty-core/src/agent/auto_classifier.rs`) fills that gap. It is a `PreToolHook` that runs before every tool execution when the permission mode is `Autonomous`.

The classifier works in two stages:

First, it checks an allowlist of inherently safe tools -- read, grep, glob, list, memory, and others that cannot modify the system. If the tool is on the list, it passes immediately without any AI call.

For everything else, the classifier makes a fast AI call (stage 1) with a small token budget (64 tokens) asking a safety-focused model whether the tool call should be ALLOWED or BLOCKED. The prompt includes specific rules: file edits within the project are fine, test runs are fine, git operations are fine, but system modifications, privilege escalation, unknown network requests, and force-pushes are blocked. If stage 1 returns ALLOW, the tool proceeds. If stage 1 returns BLOCK or is ambiguous, the classifier escalates to stage 2.

Stage 2 uses a larger token budget (4096 tokens) to give the model room to reason about edge cases. If stage 2 says ALLOW, it proceeds. If BLOCK, the tool call is rejected with a reason. If the response is ambiguous or the call fails, it defaults to deny -- the safe default in autonomous operation.

Every decision is emitted as a `ClassifierDecision` event with the tool name, the verdict, the reason, and which stage made the call. This provides a full audit trail.

## Governance: Permission Inheritance

Sub-agents do not get to decide their own permissions. Every sub-agent inherits a `DelegationPolicy` from its parent context. This policy specifies:

- **Surface** -- What kind of delegation this is (explore, build, plan, verify). This determines which tools are available.
- **Permission mode** -- Whether the sub-agent runs supervised or autonomous. Inherited from the parent session.
- **Turn budget** -- Maximum number of conversation turns before the agent is forcefully stopped. This prevents runaway agents from consuming unbounded resources. The budget cascades: the parent's `subagent_max_turns` setting flows into each task's policy.
- **Read-only flag** -- Whether the agent can only read or can also write. Explore and plan agents are always read-only.
- **Bash access** -- Whether shell commands are available. Only verify agents and testers get this by default.

The policy is enforced at tool execution time. Before a sub-agent runs any tool, `policy.authorize_tool()` checks whether the requested tool is permitted. If not, the call returns an error rather than executing.

## Resource Management

Concurrent agents are bounded at multiple levels:

**Semaphore-based concurrency.** The `SubAgentPool` uses a tokio `Semaphore` to enforce its concurrency limit. Each agent must acquire a permit before starting. If the semaphore is full, the agent waits. To prevent deadlocks from hung agents, permit acquisition has a 5-minute timeout -- if an agent can't get a slot in that time, it fails with an error rather than blocking forever.

**Staggered spawning.** Agents are not all launched simultaneously. There is a configurable delay between spawns (default 100ms, higher for rate-sensitive providers like MiniMax at 600ms). This prevents burst traffic that could trigger provider rate limits.

**Turn budgets.** Every sub-agent has a maximum turn count. The default is 200 turns, but this is typically overridden by the parent context to something more appropriate for the task (20 turns for quick explorations, 30 for builds). When the budget is exhausted, the agent stops regardless of whether it considers itself "done."

**Cancellation tokens.** Every sub-agent receives a child cancellation token from its parent. If the parent is cancelled (Ctrl+C, Mako tick interrupted, team manager shutdown), cancellation propagates to all children immediately. Each turn of the agent loop checks the token before proceeding.

**Panic recovery.** Sub-agents run as tokio tasks. If a task panics, the `JoinHandle` catches it and converts it to a failed `SubAgentResult` with the panic message. The pool continues collecting results from the remaining agents.

**Message pruning.** Long-running agents accumulate conversation history. To prevent context window exhaustion, the agent loop prunes messages when they exceed 100 entries, keeping the system prompt and the most recent exchanges while compressing older tool results.

**Cleanup hooks.** The `AgentConfig` trait includes a `cleanup()` method called when any agent exits, whether normally or due to cancellation. For builders, this releases all file locks held by that agent, ensuring no stale locks persist.

Together, these mechanisms ensure that sub-agents are bounded in concurrency, bounded in duration, bounded in context size, and cleaned up reliably when they finish or fail.
