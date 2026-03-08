# Krusty Platform Unification Plan

## Vision

One brain, many faces. The agentic loop — streaming, tool execution, context injection,
plan management, failure detection — lives in `krusty-core`. The TUI and server are thin
presentation layers that consume events and translate user input. A user on the PWA gets
the exact same agent intelligence, tools, and context as a user in the terminal.

---

## Architecture: Before and After

### Before (Current)

```
 ┌──────────────────────────────┐    ┌──────────────────────────────┐
 │      TUI  (krusty-cli)       │    │    Server  (krusty-server)   │
 │                              │    │                              │
 │  StreamingManager            │    │  chat.rs  (2,328 lines)      │
 │  ├─ state machine            │    │  ├─ run_agentic_loop()       │
 │  ├─ context_building.rs      │    │  ├─ process_stream()         │
 │  ├─ tool_execution.rs        │    │  ├─ execute_tools()          │
 │  ├─ plan detection           │    │  ├─ plan detection           │
 │  ├─ mode switching           │    │  ├─ mode switching           │
 │  ├─ failure detection        │    │  ├─ failure detection        │
 │  ├─ explore/build tools  ✅  │    │  ├─ explore/build tools  ❌  │
 │  ├─ context injection    ✅  │    │  ├─ context injection    ❌  │
 │  ├─ user hooks on reg.   ✅  │    │  ├─ user hooks on reg.   ❌  │
 │  ├─ MCP tools registered ✅  │    │  ├─ MCP tools registered ❌  │
 │  └─ title generation     ✅  │    │  └─ title generation     ⚠️  │
 │                              │    │                              │
 │  acp_bridge.rs (1,152 lines) │    │  ← ALSO duplicated here     │
 └──────────┬───────────────────┘    └──────────┬───────────────────┘
            │                                    │
            └──────────┐    ┌────────────────────┘
                       ▼    ▼
              ┌────────────────────┐
              │    krusty-core     │
              │  (primitives only) │
              │  AiClient, Tools,  │
              │  Storage, MCP      │
              └────────────────────┘
```

### After (Unified)

```
              ┌────────────────────────────────────────────┐
              │              krusty-core                    │
              │                                            │
              │  ┌──────────────────────────────────────┐  │
              │  │       agent/orchestrator.rs           │  │
              │  │                                      │  │
              │  │  AgenticOrchestrator                 │  │
              │  │  ├─ run() → LoopEvent stream         │  │
              │  │  ├─ context injection (auto)         │  │
              │  │  ├─ stream processing                │  │
              │  │  ├─ tool dispatch + approval         │  │
              │  │  ├─ plan detection + task mgmt       │  │
              │  │  ├─ mode switching                   │  │
              │  │  ├─ failure detection                │  │
              │  │  ├─ exploration budget               │  │
              │  │  └─ title generation                 │  │
              │  └──────────────┬───────────────────────┘  │
              │                 │                           │
              │  ┌──────────┐ ┌┴─────────┐ ┌───────────┐  │
              │  │ AiClient │ │ToolReg.  │ │ Storage   │  │
              │  │ Providers│ │ 18 tools │ │ Sessions  │  │
              │  │ Streaming│ │ MCP      │ │ Plans     │  │
              │  └──────────┘ │ Hooks    │ │ Skills    │  │
              │               └──────────┘ └───────────┘  │
              └──────────┬──────────────────────┬──────────┘
                         │                      │
              ┌──────────┴──────┐    ┌──────────┴──────────┐
              │  TUI (thin)     │    │  Server (thin)       │
              │                 │    │                      │
              │  Consumes       │    │  Consumes            │
              │  LoopEvent →    │    │  LoopEvent →         │
              │  render blocks  │    │  SSE events          │
              │                 │    │                      │
              │  Sends          │    │  Sends               │
              │  LoopInput ←    │    │  LoopInput ←         │
              │  keyboard/mouse │    │  HTTP endpoints      │
              │                 │    │                      │
              │  ~0 orchestr.   │    │  ~200 lines of       │
              │  logic          │    │  HTTP glue           │
              └─────────────────┘    └─────────────────────┘
```

