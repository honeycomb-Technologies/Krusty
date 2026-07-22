# The Brain: Agent Orchestrator

The agent orchestrator is the single most important piece of code in Krusty. Every conversation — whether it comes from the terminal, the web browser, a mobile app, or an editor integration — flows through the same loop. The orchestrator decides when to call the AI, when to run tools, when to ask the user for input, when to compact context, and when to stop. It is the canonical agentic loop, and there is exactly one of it.

This document explains how it works from the inside out.

## Why One Loop

Early in development, the TUI had its own agentic loop, the HTTP server had another, and the ACP server had a third. They duplicated logic for streaming, tool execution, error handling, and state persistence. When a bug was fixed in one, the others lagged behind. When a feature was added — say, failure detection — it had to be implemented three times.

The orchestrator eliminated that. It lives in `crates/krusty-core/src/agent/orchestrator.rs` and owns the entire lifecycle of an AI turn: accepting a conversation, calling the provider, processing the stream, executing tools, detecting failures, and repeating until the AI finishes or a guard intervenes. The TUI, server, and ACP are now thin presentation layers that create an orchestrator, call `run()`, and react to its events.

The architecture looks like this:

```
 Orchestrator ──── LoopEvent ────► Consumer (TUI / Server / ACP)
   (core)      ◄──── LoopInput ────  (presentation layer)
```

The orchestrator emits `LoopEvent` values for every state change — text deltas, tool executions, errors, mode switches. The consumer sends `LoopInput` values back for user interactions like tool approvals, question responses, and cancellation. This is a clean unidirectional data flow with a single feedback channel.

## Configuration and Services

Product surfaces build a validated `RunSpec`, which owns the per-run configuration and starts the crate-private orchestrator with shared `OrchestratorServices`.

**OrchestratorConfig** captures everything specific to a single run:

- `session_id` — which conversation this run belongs to
- `working_dir` and `project_dir` — filesystem context for tool execution
- `permission_mode` — whether tools run autonomously or require user approval (can be overridden by per-project `.krusty/settings.json`)
- `max_iterations` — optional turn budget (the hard ceiling on how many AI round-trips this run can make)
- `stream_idle_timeout` — how long to wait for data on a stalled stream before giving up
- `initial_work_mode` — whether the session starts in Build mode (read/write) or Plan mode (read-only)
- `generate_title` — whether to auto-generate a session title from the first AI response
- `delegated_progress_tx` — optional channel for forwarding sub-agent progress to external surfaces

**OrchestratorServices** holds the shared infrastructure:

- `ai_client` — the normalized LLM client (handles Anthropic, OpenAI, OpenRouter, and others behind one interface)
- `tool_registry` — the complete set of registered tools and their execution logic
- `process_registry` — background process management for bash commands
- `db_path` — where the SQLite database lives for persistence
- `skills_manager` — available slash-command-style skills

The consumer resolves a frozen model runtime and constructs a `RunSpec` through `RunSpecBuilder`, then calls `run()` with the shared services, conversation history, and call options. Direct orchestrator construction is crate-private. The run returns a pair of channels — an event receiver and an input sender — and spawns the loop as a tokio task. From that point on, the consumer listens for canonical events and occasionally sends input.

## The Main Loop

The heart of the orchestrator is an async loop inside `run_inner()`. Here is what happens on every iteration.

### 1. Turn budget check

The loop checks `max_iterations`. If the current iteration count meets or exceeds the budget, the loop emits a `LoopEvent::Error` with a budget exhaustion message, then a `LoopEvent::Finished` with stop reason `BudgetExhausted`, and returns. This is a hard stop — no more AI calls.

### 2. Context injection

Before each AI call, the orchestrator builds a context-injected copy of the conversation. The raw conversation (just user and assistant messages) gets a stack of system messages prepended:

- **Workspace context** — the execution directory, project directory, and workspace mode (project vs. neutral)
- **Environment context** — platform, shell, date, current git branch, modification counts
- **Persistent memory** — user preferences, feedback, project notes pulled from the database (capped at 8KB)
- **Project instructions** — contents of `KRAB.md`, `CLAUDE.md`, `.cursorrules`, or similar instruction files found in the project hierarchy
- **Project settings** — any `system_prompt_append` from `.krusty/settings.json`
- **Plan context** — the active plan's task list, progress, and which tasks are ready vs. blocked
- **Delegated run context** — recent sub-agent investigations, so the AI knows what's already been explored
- **Autonomous task context** — pending and in-progress Mako tasks
- **Report context** — recent reports available via `ReadReport`
- **Skills context** — available skills and how to invoke them

This injection happens every iteration, so the AI always sees current state. The context module lives in `crates/krusty-core/src/agent/context.rs`.

### 3. Context compaction check

Before sending the conversation to the AI, the orchestrator estimates its token count and checks whether it exceeds the model's compaction trigger threshold. If it does, the `CompactionManager` kicks in.

Compaction is a multi-pass process that tries to shrink the conversation without losing critical information:

1. **Strip old thinking blocks** — extended thinking from older turns is removed (only the two most recent are kept)
2. **Compact old tool results** — tool outputs beyond the most recent six messages are truncated to short summaries, with a note to re-run the tool if details are needed. Tool results tagged `drop_after_compaction` are replaced entirely.
3. **Summary replacement** — if the conversation is still too large after the above, the middle section (everything between the first user message and the recent tail) is replaced with a structured summary. The summary preserves user goals, assistant progress, important tool activity, and the latest user request.
4. **Aggressive continuation** — if the summary replacement still isn't small enough, the system drops everything except system messages and one or two recent messages, replacing everything else with a dense summary.

If compaction succeeds, the orchestrator emits a `ContextCompacted` event, persists the compacted conversation, and continues. If it fails (the result is still above the hard failure threshold), the loop terminates with `ContextCompactionFailed`.

This is different from "pinch" (session continuation), which creates an entirely new session with a handoff artifact. Compaction keeps the same session alive by trimming history in place.

### 4. Stream the AI response

The orchestrator calls `ai_client.call_streaming()` with the context-injected conversation and receives a stream of `StreamPart` events. The stream processor in `crates/krusty-core/src/agent/stream.rs` accumulates text, thinking blocks, and tool calls while forwarding deltas to the event channel as `TextDelta`, `ThinkingDelta`, `ToolCallStart`, and `ToolCallComplete` events.

During streaming, the orchestrator periodically checkpoints the partial assistant state to the database. If the process crashes mid-stream, the session recovery system can detect an interrupted turn and present options to the user.

If the stream stalls beyond the configured idle timeout, processing stops with a `StreamIdleTimeout` reason. If the AI provider returns an error, it stops with `ProviderError`.

### 5. Build and save the assistant message

Once streaming completes, the orchestrator constructs a `ModelMessage` from the accumulated text, thinking blocks, and tool calls, pushes it onto the conversation, updates the context ledger, and persists it to the database.

On the first response, if title generation is enabled, the orchestrator spawns a background task that calls the AI with the user's first message to generate a short session title.

### 6. Handle the no-tools case

If the AI response contains no tool calls, the turn is complete. The orchestrator checks for plan detection (in plan mode, the AI might have written a structured plan in its response text), emits `TurnComplete` and `Finished` events, and returns.

### 7. Handle AskUser partitioning

If the AI wants to ask the user a question (via the `AskUserQuestion` tool), the orchestrator partitions the tool calls. Non-AskUser tools execute immediately. Then placeholder results are added for the AskUser calls, and the loop emits `AwaitingInput` events and pauses. The consumer is responsible for collecting the user's response and submitting it to resume the session.

### 8. Failure detection (pre-execution)

Before executing tools, the orchestrator checks for repeated read-only exploration sequences. If the AI keeps issuing the exact same read-only tool pattern (same tools with same arguments) across multiple iterations without taking action, a loop guard fires and the session stops with an error message telling the AI to act on the evidence or change strategy. The threshold is four identical sequences.

### 9. Execute tools

Tool calls go through `crates/krusty-core/src/agent/executor/mod.rs`. For each tool call in the batch:

1. **Disabled tool check** — if the tool is listed in the project's `disabled_tools`, it's denied immediately
2. **Authorization** — the `ToolControl` module checks the permission mode. In autonomous mode, tools execute without approval. In supervised mode, write-category tools emit a `ToolApprovalRequired` event and wait for the user to approve or deny. Read-only tools run without approval regardless of mode. There's a five-minute timeout on approval requests.
3. **Special tool interception** — mode switch tools (`set_work_mode`, `enter_plan_mode`) and plan task tools (`task_start`, `task_complete`, `add_subtask`, `set_dependency`) are handled directly by the orchestrator rather than going through the tool registry, because they mutate the loop's own state.
4. **Regular execution** — the tool runs through the `ToolRegistry`, which handles argument validation, path and permission policy enforcement, and output streaming. These policies are not an OS sandbox for Bash. Tool output gets streamed back via `ToolOutputDelta` events (so bash output, for example, appears in real time).
5. **Retry policy** — read-only tools that time out get one automatic retry. Everything else stops on failure.
6. **Result shaping** — the `ToolControl` module truncates oversized tool outputs (over 30,000 characters), wraps the result with history metadata for later compaction, and publishes the `ToolResult` event.

During execution of sub-agent tools (the `agent` tool), the executor sets up a progress channel so delegated progress events flow back to the parent session's event stream.

### 10. Failure detection (post-execution)

After tool execution, the orchestrator runs two more failure detectors:

- **Repeated failure detection** — tracks tool error signatures across iterations. If the same tool fails with the same error pattern (same error code, same fingerprint, same arguments) twice, the loop stops. Success on a tool clears its failure counter. This prevents infinite retry loops where the AI keeps calling a broken tool the same way.
- **Terminal explore failure** — if a delegated explore tool returned with zero usable agents or zero files examined, the loop stops immediately rather than letting the AI try to manually re-explore.

There's also a post-explore manual fallback detector that stops the AI from issuing broad read-only probes after a delegated exploration already returned usable coverage.

### 11. Exploration budget

The orchestrator tracks a soft exploration budget. Read-only tool calls (read, glob, grep) increment a counter. Any write action resets it. At 15 read-only calls, a warning is logged. At 30, a hard warning fires. This doesn't stop the loop by itself, but it signals that the AI may be over-exploring without acting.

### 12. Save results and loop

Tool results are saved as a user message (since tool results go in the user role per the Anthropic message format), the context ledger is updated, and the loop continues from step 1.

## Turn Management

Every pass through the main loop is a "turn" — one AI call and its resulting tool executions. The iteration counter increments at the top of each loop pass. The turn budget (`max_iterations`) provides a hard ceiling.

`AgentState` in `crates/krusty-core/src/agent/state.rs` tracks turn count, timing, and interruption status. Interactive, ACP, and delegated runs are unlimited by default. `AgentConfig` can provide separate explicit `primary_max_turns`, `subagent_max_turns`, and `acp_max_turns` resource ceilings; the legacy `max_turns` field remains a lower-precedence migration fallback. Semantic repetition is handled by the progress ledger rather than a hidden turn count.

## The Hook System

Hooks intercept tool calls before and after execution. They're defined by the `PreToolHook` and `PostToolHook` traits in `crates/krusty-core/src/agent/hooks/mod.rs`.

### Built-in Hooks

**SafetyHook** — blocks dangerous bash commands before they execute. It parses shell commands into segments (handling pipes, semicolons, and logical operators), strips environment variable prefixes, and checks each segment against a set of rules:

- Fork bombs
- `curl`/`wget` piped to `sh`/`bash`
- Redirects to block devices (`/dev/sda`, etc.)
- `sudo`, `doas`, `su` (privilege escalation)
- `rm -rf /` and similar destructive targets
- `chmod 777`
- `dd` with raw device access
- `mkfs` (filesystem formatting)

The hook is regex-based with proper shell quoting awareness, so `rm '-rf' /` is caught just as well as `rm -rf /`.

**PlanModeHook** — when plan mode is active, blocks all write-category tools and modifying bash commands. It uses the tool registry's per-tool policy to determine categories. Read-only tools and read-only bash commands (like `ls`, `git status`, `git diff`) pass through. This enforces the "read before you write" discipline of plan mode.

