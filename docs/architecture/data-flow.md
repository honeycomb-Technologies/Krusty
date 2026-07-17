# How a Message Flows Through the System

This document traces a single user message from the moment the user presses Enter to the final response rendered on screen. It covers every major subsystem the message touches, explains the event-driven architecture that ties them together, and describes how tool execution creates multi-turn agentic loops.

---

## 1. User Input Arrives

Krusty supports three entry points for user messages. All three converge on the same path: building a `Vec<ModelMessage>` conversation and handing it to the `AgenticOrchestrator`.

**TUI (terminal interface).** The user types a message and presses Enter. The TUI appends a `ModelMessage` with `Role::User` to the in-memory conversation, persists it to SQLite via `MessageStore::save_message`, then constructs an `AgenticOrchestrator` and calls `run()`.

**HTTP server.** A client sends an HTTP POST with the message body. The server handler loads the session's conversation from the database, appends the new user message, persists it, and creates an orchestrator identically to the TUI path. The server then streams `LoopEvent`s back as Server-Sent Events (SSE).

**ACP (Agent Client Protocol).** An editor integration sends a JSON-RPC request. The ACP handler follows the same pattern: load conversation, append message, persist, create orchestrator, and bridge `LoopEvent`s to JSON-RPC notifications.

The key design principle is that none of these entry points contain any AI logic. They are thin presentation layers. All intelligence lives in the orchestrator.

---

## 2. The Orchestrator Starts

`AgenticOrchestrator::run()` is the single function that kicks off the agentic loop. It takes the conversation and a `CallOptions` struct (model, temperature, tool definitions, thinking budget) and returns two channels:

```
(mpsc::UnboundedReceiver<LoopEvent>, mpsc::UnboundedSender<LoopInput>)
```

The caller receives events through the first channel and sends user interactions (tool approvals, question responses, cancellation) through the second. This channel pair is the entire interface between the core and the presentation layer. The orchestrator then spawns a tokio task that runs `run_inner`, the actual loop.

Before entering the loop, `run_inner` unpacks its configuration: session ID, working directory, project directory, permission mode (supervised or autonomous), maximum iteration budget, stream idle timeout, and the initial work mode (build or plan). It loads per-project settings from `.krusty/settings.json`, which can override the permission mode or disable specific tools. It initializes a `CompactionManager` sized to the model's context window, and a `ContextLedger` that tracks conversation state for crash recovery.

---

## 3. Context Injection

At the top of every loop iteration, the orchestrator calls `context::inject_context`. This function clones the raw conversation and prepends system messages that give the AI the context it needs. The injection order is deterministic and matters for prompt caching:

1. **Workspace context.** The working directory and project directory, telling the AI where it is operating.
2. **Environment context.** Platform details, the current model ID, git branch, and other environmental facts gathered from the local machine.
3. **Persistent memory.** User preferences, project decisions, and feedback stored in the memory database. Only memories with meaningful overlap with the latest user objective are previewed; generic terms do not qualify on their own. Previews are capped at three memories per type, 180 characters each, with a 2 KiB total ceiling.
4. **Project instructions.** Contents of instruction files discovered in the project root: `KRAB.md`, `CLAUDE.md`, `.cursorrules`, `AGENTS.md`, and others. These are read from disk on every iteration so they always reflect the latest version.
5. **Project settings append.** An optional `system_prompt_append` from `.krusty/settings.json`.
6. **Plan context.** If a plan exists for this session, its current state (tasks, completion status, dependencies) is serialized and injected so the AI knows what has been done and what remains.
7. **Delegated run context.** Summaries of recent sub-agent explorations, so the AI can resume or deepen prior investigations instead of starting over.
8. **Autonomous task context.** Status of any autonomous tasks assigned to this session.
9. **Report context.** Recent reports generated for this project.
10. **Coordinator context.** Session-type-specific behavioral guidance (code sessions vs. Mako autonomous sessions).
11. **Skills context.** Available skills and their trigger descriptions, so the AI knows what extended capabilities exist.

For chat-only sessions, this entire pipeline is bypassed. Chat sessions get a minimal system prompt with no tool access, no workspace context, and no project data -- just memory injection and a conversational persona.

After injection, the orchestrator estimates the token count of the full conversation. If it exceeds the model's context pressure threshold, the `CompactionManager` attempts live compaction: summarizing older messages in place to reduce token count while preserving the most recent context. If compaction succeeds, the conversation is replaced and re-persisted. If it fails (the conversation cannot be meaningfully reduced), the loop emits a `ContextCompactionFailed` stop reason and terminates.