---

## Phase 0: Critical Quick Fixes

> Immediate parity wins. No architectural changes. Can ship today.

### 0.1 Register explore + build tools on server

**File:** `crates/krusty-server/src/lib.rs` (after line 223)

The server calls `register_all_tools()` but never registers the two most powerful
tools. The TUI does this in `handlers/provider.rs:59-82` after creating an AI client.

**Change:** After `register_all_tools`, if we have an AI client, register the
sub-agent tools. Requires creating an `AgentCancellation` for the server.

**Impact:** PWA gains parallel exploration and multi-file build capabilities.

### 0.2 Wire user hooks to tool registry

**File:** `crates/krusty-server/src/lib.rs` (after line 221)

The server creates `UserHookManager` (line 247) but never adds `UserPreToolHook`
and `UserPostToolHook` to the tool registry's hook chain. The TUI does this in
`app_builder.rs` via `init_tool_registry()`.

**Change:** Add pre/post user hook wrappers to the registry before `register_all_tools`.

**Impact:** User-configured hooks actually fire on the server.

### 0.3 Register MCP tools

**File:** `crates/krusty-server/src/lib.rs` (after line 233)

The server creates `McpManager`, loads config, connects servers — but never calls
`register_mcp_tools()` to expose those tools to the AI. The TUI does this in
`app_builder.rs:311`.

**Change:** After `mcp_manager.connect_all()`, register MCP tools on the tool registry.

**Impact:** MCP server tools become available to PWA sessions.

### 0.4 Add context injection to server agentic loop

**File:** `crates/krusty-server/src/routes/chat.rs` (around line 709, before AI call)

The TUI injects plan context, skills context, and project context as system messages
at position 0 of the conversation before every AI call (`streaming/mod.rs:262-326`).
The server injects none.

**Change:** Port the context building logic from `context_building.rs` into the
server's agentic loop. Build plan/skills/project context strings, insert as
`ModelMessage { role: Role::System }` at conversation head before each streaming call.

**Impact:** PWA agent becomes plan-aware, skill-aware, project-aware.

---

## Phase 1: Core Orchestrator Extraction

> The foundation. Extract the agentic loop into a reusable, platform-agnostic
> orchestrator in krusty-core that both TUI and server consume.

### 1.1 Define the event protocol

**New file:** `crates/krusty-core/src/agent/events.rs` (expand existing)

Define `LoopEvent` — the canonical event type emitted by the orchestrator.
This replaces the server's `AgenticEvent` and the TUI's `StreamPart` processing
as the single source of truth.

```rust
pub enum LoopEvent {
    // Streaming
    TextDelta { delta: String },
    ThinkingDelta { thinking: String },

    // Tool lifecycle
    ToolCallStart { id: String, name: String },
    ToolCallComplete { id: String, name: String, arguments: Value },
    ToolExecuting { id: String, name: String },
    ToolOutputDelta { id: String, delta: String },
    ToolResult { id: String, output: String, is_error: bool },

    // Interaction
    AwaitingInput { tool_call_id: String, tool_name: String },
    ToolApprovalRequired { id: String, name: String, arguments: Value },
    ToolApproved { id: String },
    ToolDenied { id: String },

    // Mode + Plan
    ModeChange { mode: WorkMode, reason: Option<String> },
    PlanUpdate { plan: PlanFile },
    PlanComplete { tool_call_id: String, title: String, task_count: usize },

    // Turn lifecycle
    TurnComplete { turn: usize, has_more: bool },
    Usage { prompt_tokens: usize, completion_tokens: usize },
    TitleGenerated { title: String },
    Finished { session_id: String },
    Error { error: String },
}
```

