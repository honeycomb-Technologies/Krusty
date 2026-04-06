# Mako: Autonomous Agent Mode

Mako is Krusty's autonomous agent system. Named after the mako shark --- an obligate ram ventilator that never stops swimming --- Mako transforms Krusty from a tool you talk to into a daemon that works in the background on complex tasks without waiting for your input at every step.

You give Mako a high-level objective ("refactor the auth module", "add pagination to every list endpoint"), and it plans the work, breaks it into tasks, executes them, verifies the results, and reports back when it is done. You can watch it work in real-time, pause it, send it follow-up instructions, or walk away entirely and check the results later.

## The Three Session Modes

Krusty organizes all work into sessions, and every session has one of three types that determine what the agent can do and how it operates.

**Chat** is the conversational mode. It gives the AI access to web search and research tools, but no ability to edit files or run commands. Think of it as a knowledgeable colleague you can ask questions. Chat sessions are interactive --- the AI responds to your messages and waits for the next one.

**Code** is the full agentic coding assistant. It has access to every tool: file editing, terminal commands, git operations, sub-agent delegation, and planning. Code sessions are also interactive. The AI proposes changes, you approve tool calls when prompted, and you stay in the loop throughout.

**Mako** is fully autonomous. It has every tool Code has, plus additional coordination tools (task management, sleep signaling, user messaging). The critical difference is that Mako sessions run without a human in the loop. There is no approval prompt for tool calls. Instead, an automatic safety classifier evaluates every tool invocation in real-time, and the agent operates on a tick-based execution engine that keeps it working across multiple rounds of activity and sleep.

The permission model reflects this distinction. Code sessions run in `Supervised` mode, where certain tools require explicit user approval. Mako sessions run in `Autonomous` mode, where the auto-classifier replaces the human approval step.

## How the Tick Engine Works

In a normal interactive session, the AI processes your message, does some work, and stops. It waits for your next message before doing anything else. Mako cannot work this way --- there is no one sitting at the keyboard.

The tick engine solves this by wrapping the standard orchestrator in a wake-execute-sleep cycle. When a Mako session starts, the tick engine launches the orchestrator with the conversation so far. The orchestrator processes messages, calls tools, and eventually finishes its current run. At this point, instead of stopping and waiting for input, the tick engine looks at the final tool output for a sleep signal.

If the agent called the `sleep` tool (indicating it has nothing to do right now and wants to wake up later), the tick engine records the requested duration and transitions the session into a sleeping state. The runtime manager schedules a future wake at the specified time. When that time arrives, the session wakes, a new tick is injected into the conversation, and the orchestrator runs again.

The default tick interval is 30 seconds, and sessions can run for up to 1000 ticks before the engine stops them. In practice, Mako sessions rarely hit this limit because they sleep when idle rather than busy-looping.

The tick engine also forwards external inputs. If you send a follow-up message to a sleeping Mako session, the runtime manager wakes it immediately rather than waiting for the scheduled time. If you cancel a session, the engine propagates the cancellation to the inner orchestrator and drains remaining events cleanly.

## The Auto-Classifier

Since Mako sessions have no human approving tool calls, every tool invocation passes through the auto-classifier --- an AI-powered safety gate implemented as a `PreToolHook`.

The classifier operates in three tiers.

**Tier 0: Safe tool bypass.** A hardcoded allowlist of tools that are inherently read-only or coordination-only skips classification entirely. This includes `read`, `grep`, `glob`, `list`, `memory`, `create_task`, `update_task`, `list_tasks`, `send_user_message`, `sleep`, and several others. These tools cannot cause damage, so checking them would waste time and tokens.

**Tier 1: Fast classification.** For tools not on the safe list (like `bash`, `write`, or `apply_patch`), the classifier sends the tool name and its arguments to the AI with a compact safety prompt. It requests a verdict in 64 tokens or fewer. If the response clearly says ALLOW, the tool proceeds. If it says BLOCK, or is ambiguous, the call escalates to tier 2.

**Tier 2: Thinking classification.** The same prompt and tool call are sent again, but with a 4096-token budget so the classifier can reason through edge cases. If this stage returns ALLOW, the tool proceeds. If it returns BLOCK, is ambiguous, or errors out, the tool is denied.

The classifier's safety prompt defines clear boundaries. It allows file edits within the project, test and build commands, git operations, and dependency installation from lockfiles. It blocks operations outside the project directory, network requests to unknown hosts, system package installation, privilege escalation, credential access, and destructive git operations like force-push.

Every classifier decision is emitted as a `ClassifierDecision` event, so you can see exactly what was allowed or blocked and why when watching the event stream.

## CLI Interface

Mako is controlled entirely through the `krusty mako` subcommand. The CLI talks to the Krusty server over HTTP.

**`krusty mako run "<task>"`** dispatches a new task. This creates a Mako session, saves your task as the first message, and starts the tick engine. You get back a session ID. Add `--attach` to immediately stream events, or `--project-dir` to specify a different working directory.

**`krusty mako status`** lists all Mako sessions with their runtime status, agent state, and title. Pass a session ID for detailed status including the task list, sleep/wake reasons, and errors.

**`krusty mako attach <session-id>`** connects to the live event stream. You see text output, tool calls and results, classifier decisions, tick injections, sleep notifications, and completion or error events as they happen. The stream uses Server-Sent Events.