---

## 4. The AI Call

The context-injected conversation is passed to `AiClient::call_streaming`. The client routes the request to the appropriate format handler based on the provider's API format:

- **Anthropic format** -- used by Anthropic directly and by compatible providers like MiniMax and Z.ai.
- **OpenAI format** -- used by OpenRouter, OpenAI, and Codex-style providers. There are two sub-variants: the classic chat completions format and the newer responses format.
- **Google format** -- used by Google Gemini.

Each format handler implements the `FormatHandler` trait, which defines three responsibilities: converting the unified `ModelMessage` types into provider-specific JSON, converting the tool schemas, and building the complete request body. This abstraction means the orchestrator never thinks about wire formats.

For the Anthropic path specifically, the system prompt is split into cache-optimized blocks. Anthropic's prompt caching is prefix-based, so static content (the base system prompt, project instructions) is placed first with `cache_control: ephemeral` markers, while dynamic content (plan state, skills) is appended last without cache markers so it does not invalidate the cached prefix between turns. Tools are sorted deterministically by name for the same reason -- non-deterministic ordering from HashMap iteration would silently break caching.

The client builds the HTTP request with appropriate authentication (Bearer token or X-API-Key header depending on the provider), sets the API version headers, and fires the request. For providers that support WebSocket streaming (like Codex), the client opens a WebSocket connection instead.

---

## 5. Streaming Response Handling

The API response comes back as a stream of Server-Sent Events (or WebSocket frames). The streaming infrastructure works in layers:

1. **Transport layer.** Raw bytes arrive from the HTTP response stream or WebSocket. A spawned tokio task (`spawn_sse_stream_task`) reads chunks and feeds them to an `SseStreamProcessor`.

2. **SSE parsing.** The processor splits the byte stream into individual SSE events and passes each event's data field to a format-specific parser (`AnthropicParser`, `OpenAIParser`, or `GoogleParser`). The parser converts provider-specific JSON into a unified `StreamPart` enum.

3. **Buffer processing.** A separate buffer processor task handles reassembly of multi-chunk events and forwards complete `StreamPart` values through an unbounded channel back to the caller.

The `StreamPart` enum is the normalized vocabulary for streaming events. It includes variants for text deltas, thinking deltas (extended reasoning), thinking completion, tool call starts, tool call argument deltas, tool call completion, server-side tool events (web search, web fetch), usage statistics, and errors.

Back in the orchestrator, `stream::process_stream` consumes these `StreamPart` values and performs two jobs simultaneously. First, it translates each `StreamPart` into a `LoopEvent` and sends it to the presentation layer through the event channel. This is how the TUI renders text as it arrives -- each `TextDelta` stream part becomes a `LoopEvent::TextDelta` that the TUI appends to the display. Second, it accumulates the complete response: building up the full text buffer, collecting thinking blocks with their signatures, and assembling completed tool calls with their parsed arguments.

The stream processor also maintains recovery checkpoints. Every 256 characters of accumulated text, it snapshots the current state (text so far, thinking so far, tool calls detected so far) and passes it to a callback that persists it to the database. If the process crashes mid-stream, the recovery state contains enough information to resume the conversation.

If no data arrives within the configured idle timeout (defaulting to the stream timeout constant), the processor emits a `StreamIdleTimeout` stop reason and terminates the stream.

---

## 6. Tool Call Detection and Execution

When the stream completes, the orchestrator has a `StreamResult` containing the full response text, any thinking blocks, and a list of `AiToolCall` structs (each with an ID, tool name, and parsed JSON arguments). If the tool call list is empty, the turn is complete and the orchestrator skips to response finalization (section 8).

If tool calls are present, the orchestrator enters tool execution. This is where the agentic behavior lives: the AI requested actions, and the system carries them out.

### Authorization

Before any tool runs, it goes through `ToolControl::authorize`. The tool control layer implements a centralized permission policy:

- Each tool has a `ToolPolicy` with a category (`ReadOnly`, `Write`, or `Interactive`), an approval flag, a retry policy, and plan-mode eligibility.
- In **autonomous mode**, all tools execute without prompting.
- In **supervised mode**, write-category tools (edit, write, bash) require user approval. The orchestrator emits a `LoopEvent::ToolApprovalRequired` and waits for a `LoopInput::ToolApproval` from the presentation layer. The user sees a prompt and can approve or deny. If denied, the tool returns a `permission_denied` error result to the AI.
- There is a 5-minute approval timeout. If the user does not respond, the tool is denied with a timeout error.