Define `LoopInput` — external inputs the platform provides back:

```rust
pub enum LoopInput {
    ToolApproval { tool_call_id: String, approved: bool },
    UserResponse { tool_call_id: String, response: String },
    Cancel,
}
```

### 1.2 Define the orchestrator configuration

**New file:** `crates/krusty-core/src/agent/orchestrator.rs`

```rust
pub struct OrchestratorConfig {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub permission_mode: PermissionMode,
    pub max_iterations: usize,         // default: 50
    pub user_id: Option<String>,
    pub initial_work_mode: WorkMode,
}
```

Separate from the services it needs:

```rust
pub struct OrchestratorServices {
    pub ai_client: Arc<AiClient>,
    pub tool_registry: Arc<ToolRegistry>,
    pub process_registry: Arc<ProcessRegistry>,
    pub db_path: PathBuf,
    pub skills_manager: Arc<RwLock<SkillsManager>>,
    pub mcp_manager: Arc<McpManager>,
}
```

### 1.3 Extract stream processing

**New file:** `crates/krusty-core/src/agent/stream.rs`

Move `process_stream()` from `chat.rs:1083-1219` into core. This function
consumes `StreamPart` events from `AiClient` and:
- Accumulates text, thinking blocks, tool calls
- Emits `LoopEvent`s via sender
- Handles stream timeout (120s)

```rust
pub(crate) async fn process_stream(
    api_rx: mpsc::UnboundedReceiver<StreamPart>,
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
) -> StreamResult {
    // Returns (text, thinking_blocks, tool_calls, finish_reason, token_count)
}
```

### 1.4 Extract tool execution

**New file:** `crates/krusty-core/src/agent/executor.rs`

Move `execute_tools()` from `chat.rs:1221-1488` into core. Handles:
- Permission-based approval workflow (supervised mode)
- Special tool dispatch (mode switch, plan tasks)
- Regular tool execution via `ToolRegistry::execute()`
- Output truncation
- Tool output streaming via `ToolOutputChunk` → `LoopEvent::ToolOutputDelta`

```rust
pub(crate) async fn execute_tools(
    ctx: &ExecutionContext<'_>,
    tool_calls: &[AiToolCall],
    event_tx: &mpsc::UnboundedSender<LoopEvent>,
    input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
) -> (Vec<Content>, WorkMode)
```

### 1.5 Extract plan + mode handlers

**New file:** `crates/krusty-core/src/agent/plan_handler.rs`

Move from `chat.rs:1490-1864`:
- `handle_mode_switch()` — set_work_mode / enter_plan_mode tool dispatch
- `handle_plan_task()` — task_start / task_complete / add_subtask / set_dependency
- `try_detect_plan()` — parse plan from AI response text

### 1.6 Extract failure detection

**New file:** `crates/krusty-core/src/agent/failure.rs`

Move from `chat.rs:1953-2328`:
- `detect_repeated_tool_failures()` — signature tracking, threshold detection
- `classify_error_code()` — error categorization
- `normalize_error_fingerprint()` — fingerprint normalization

### 1.7 Extract context building

**New file:** `crates/krusty-core/src/agent/context.rs`

Move from `krusty-cli/src/tui/handlers/streaming/context_building.rs`:
- `build_plan_context(plan_manager, session_id, work_mode)` — reads plan, formats task state
- `build_skills_context(skills_manager)` — lists skills as markdown
- `build_project_context(working_dir)` — reads KRAB.md / CLAUDE.md / .cursorrules

These become standalone functions (no `App` dependency) that take their data
sources as parameters.

```rust
pub fn build_plan_context(
    plan_manager: &PlanManager,
    session_id: &str,
    work_mode: WorkMode,
) -> String

pub fn build_skills_context(
    skills_manager: &RwLock<SkillsManager>,
) -> String

pub fn build_project_context(
    working_dir: &Path,
) -> String
```