**LoggingHook** — runs after every tool execution, logging the tool name, duration, error status, and output size via the tracing system.

### User-Configurable Hooks

Users can define their own hooks in `crates/krusty-core/src/agent/user_hooks/mod.rs`. Each hook is a shell command with a regex pattern that matches tool names. The hook receives JSON on stdin containing the tool name, arguments, hook ID, and hook type.

The exit code protocol:
- `0` — continue (stdout/stderr not shown)
- `2` — block tool execution, show stderr as the reason
- Any other code — warn the user with stderr, but continue

User hooks have four types: `PreToolUse`, `PostToolUse`, `Notification`, and `UserPromptSubmit`. They're persisted in the database, support enable/disable toggling, and have a 30-second execution timeout.

The `UserPreToolHook` and `UserPostToolHook` wrappers adapt user hooks into the `PreToolHook`/`PostToolHook` traits so they plug into the same execution pipeline as built-in hooks.

## Context Ledger

The `ContextLedger` in `crates/krusty-core/src/agent/context_ledger.rs` tracks the high-level state of the conversation for compaction and continuation decisions. It counts canonical messages, summarized messages, dropped messages, pinned messages (like project instructions), and replayed messages. It also extracts the latest user objective — the most recent user message that contains actual text (not just tool results).

The ledger produces two persistence artifacts:

- **ContextLedgerRecord** — a snapshot of all counters and the last compaction event, serialized to the database after every state change
- **ContinuationContract** — a `Resumable` or `NonResumable` decision used by the pinch system. If the conversation has messages and a clear user objective, it's resumable. If the conversation is empty or has no extractable objective, it's non-resumable.

## Summarization

When a session reaches the end of its useful context window, the pinch system can create a continuation session. The summarizer in `crates/krusty-core/src/agent/summarizer.rs` calls the AI to produce a structured JSON summary with four fields:

- `work_summary` — 2-3 paragraphs of what was accomplished
- `key_decisions` — architectural choices and trade-offs
- `pending_tasks` — incomplete work and identified next steps
- `important_files` — the top 10 most relevant file paths

The summarizer is cache-safe: when conversation history exists, it reuses the parent conversation's cached prefix (same system prompt, same message sequence) and appends the summarization instruction as a new user message. This means the provider only needs to process the summarization instruction itself — everything before it is already cached. This saves significant cost on long conversations.

## Session Recovery

The orchestrator persists recovery state at critical transition points throughout the loop. Before streaming starts, it writes a `Streaming` status. During streaming, checkpoints capture the partial assistant response (text, thinking, tool calls accumulated so far). Before tool execution, it writes a `ToolExecuting` status. After tool execution, recovery state is cleared.

If the process crashes or the connection drops, the next session load can inspect the recovery state and determine what happened:

- **Interrupted during streaming** — the partial response can be shown to the user, and they can choose to retry or continue
- **Interrupted during tool execution** — the user is warned that tools may have partially executed

The recovery state includes the context ledger snapshot, the stop reason (if any), error details, and the partial assistant state. This is stored per-session in the database.

## The LoopEvent / LoopInput Protocol

`LoopEvent` (in `crates/krusty-core/src/agent/loop_events.rs`) is a tagged enum with roughly 30 variants covering every observable state change:

**Streaming events** — `TextDelta`, `TextDeltaWithCitations`, `ThinkingDelta`, `ThinkingComplete`

**Tool lifecycle** — `ToolCallStart`, `ToolCallComplete`, `ToolExecuting`, `ToolOutputDelta`, `ToolResult`

**Interaction** — `AwaitingInput`, `ToolApprovalRequired`, `ToolApproved`, `ToolDenied`

**Server-side tools** — `ServerToolStart`, `ServerToolComplete`, `WebSearchResults`, `WebFetchResult`, `ServerToolError`

**Mode and plan** — `ModeChange`, `PlanUpdate`, `PlanComplete`

**Turn lifecycle** — `TurnComplete`, `TickInjected`, `Usage`, `ContextCompacted`, `TitleGenerated`, `Finished`, `Error`