### Special Tool Dispatch

Some tools are handled directly by the executor without going through the `ToolRegistry`:

- **Mode switch tools** (`set_work_mode`, `enter_plan_mode`) change the session's work mode between build and plan.
- **Plan task tools** (`task_start`, `task_complete`, `add_subtask`, `set_dependency`) manipulate the active plan and emit `PlanUpdate` events.
- **AskUserQuestion** is partitioned out of the tool call batch. Non-ask tools execute first, then the orchestrator emits `AwaitingInput` events for each question and terminates the loop with a `LoopStopReason::AwaitingInput`. The presentation layer collects the user's answer and sends it back as a `LoopInput::UserResponse`, which starts a new orchestrator run.

### Regular Tool Execution

For standard tools (read, edit, write, bash, grep, glob, explore, and others), the executor calls `ToolRegistry::execute`. The registry looks up the tool by name, runs any registered pre-hooks (which can block execution), then calls the tool's `execute` method with a timeout. After execution, post-hooks run for logging and metrics.

Tools that produce streaming output (like bash commands) emit `ToolOutputChunk` values that the executor forwards as `LoopEvent::ToolOutputDelta` events, allowing the user to see command output in real time.

The tool result is wrapped in a `ToolResult` struct with structured JSON output and an error flag. The `ToolControl` layer publishes the result as a `LoopEvent::ToolResult` and converts it into a `Content::ToolResult` for the conversation.

### Retry Policy

Read-only tools that time out get one automatic retry. This is controlled by `ToolControl::retry_directive`, which checks the tool's policy and the error code. Write tools are never retried.

---

## 7. Tool Results Fed Back

After all tool calls in a batch are executed, the orchestrator collects the results into a `ModelMessage` with `Role::User` (tool results are user-role messages in the Anthropic message format). This message is appended to the conversation, persisted to the database, and the context ledger is updated.

The orchestrator then performs two safety checks before looping:

**Failure detection.** `failure::detect_repeated_failures` tracks tool error signatures across iterations. If the same tool keeps failing with the same error (a stuck loop), it triggers a `LoopGuardTriggered` stop. Similarly, `detect_repeated_read_only_sequence` catches the AI reading the same files in a cycle without making progress.

**Exploration budget.** The orchestrator counts consecutive read-only tool calls (read, glob, grep). If the AI spends too many turns exploring without taking action, a soft warning is logged at 15 calls and a hard threshold triggers at 30. This prevents the AI from endlessly reading files without doing anything.

If both checks pass, the orchestrator emits a `LoopEvent::TurnComplete { has_more: true }`, sets the agent state to "streaming", and loops back to step 3 -- context injection, AI call, stream processing, tool execution. This is the agentic loop: the AI keeps working, calling tools and receiving results, until it produces a response with no tool calls.

---

## 8. Response Completion and Persistence

When the AI produces a response with no tool calls, the turn is done. The orchestrator takes the following steps:

1. **Plan detection.** If the session is in plan mode, the orchestrator checks whether the AI's response contains a structured plan. If so, it parses the plan, saves it via `PlanManager`, emits `PlanUpdate` and `PlanComplete` events, and terminates with `AwaitingInput` so the user can confirm or reject the plan.

2. **Title generation.** On the first meaningful AI response in a new session, the orchestrator fires off an asynchronous title generation call. This uses a separate, lightweight AI call to produce a short session title from the conversation. The title is persisted to the session record and emitted as a `LoopEvent::TitleGenerated`.

3. **Token count update.** The final token count from the stream's usage statistics is written to the session record.

4. **Recovery state cleanup.** The recovery checkpoint is cleared, since the turn completed successfully.

5. **Agent state transition.** The session's agent state is set to "idle".

6. **Terminal events.** The orchestrator emits `LoopEvent::TurnComplete { has_more: false }` followed by `LoopEvent::Finished { stop_reason: Completed }`. The event channel is then dropped, signaling the presentation layer that the run is over.

All messages (user, assistant, and tool results) are persisted to SQLite throughout the loop via `MessageStore`. The conversation can be fully reconstructed from the database for session resumption. The `ContextLedger` also persists its state so that if the process crashes, the system can determine whether the session is resumable and what the user's latest objective was.

---

## 9. The Event Loop Protocol