### 1.8 Implement the orchestrator

**File:** `crates/krusty-core/src/agent/orchestrator.rs`

The main `AgenticOrchestrator` struct with its `run()` method. This is the
single canonical agentic loop that replaces:
- `chat.rs:run_agentic_loop()` (server)
- `acp_bridge.rs:run_agentic_loop_bridge()` (server v2)
- TUI's distributed event-driven loop

```rust
pub struct AgenticOrchestrator {
    services: OrchestratorServices,
    config: OrchestratorConfig,
    cancellation: AgentCancellation,
}

impl AgenticOrchestrator {
    pub fn new(
        services: OrchestratorServices,
        config: OrchestratorConfig,
    ) -> Self

    /// Start the agentic loop. Returns event stream + input channel.
    ///
    /// The loop runs as a spawned tokio task. It emits LoopEvents for
    /// every state change. The caller provides LoopInputs for user
    /// interaction (approvals, AskUser responses, cancellation).
    pub fn run(
        self,
        conversation: Vec<ModelMessage>,
        options: CallOptions,
    ) -> (
        mpsc::UnboundedReceiver<LoopEvent>,
        mpsc::UnboundedSender<LoopInput>,
    )
}
```

Internal run loop pseudocode:

```
fn run_inner(self, conversation, options, event_tx, input_rx):
    state = AgentState::new()
    failure_tracker = FailureTracker::new()
    work_mode = self.config.initial_work_mode

    for turn in 1..=self.config.max_iterations:
        // 1. Build context (plan, skills, project)
        injected = build_all_context(...)
        conversation_with_context = inject_system_messages(conversation, injected)

        // 2. Stream AI response
        api_rx = self.services.ai_client
            .call_streaming(conversation_with_context, &options)
        stream_result = process_stream(api_rx, &event_tx)

        // 3. Save assistant message
        save_message(...)
        conversation.push(assistant_message)

        // 4. No tool calls → check plan detection → finish
        if stream_result.tool_calls.is_empty():
            if work_mode == Plan:
                try_detect_plan(...)
            emit TurnComplete { has_more: false }
            break

        // 5. AskUser partition
        if has_ask_user_calls:
            execute non-ask-user tools
            emit AwaitingInput for each ask-user call
            return (loop pauses, resumes on LoopInput::UserResponse)

        // 6. Execute tools (with approval workflow)
        (results, new_mode) = execute_tools(
            tool_calls, event_tx, input_rx
        )
        work_mode = new_mode

        // 7. Check repeated failures
        if failure_tracker.check(tool_calls, results):
            emit Error
            break

        // 8. Add results to conversation, continue
        conversation.push(tool_results_message)
        emit TurnComplete { has_more: true }

    emit Finished
```

### 1.9 Wire up title generation

Title generation fires automatically after the first assistant response in a
new session. The orchestrator spawns a background task using the same AI client
with a fast model (Haiku) via `krusty_core::ai::title::generate_title()`.
On completion, emits `LoopEvent::TitleGenerated { title }`.

### 1.10 Update agent module exports

**File:** `crates/krusty-core/src/agent/mod.rs`

Add new submodules and public exports:

```rust
mod context;
mod executor;
mod failure;
mod orchestrator;
mod plan_handler;
mod stream;

pub use context::{build_plan_context, build_skills_context, build_project_context};
pub use orchestrator::{
    AgenticOrchestrator, OrchestratorConfig, OrchestratorServices,
    LoopEvent, LoopInput,
};
```

---

## Phase 2: Server Integration

> Replace the 2,328-line chat.rs monolith with thin HTTP handlers that
> delegate to the core orchestrator.

### 2.1 Rewrite chat handler

**File:** `crates/krusty-server/src/routes/chat.rs`