**Background agents** — `AgentBackgroundStarted`, `AgentBackgroundCompleted`, `UserMessage`, `ClassifierDecision`

**Team events** — `TeammateSpawned`, `TeammateTaskCompleted`, `TeammateTaskFailed`, `TeammateCancelled`

`LoopStopReason` is a separate enum that tags how the loop terminated: `Completed`, `AwaitingInput`, `BudgetExhausted`, `ProviderError`, `LoopGuardTriggered`, `StreamIdleTimeout`, `UserAbort`, `ContextCompactionFailed`, or `Sleeping`.

`LoopInput` has three variants:

- `ToolApproval` — user approved or denied a tool (carries the tool call ID and a boolean)
- `UserResponse` — user answered an AskUser or PlanConfirm prompt (carries the tool call ID and response text)
- `Cancel` — user requested cancellation

This protocol is the complete contract between the orchestrator and any consumer. If you can emit `LoopInput` and consume `LoopEvent`, you can build a new Krusty frontend.

## Cancellation

`AgentCancellation` in `crates/krusty-core/src/agent/cancellation.rs` wraps a `tokio_util::CancellationToken`. It provides `cancel()` to signal all tasks, `child_token()` to create scoped sub-tokens for individual operations, and `reset()` to create a fresh token for the next request. This is how the user's Ctrl+C propagates through nested async operations — the cancellation token flows from the session down through the orchestrator, into tool execution, and through sub-agent spawning.

## Plan Mode Integration

The orchestrator has deep integration with plan mode through `crates/krusty-core/src/agent/plan_handler.rs`. When the AI calls `set_work_mode` or `enter_plan_mode`, the orchestrator intercepts the call before it reaches the tool registry, updates the session's work mode in the database, and emits a `ModeChange` event.

In plan mode, the AI can read files and explore the codebase but cannot write. The `PlanModeHook` enforces this by blocking write-category tools and modifying bash commands. When the AI produces a response that contains a structured plan (detected by `try_detect_plan()`), the orchestrator parses it, saves it to the database, and emits `PlanUpdate` and `PlanComplete` events. The consumer then presents the plan to the user for confirmation before switching to build mode.

During build mode with an active plan, the AI uses `task_start` and `task_complete` to track progress through the plan's phases. The orchestrator intercepts these calls too, updates the plan state in the database, and emits `PlanUpdate` events so the UI can show real-time progress.

## Key Source Files

| File | Purpose |
|------|---------|
| `crates/krusty-core/src/agent/orchestrator.rs` | The orchestrator itself — config, services, and the main loop |
| `crates/krusty-core/src/agent/mod.rs` | Module index and public re-exports |
| `crates/krusty-core/src/agent/executor/mod.rs` | Tool execution engine with approval workflow and retry policy |
| `crates/krusty-core/src/agent/state.rs` | Turn counting, timing, and per-session configuration |
| `crates/krusty-core/src/agent/hooks/mod.rs` | SafetyHook, PlanModeHook, LoggingHook, and the hook traits |
| `crates/krusty-core/src/agent/user_hooks/mod.rs` | User-configurable hooks with shell command execution |
| `crates/krusty-core/src/agent/failure.rs` | Repeated failure detection and exploration loop guards |
| `crates/krusty-core/src/agent/context.rs` | Context injection — system prompt assembly from all sources |
| `crates/krusty-core/src/agent/context_ledger.rs` | Context tracking for compaction and continuation decisions |
| `crates/krusty-core/src/agent/compaction/mod.rs` | Live conversation compaction (same-session context trimming) |
| `crates/krusty-core/src/agent/summarizer.rs` | AI-powered conversation summarization for session handoff |
| `crates/krusty-core/src/agent/cancellation.rs` | Cancellation token wrapper for async task interruption |
| `crates/krusty-core/src/agent/plan_handler.rs` | Plan and mode switch tool handlers |
| `crates/krusty-core/src/agent/loop_events.rs` | LoopEvent and LoopInput type definitions |
| `crates/krusty-core/src/agent/stream.rs` | Stream processing — accumulates AI response chunks |
| `crates/krusty-core/src/agent/tool_control.rs` | Approval, retry, and result-shaping policy |