The `LoopEvent` / `LoopInput` protocol is the architectural boundary between the agentic core and every presentation surface. Understanding it is key to understanding why Krusty can support a TUI, an HTTP server, and ACP with no code duplication in the core.

### LoopEvent (orchestrator to presentation)

`LoopEvent` is a tagged enum with roughly 30 variants, organized into groups:

- **Streaming events:** `TextDelta`, `TextDeltaWithCitations`, `ThinkingDelta`, `ThinkingComplete`. These arrive in real time as the AI generates its response.
- **Tool lifecycle:** `ToolCallStart`, `ToolCallComplete`, `ToolExecuting`, `ToolOutputDelta`, `ToolResult`. These trace a tool call from the moment the AI begins generating it through execution to its result.
- **Interaction events:** `AwaitingInput`, `ToolApprovalRequired`, `ToolApproved`, `ToolDenied`. These pause the loop and wait for the presentation layer to relay a user decision.
- **Server-side tools:** `ServerToolStart`, `ServerToolComplete`, `WebSearchResults`, `WebFetchResult`, `ServerToolError`. These track provider-side tool use like web search.
- **Plan and mode events:** `ModeChange`, `PlanUpdate`, `PlanComplete`, `AgentSleeping`. These report plan state changes and autonomous agent behavior.
- **Turn lifecycle:** `TurnComplete`, `TickInjected`, `Usage`, `ContextCompacted`, `TitleGenerated`, `Finished`, `Error`. These mark turn boundaries, resource consumption, and terminal states.
- **Agent delegation:** `AgentBackgroundStarted`, `AgentBackgroundCompleted`, `TeammateSpawned`, `TeammateTaskCompleted`, `TeammateTaskFailed`, `TeammateCancelled`. These report sub-agent and teammate lifecycle.

Every variant is `Serialize`, so HTTP consumers can forward them as JSON SSE frames without transformation.

### LoopInput (presentation to orchestrator)

`LoopInput` has just three variants:

- `ToolApproval { tool_call_id, approved }` -- the user's yes/no decision on a tool that requires supervised approval.
- `UserResponse { tool_call_id, response }` -- the user's text answer to an `AskUserQuestion` or `PlanConfirm` prompt.
- `Cancel` -- the user wants to abort the current run.

This asymmetry is intentional. The orchestrator is a state machine that mostly pushes information out. The only times it blocks and waits for input are tool approval and user questions -- both of which are explicit, typed responses to a specific prompt.

### LoopStopReason

When the loop terminates, the `Finished` event carries a `LoopStopReason` that tells the presentation layer why:

- `Completed` -- the AI finished its response normally.
- `AwaitingInput` -- the AI asked a question or proposed a plan and is waiting for the user.
- `BudgetExhausted` -- the iteration limit was reached.
- `ProviderError` -- the AI API returned an error.
- `LoopGuardTriggered` -- the failure detector caught a stuck loop.
- `StreamIdleTimeout` -- the stream stopped sending data.
- `UserAbort` -- the user cancelled.
- `ContextCompactionFailed` -- the conversation is too large and cannot be reduced.
- `Sleeping` -- the autonomous agent is intentionally pausing between ticks.

The presentation layer uses this to decide what to display (an error banner, a waiting indicator, or nothing) and whether to expect another orchestrator run for this session.

---

## Summary

The complete flow for a single message:

1. User input arrives at any surface (TUI, HTTP, ACP) and is appended to the conversation.
2. An `AgenticOrchestrator` is created and `run()` is called, returning event/input channels.
3. The orchestrator injects context (workspace, memory, project, plan, skills) as system messages.
4. The conversation is sent to the AI provider via `AiClient::call_streaming`, which selects the right format handler and builds a provider-specific request.
5. The streaming response is parsed from SSE into `StreamPart` values, which are translated to `LoopEvent`s and accumulated into a `StreamResult`.
6. If the AI made tool calls, each tool goes through authorization (with optional user approval), execution (with pre/post hooks and timeout), and result collection.
7. Tool results are appended to the conversation and the loop repeats from step 3.
8. When the AI responds with no tool calls, the turn completes: the response is persisted, a title is generated if needed, and the orchestrator emits `Finished`.

The `LoopEvent`/`LoopInput` protocol ensures that all of this complexity is invisible to the TUI, server, and ACP layers. They only need to render events and relay user decisions. The core is the single source of truth for AI interaction, tool execution, failure detection, context management, and session persistence.