The new `chat()` handler:
1. Resolves session, loads conversation (existing logic, ~60 lines)
2. Creates `AgenticOrchestrator` from `AppState`
3. Calls `orchestrator.run(conversation, options)`
4. Maps `LoopEvent` → `AgenticEvent` (SSE) via `From` impl
5. Stores `input_tx` in session state for tool-result/tool-approval

```rust
async fn chat(
    State(state): State<AppState>,
    user: Option<CurrentUser>,
    Json(req): Json<ChatRequest>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let (session_id, conversation, options, work_mode) =
        setup_session(&state, &user, &req).await?;

    let orchestrator = AgenticOrchestrator::new(
        OrchestratorServices::from_app_state(&state),
        OrchestratorConfig {
            session_id: session_id.clone(),
            working_dir: state.working_dir.as_ref().clone(),
            permission_mode: req.permission_mode,
            max_iterations: 50,
            user_id: user.as_ref().map(|u| u.id.clone()),
            initial_work_mode: work_mode,
        },
    );

    let (event_rx, input_tx) = orchestrator.run(conversation, options);

    // Store input_tx so tool-result/tool-approval can send to it
    state.store_session_input(session_id, input_tx).await;

    let stream = ReceiverStream::new(event_rx).map(|event| {
        Ok(Event::default().json_data(&AgenticEvent::from(event)).unwrap())
    });

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}
```

### 2.2 Rewrite tool-result handler

```rust
async fn tool_result(
    State(state): State<AppState>,
    Json(req): Json<ToolResultRequest>,
) -> Result<..., AppError> {
    let input_tx = state.get_session_input(&req.session_id).await?;
    input_tx.send(LoopInput::UserResponse {
        tool_call_id: req.tool_call_id,
        response: req.result,
    })?;
    Ok(...)
}
```

### 2.3 Rewrite tool-approval handler

```rust
async fn tool_approval(
    State(state): State<AppState>,
    Json(req): Json<ToolApprovalRequest>,
) -> Result<..., AppError> {
    let input_tx = state.get_session_input(&req.session_id).await?;
    input_tx.send(LoopInput::ToolApproval {
        tool_call_id: req.tool_call_id,
        approved: req.approved,
    })?;
    Ok(...)
}
```

### 2.4 Session input channel management

**File:** `crates/krusty-server/src/lib.rs`

Add to `AppState`:

```rust
pub session_inputs: Arc<RwLock<HashMap<String, mpsc::UnboundedSender<LoopInput>>>>,
```

With methods `store_session_input()` and `get_session_input()`.

### 2.5 LoopEvent → AgenticEvent mapping

**File:** `crates/krusty-server/src/types.rs`

Implement `From<LoopEvent> for AgenticEvent` — this should be a near-1:1 mapping
since both types represent the same semantics. The server-side `AgenticEvent` stays
as the SSE wire format for backwards compatibility with the PWA frontend.

### 2.6 Delete acp_bridge.rs

The entire `acp_bridge.rs` file is replaced by the core orchestrator integration.
Remove it and its `pub mod acp_bridge` declaration from `lib.rs`.

### 2.7 Push notification integration

Push notifications are server-specific. The server subscribes to `LoopEvent`s and
fires push on:
- `LoopEvent::Finished` → completion notification
- `LoopEvent::AwaitingInput` → "needs your input" notification
- `LoopEvent::Error` → error notification

This stays in the server as a thin subscriber layer (~30 lines).

---

## Phase 3: TUI Integration

> Refactor the TUI to consume the core orchestrator instead of managing
> its own streaming state machine and tool execution.

### 3.1 Replace send_to_ai with orchestrator.run()

**File:** `crates/krusty-cli/src/tui/handlers/streaming/mod.rs`

The TUI's `send_to_ai()` currently:
1. Builds context (plan, skills, project)
2. Injects system messages
3. Creates streaming call
4. Manages `StreamingManager` state machine

Replace with:
1. Create `AgenticOrchestrator` from `App` state
2. Call `orchestrator.run()` — context injection happens inside core
3. Store `event_rx` for the event loop to poll
4. Store `input_tx` for approval/AskUser responses