**`krusty mako pause <session-id>`** stops the active run and moves the session into a paused state. The tick engine stops, scheduled wakes are cancelled, and the session sits idle until you resume it.

**`krusty mako resume <session-id>`** restarts a paused or idle session. The tick engine picks up where it left off.

**`krusty mako cancel <session-id>`** stops the session, cleans up its runtime state, and deletes it entirely.

**`krusty mako send <session-id> "<message>"`** queues a follow-up message to an existing session. This saves the message and wakes the session immediately, even if it was sleeping. The agent sees your message on its next tick and can adjust its plan.

## Event Streaming

When you attach to a Mako session, you receive a stream of typed events that show exactly what the agent is doing.

`text_delta` events carry the agent's reasoning and output text, streamed token by token. `tool_call_start` and `tool_call_complete` events bracket each tool invocation with the tool name. `tool_result` events carry the output of completed tool calls. `tool_output_delta` events stream long tool outputs incrementally.

`classifier_decision` events report the auto-classifier's verdict for each non-safe tool call, including the tool name, the decision (allow or block), the reason, and which classification stage made the decision.

`tick_injected` events mark the boundaries between execution rounds, showing the tick number. `agent_sleeping` events report when the agent decides to sleep, including the duration and reason.

`user_message` events surface messages the agent explicitly sends to you via the `send_user_message` tool, with a severity level and optional title. `plan_update` events report changes to the agent's internal plan. `delegated_progress` events track sub-agents spawned by the coordinator.

`finish` events signal the end of a run with a stop reason. `error` events report failures.

The event stream supports replay. When you attach, the server replays recent events (up to 200, defaulting to 50) so you can catch up on what happened while you were away. You can specify `after_sequence` to resume from a specific point.

## Task Management

Mako breaks complex objectives into discrete, trackable tasks. The task system is backed by SQLite and exposed to the agent through three tools: `create_task`, `update_task`, and `list_tasks`.

Each task has a subject (short title), a description, a status (pending, in_progress, completed, or failed), an optional owner, and a list of dependency edges. Dependencies are expressed as `blocked_by` references to other task IDs, which lets the agent define a dependency graph. A task is considered available for work only when all of its blockers have been completed.

The typical workflow follows the coordinator prompt's phases: research (read the codebase, understand the problem), synthesis (create tasks with dependency ordering), implementation (claim tasks, delegate to sub-agents, mark results), and verification (confirm the work was actually done correctly).

When you run `krusty mako status <session-id>`, the task list is displayed with each task's status, subject, owner, blockers, and result. This gives you a clear picture of how far along a complex job is, which parts are done, and what is blocked.

## Runtime State

The Mako runtime manager tracks the lifecycle of every autonomous session through persistent state stored in SQLite. Each session can be in one of seven states: `idle`, `running`, `sleeping`, `awaiting_input`, `paused`, `error`, or `cancelled`.

When running, the state records the current run ID and wake reason (dispatch, resume, user_message, sleep, or startup recovery). When sleeping, it records the next wake time and sleep reason. When in error, it records the error message. This state persists across server restarts --- if the server goes down while a Mako session is running or sleeping, it recovers those sessions on startup. Running sessions restart immediately; sleeping sessions resume their scheduled wake or start right away if the wake time has passed.

The runtime manager coordinates this through a central wake command channel. Scheduled wakes are tokio tasks that sleep for the required duration and then send a wake command. The manager ensures only one run is active per session at a time, cancelling any existing run before starting a new one.

## The Server API

The Krusty server exposes Mako management through a dedicated route group under `/api/mako/`.

`POST /api/mako/dispatch` accepts a task string and optional project directory and model override. It creates a new Mako session, saves the task as the opening message, starts the tick engine, and returns the session ID.

`GET /api/mako/sessions` lists all Mako sessions for the current user, including each session's title, agent state, and runtime state.

`GET /api/mako/sessions/:id/status` returns detailed status for a single session: session type, title, task list, agent state, and runtime state with sleep/wake/error details.

`GET /api/mako/sessions/:id/events` opens an SSE stream for real-time observation. It supports `replay_limit` and `after_sequence` query parameters for catching up on past events.

`POST /api/mako/sessions/:id/message` sends a follow-up message and wakes the session.

`POST /api/mako/sessions/:id/pause` and `POST /api/mako/sessions/:id/resume` control session execution.

`DELETE /api/mako/sessions/:id` cancels and deletes a session, cleaning up runtime state, scheduled wakes, and session locks.

All endpoints enforce session ownership. Multi-tenant deployments isolate Mako sessions by user ID; requests for another user's sessions return 404.

## Putting It All Together

A typical Mako interaction looks like this. You dispatch a task from the CLI or the Expo app. The server creates a session, the tick engine starts, and the coordinator prompt instructs the AI to begin with research. The agent reads relevant files, creates tasks with dependency edges, and starts working through them. It delegates implementation to sub-agents, tracks progress through the task list, and sleeps when waiting for background work to finish. The auto-classifier evaluates every tool call, blocking anything dangerous. When all tasks are complete, the agent runs verification, sends you a summary, and the session goes idle.

Mako is designed for work that takes an hour of focused coding --- work where the plan is clear enough to express in a sentence, but the execution involves many files and enough mechanical effort that you would rather hand it off and check back later.