### 3.2 Replace StreamingManager with LoopEvent consumer

**File:** `crates/krusty-cli/src/tui/streaming/state.rs`

The current `StreamPhase` state machine (Idle → Receiving → ReadyForTools →
Complete) is replaced by simply polling `event_rx`:

```rust
pub enum StreamPhase {
    Idle,
    Running {
        event_rx: mpsc::UnboundedReceiver<LoopEvent>,
        input_tx: mpsc::UnboundedSender<LoopInput>,
    },
}
```

### 3.3 Map LoopEvent → TUI blocks

**File:** `crates/krusty-cli/src/tui/handlers/stream_events.rs`

Map each `LoopEvent` to the appropriate TUI block:

| LoopEvent | TUI Action |
|-----------|------------|
| TextDelta | Append to streaming message |
| ThinkingDelta | Update ThinkingBlock |
| ToolCallStart | Create tool block |
| ToolExecuting | Show spinner on block |
| ToolOutputDelta | Stream to BashBlock/ToolResultBlock |
| ToolResult | Finalize block, show output |
| AwaitingInput | Show approval prompt popup |
| ModeChange | Update `ui.work_mode`, show indicator |
| PlanUpdate | Update plan sidebar |
| TurnComplete | Update turn counter |
| Usage | Update token display |
| TitleGenerated | Update session title |
| Finished | Transition to Idle |
| Error | Show error message |

### 3.4 Wire LoopInput from TUI interactions

When the TUI needs to send input back:
- **Tool approval popup** → `LoopInput::ToolApproval { ... }`
- **AskUser popup response** → `LoopInput::UserResponse { ... }`
- **Ctrl+C interruption** → `LoopInput::Cancel`

### 3.5 Remove TUI-side context building

**Delete:** The context building calls from `streaming/mod.rs:262-326` — this
now happens inside the orchestrator automatically.

**Delete:** `context_building.rs` methods that were moved to core — the TUI
module becomes a thin import of `krusty_core::agent::build_*_context` if it
still needs to display context info.

### 3.6 Remove TUI-side tool execution

**Slim down:** `streaming/tool_execution.rs` — the orchestrator handles all tool
execution internally. The TUI only needs to handle `LoopEvent`s. Remove:
- `handle_mode_switch_tools()`
- `handle_task_complete_tools()`
- `check_and_execute_tools()`
- The entire approval flow

---

## Phase 4: Unification Polish

> Final cleanup, consistency, and elegance.

### 4.1 Unified tool initialization function

**New function in core:** `create_full_tool_registry()`

Both TUI and server currently assemble their tool registries ad-hoc with
different subsets. Create a single canonical initialization:

```rust
pub async fn create_full_tool_registry(
    ai_client: Option<&Arc<AiClient>>,
    cancellation: &AgentCancellation,
    hook_manager: &Arc<RwLock<UserHookManager>>,
    mcp_manager: &Arc<McpManager>,
) -> Arc<ToolRegistry> {
    let mut registry = ToolRegistry::new();

    // Safety hooks (always)
    registry.add_pre_hook(Arc::new(SafetyHook::new()));
    registry.add_pre_hook(Arc::new(PlanModeHook::new()));
    registry.add_post_hook(Arc::new(LoggingHook::new()));

    // User hooks
    registry.add_pre_hook(Arc::new(UserPreToolHook::new(hook_manager.clone())));
    registry.add_post_hook(Arc::new(UserPostToolHook::new(hook_manager.clone())));

    // All base tools
    register_all_tools(&registry).await;

    // Sub-agent tools (require AI client)
    if let Some(client) = ai_client {
        register_explore_tool(&registry, client.clone(), cancellation.clone()).await;
        register_build_tool(&registry, client.clone(), cancellation.clone()).await;
    }

    // MCP tools
    register_mcp_tools(mcp_manager, &registry).await;

    Arc::new(registry)
}
```

Both TUI (`app_builder.rs`) and server (`lib.rs`) call this one function.

### 4.2 Unified session setup

Extract common session setup logic (load conversation, parse messages, resolve
model, build options) that both TUI and server duplicate:

```rust
pub fn load_conversation(
    session_manager: &SessionManager,
    session_id: &str,
) -> Result<Vec<ModelMessage>>

pub fn apply_thinking_config(
    ai_client: &AiClient,
    thinking_level: ThinkingLevel,
    options: &mut CallOptions,
)
```

### 4.3 Consolidate ThinkingLevel

The server has its own `ThinkingLevel` enum in `types.rs`. The TUI has a
different one. Move a canonical `ThinkingLevel` to core:

```rust
// krusty-core/src/ai/thinking.rs
pub enum ThinkingLevel { Off, Low, Medium, High, XHigh }
```

Both server and TUI import from core.

### 4.4 Delete dead code

- Delete `crates/krusty-server/src/acp_bridge.rs` entirely
- Gut `crates/krusty-server/src/routes/chat.rs` from 2,328 → ~250 lines
- Remove duplicated helpers: `truncate_output`, `build_assistant_message`,
  `save_message`, `set_agent_state`, `detect_repeated_tool_failures`,
  `try_detect_plan`, `handle_mode_switch_tool_call`, `handle_plan_task_tool_call`
- Remove TUI-side `context_building.rs` (moved to core)
- Remove TUI-side tool execution handlers (replaced by orchestrator events)

### 4.5 Ensure ACP mode also uses the orchestrator

The ACP mode (`krusty-core/src/acp/`) should also be updated to use
`AgenticOrchestrator`, making it the third consumer of the same core loop.
This ensures editor integrations (Zed, Neovim, JetBrains) have full parity too.

---

## Phase 5: Extended Capabilities

> Features that become trivial once the orchestrator exists.

### 5.1 Auto-pinch (context summarization)

The orchestrator can track conversation size and automatically trigger
summarization when approaching context limits. Currently TUI-only via
`poll_auto_pinch()`. With the orchestrator, this becomes a core capability
available to all platforms.

### 5.2 Agent introspection API

With `LoopEvent` as the canonical event stream, add a server endpoint that
exposes real-time agent state:

```
GET /api/sessions/:id/events → SSE stream of LoopEvents
```

This enables external tooling, dashboards, and monitoring.

### 5.3 Multi-session orchestration

The orchestrator's clean separation enables future multi-session features:
session forking, parallel branches, and session-to-session handoff — all
using the same event protocol.

---

## Dependency Graph

```
Phase 0  (quick fixes — no dependencies, do first)
  ├─ 0.1  Register explore/build tools
  ├─ 0.2  Wire user hooks
  ├─ 0.3  Register MCP tools
  └─ 0.4  Add context injection
         │
Phase 1  (core extraction — foundation for everything)
  ├─ 1.1  Define LoopEvent / LoopInput
  ├─ 1.2  Define OrchestratorConfig / Services
  ├─ 1.3  Extract stream processing
  ├─ 1.4  Extract tool execution
  ├─ 1.5  Extract plan + mode handlers
  ├─ 1.6  Extract failure detection
  ├─ 1.7  Extract context building
  ├─ 1.8  Implement orchestrator  ← depends on 1.1-1.7
  ├─ 1.9  Wire title generation
  └─ 1.10 Update module exports
         │
    ┌────┴────┐
    ▼         ▼
Phase 2    Phase 3  (can run in parallel)
Server     TUI
  ├─ 2.1     ├─ 3.1
  ├─ 2.2     ├─ 3.2
  ├─ 2.3     ├─ 3.3
  ├─ 2.4     ├─ 3.4
  ├─ 2.5     ├─ 3.5
  ├─ 2.6     └─ 3.6
  └─ 2.7
         │
         ▼
Phase 4  (cleanup — depends on 2 + 3)
  ├─ 4.1  Unified tool init
  ├─ 4.2  Unified session setup
  ├─ 4.3  Consolidate ThinkingLevel
  ├─ 4.4  Delete dead code
  └─ 4.5  ACP integration
         │
         ▼
Phase 5  (extensions — depends on 4)
  ├─ 5.1  Auto-pinch
  ├─ 5.2  Introspection API
  └─ 5.3  Multi-session
```

---

## File Impact Summary

### New Files (in krusty-core)

| File | Purpose | Est. Lines |
|------|---------|------------|
| `agent/orchestrator.rs` | Main agentic loop | ~400 |
| `agent/stream.rs` | Stream processing | ~150 |
| `agent/executor.rs` | Tool execution dispatch | ~300 |
| `agent/plan_handler.rs` | Plan + mode tool handling | ~250 |
| `agent/failure.rs` | Repeated failure detection | ~120 |
| `agent/context.rs` | Context building (plan/skills/project) | ~200 |

### Modified Files

| File | Change | Impact |
|------|--------|--------|
| `krusty-core/src/agent/mod.rs` | Add new module exports | Small |
| `krusty-core/src/agent/events.rs` | Add LoopEvent, LoopInput | Medium |
| `krusty-server/src/lib.rs` | Unified tool init, session inputs | Medium |
| `krusty-server/src/routes/chat.rs` | 2,328 → ~250 lines | Large |
| `krusty-server/src/types.rs` | Add From<LoopEvent> impl | Small |
| `krusty-cli/src/tui/handlers/streaming/mod.rs` | Use orchestrator | Large |
| `krusty-cli/src/tui/streaming/state.rs` | Simplified state machine | Medium |
| `krusty-cli/src/tui/handlers/stream_events.rs` | Map LoopEvent → blocks | Medium |
| `krusty-cli/src/tui/app_builder.rs` | Use create_full_tool_registry | Small |

### Deleted Files

| File | Reason |
|------|--------|
| `krusty-server/src/acp_bridge.rs` | Replaced by core orchestrator |
| `krusty-cli/src/.../context_building.rs` | Moved to core |

### Net Line Count

| Area | Before | After | Delta |
|------|--------|-------|-------|
| krusty-core agent/ | ~600 | ~2,020 | +1,420 |
| krusty-server chat.rs | 2,328 | ~250 | -2,078 |
| krusty-server acp_bridge.rs | 1,152 | 0 | -1,152 |
| krusty-cli streaming/ | ~1,500 | ~600 | -900 |
| **Total** | **~5,580** | **~2,870** | **-2,710** |

The codebase gets **smaller** while gaining full platform parity.

---

## Verification Criteria

After each phase, these must pass:

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace
```

### Functional Verification

Phase 0:
- [ ] Server tool list includes `explore` and `build`
- [ ] User hooks fire on server tool execution
- [ ] MCP tools appear in server's AI tool list
- [ ] Server injects plan/skills/project context

Phase 1:
- [ ] `AgenticOrchestrator::run()` compiles and emits correct LoopEvents
- [ ] Unit tests for stream processing, failure detection, context building

Phase 2:
- [ ] PWA chat works end-to-end via orchestrator
- [ ] Tool approval works via LoopInput channel
- [ ] AskUser flow works via LoopInput channel
- [ ] Plan detection and confirmation works
- [ ] Title generation works

Phase 3:
- [ ] TUI chat works end-to-end via orchestrator
- [ ] All block types render correctly from LoopEvents
- [ ] Ctrl+C cancellation works
- [ ] Tool approval popup works

Phase 4:
- [ ] No duplicated orchestration logic remains
- [ ] All three consumers (TUI, server, ACP) use the same core
- [ ] acp_bridge.rs deleted
- [ ] chat.rs under 300 lines
