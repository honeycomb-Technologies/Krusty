# Krusty Platform Unification Audit

## Executive Summary

The TUI and Server/PWA have **diverged significantly**. While they share the same `krusty-core` crate for tools, AI client, and storage, the **orchestration layer** (how the agentic loop runs, what context gets injected, what capabilities are available) is duplicated and inconsistent. The server has a 2,328-line monolithic `chat.rs` that reimplements core logic instead of reusing it.

---

## Architecture Diagrams

### Current State: Two Separate Brains

```
                    ┌─────────────────────────────────────────────┐
                    │              krusty-core                     │
                    │                                             │
                    │  ┌──────────┐ ┌──────────┐ ┌────────────┐  │
                    │  │ AiClient │ │ToolReg.  │ │ Storage    │  │
                    │  │ stream() │ │ execute()│ │ sessions   │  │
                    │  └────┬─────┘ └────┬─────┘ └─────┬──────┘  │
                    │       │            │             │          │
                    │  ┌────┴────┐ ┌─────┴─────┐ ┌────┴───┐     │
                    │  │Providers│ │16+ Tools   │ │Plans   │     │
                    │  │Anthropic│ │bash,read,  │ │Skills  │     │
                    │  │OpenAI   │ │edit,write, │ │MCP     │     │
                    │  │etc.     │ │grep,glob...│ │Hooks   │     │
                    │  └─────────┘ └───────────┘ └────────┘     │
                    └───────────────┬───────────────┬────────────┘
                                    │               │
                    ┌───────────────┴──┐   ┌───────┴───────────────┐
                    │   TUI (krusty-cli)│   │  Server (krusty-server)│
                    │                   │   │                        │
                    │ ▸ Event-driven    │   │ ▸ Imperative loop      │
                    │   streaming       │   │   (run_agentic_loop)   │
                    │ ▸ StreamingMgr    │   │ ▸ Inline in chat.rs    │
                    │   state machine   │   │   (2,328 lines!)       │
                    │ ▸ Context inject: │   │ ▸ Context inject:      │
                    │   - plan_context  │   │   ❌ NONE              │
                    │   - skills_context│   │                        │
                    │   - project_ctx   │   │ ▸ Missing:             │
                    │ ▸ explore/build   │   │   ❌ explore tool      │
                    │   sub-agents      │   │   ❌ build tool        │
                    │ ▸ WASM extensions │   │   ❌ WASM extensions   │
                    │ ▸ Plugins         │   │   ❌ plugins           │
                    │ ▸ Title gen       │   │   ❌ title generation  │
                    │ ▸ Auto-pinch      │   │   ❌ auto-pinch       │
                    │ ▸ Thinking levels │   │   ⚠️  partial          │
                    │ ▸ User hooks (pre │   │   ⚠️  hooks loaded but │
                    │   + post + custom)│   │      not on registry   │
                    └───────────────────┘   └────────────────────────┘
```

### Target State: Unified Core Orchestration

```
                    ┌─────────────────────────────────────────────┐
                    │              krusty-core                     │
                    │                                             │
                    │  ┌───────────────────────────────────────┐  │
                    │  │         AgenticLoop (NEW)             │  │
                    │  │                                       │  │
                    │  │  ▸ Runs streaming + tool execution    │  │
                    │  │  ▸ Context injection (plan, skills,   │  │
                    │  │    project)                           │  │
                    │  │  ▸ AskUser handling                   │  │
                    │  │  ▸ Plan detection                     │  │
                    │  │  ▸ Exploration budget                 │  │
                    │  │  ▸ Repeated failure detection         │  │
                    │  │  ▸ Title generation                   │  │
                    │  │  ▸ Emits AgenticEvent stream          │  │
                    │  └───────┬───────────────────┬───────────┘  │
                    │          │                   │              │
                    │  ┌───────┴──┐  ┌─────────┐ ┌┴──────────┐   │
                    │  │ AiClient │  │ToolReg. │ │ Storage   │   │
                    │  │ stream() │  │execute()│ │ sessions  │   │
                    │  └──────────┘  └─────────┘ └───────────┘   │
                    └───────────────┬───────────────┬─────────────┘
                                    │               │
                    ┌───────────────┴──┐   ┌───────┴───────────────┐
                    │   TUI (thin)     │   │  Server (thin)         │
                    │                  │   │                        │
                    │ ▸ Consumes       │   │ ▸ Consumes             │
                    │   AgenticEvent   │   │   AgenticEvent         │
                    │ ▸ Renders blocks │   │ ▸ Forwards as SSE      │
                    │ ▸ Handles input  │   │ ▸ Handles HTTP/WS      │
                    │ ▸ UI only        │   │ ▸ Transport only       │
                    └──────────────────┘   └────────────────────────┘
```

---

## Gap Analysis: Feature Comparison

| Capability                      | TUI | Server/PWA | Gap Severity |
|---------------------------------|-----|------------|--------------|
| **Core AI streaming**           | ✅  | ✅         | None         |
| **16 base tools**               | ✅  | ✅         | None         |
| **explore sub-agent tool**      | ✅  | ❌         | **CRITICAL** |
| **build sub-agent tool**        | ✅  | ❌         | **CRITICAL** |
| **Plan context injection**      | ✅  | ❌         | **HIGH**     |
| **Skills context injection**    | ✅  | ❌         | **HIGH**     |
| **Project context injection**   | ✅  | ❌         | **HIGH**     |
| **Title generation (Haiku)**    | ✅  | ❌         | Medium       |
| **WASM extensions**             | ✅  | ❌         | Medium       |
| **Plugin system**               | ✅  | ❌         | Medium       |
| **Auto-pinch (summarization)**  | ✅  | ❌         | Medium       |
| **User hooks on registry**      | ✅  | ⚠️ loaded   | **HIGH**     |
| **MCP tool registration**       | ✅  | ⚠️ partial  | **HIGH**     |
| **Thinking level config**       | ✅  | ⚠️ partial  | Medium       |
| **Session persistence**         | ✅  | ✅         | None         |
| **Plan mode + task tracking**   | ✅  | ✅         | None         |
| **Permission mode (supervised)**| ✅  | ✅         | None         |
| **Tool approval workflow**      | ✅  | ✅         | None         |
| **Push notifications**          | N/A | ✅         | N/A          |

### Critical Gaps Explained

#### 1. Missing explore/build tools (CRITICAL)
The TUI registers `explore` and `build` tools after creating an AI client. These are the most powerful tools — they spawn sub-agent pools for parallel codebase exploration and multi-file builds. The server **never calls `register_explore_tool()` or `register_build_tool()`**, so the PWA AI literally cannot use these capabilities.

**Impact:** PWA users get a fundamentally weaker agent that can't do parallel exploration or coordinated multi-file builds.

#### 2. No context injection (HIGH)
The TUI injects three types of context as system messages before each AI call:
- **Plan context**: Current plan state, task statuses, dependencies
- **Skills context**: Loaded skill definitions and instructions
- **Project context**: Diagnostics, project configuration hints

The server injects **none of this**. The AI on the PWA is flying blind — it doesn't know about the active plan, available skills, or project-specific context.

#### 3. User hooks not wired to registry (HIGH)
The server loads `UserHookManager` but never adds `UserPreToolHook`/`UserPostToolHook` to the `ToolRegistry`'s hook chain. User-configured hooks simply don't fire.

#### 4. MCP tools not registered (HIGH)
The server creates an `McpManager` and connects to MCP servers, but never calls `register_mcp_tools()` to add those tools to the `ToolRegistry`. MCP tools exist but are invisible to the AI.

---

## Code Duplication Analysis

### Duplicated Logic Between chat.rs and acp_bridge.rs

The codebase already has **two** agentic loop implementations in the server:
1. `routes/chat.rs` — 2,328 lines, the original monolith
2. `acp_bridge.rs` — 1,152 lines, a cleaner rewrite (in progress)

Both duplicate:
- Stream processing (`process_stream` / `process_stream_bridge`)
- Tool execution (`execute_tools` / `execute_tools_bridge`)
- Plan detection (`try_detect_plan`)
- AskUser handling (`handle_ask_user`)
- Repeated failure detection
- Message building
- Agent state management
- Push notifications

### Duplicated Logic Between TUI and Server

| Logic                        | TUI Location                                   | Server Location                    |
|------------------------------|-----------------------------------------------|------------------------------------|
| Agentic loop                 | `streaming/mod.rs` (event-driven)             | `chat.rs:669-1076` (imperative)    |
| Stream processing            | `streaming/state.rs` (state machine)          | `chat.rs:1083-1219`               |
| Tool execution               | `streaming/tool_execution.rs`                 | `chat.rs:1221-1488`               |
| Context building             | `streaming/context_building.rs`               | **MISSING**                        |
| Plan detection               | `streaming/mod.rs`                            | `chat.rs:758-833`                  |
| Mode switching               | `streaming/tool_execution.rs`                 | `chat.rs:1490-1614`               |
| Plan task handling            | `streaming/tool_execution.rs`                 | `chat.rs:1616-1864`               |
| AskUser flow                 | `streaming/tool_execution.rs`                 | `chat.rs:853-937`                  |
| Repeated failure detection   | `streaming/tool_execution.rs`                 | `chat.rs:981-1046`                |
| Title generation             | `handlers/title.rs`                           | **MISSING**                        |
| Explore/build registration   | `handlers/provider.rs:59-82`                  | **MISSING**                        |

---

## Server Monolith Analysis

### chat.rs Structural Breakdown (2,328 lines)

```
Lines     Purpose                              Should Live In
──────────────────────────────────────────────────────────────
1-66      Imports, constants, router            routes/chat.rs (thin)
68-96     Context structs                       core: AgenticLoopConfig
98-133    ExecuteToolsContext                   core: internal
134-264   Session locking + message parsing     core: SessionManager
265-315   chat() handler entry point            routes/chat.rs (thin)
316-488   chat() spawning agentic loop          core: AgenticLoop
490-620   tool_result() handler                 routes/chat.rs (thin)
622-632   tool_approval() handler               routes/chat.rs (thin)
634-667   AgenticLoopContext builder             core: AgenticLoopConfig
669-1076  run_agentic_loop() ← THE CORE LOOP   core: AgenticLoop::run()
1083-1219 process_stream()                      core: StreamProcessor
1221-1488 execute_tools()                       core: ToolExecutor
1490-1614 handle_mode_switch_tool_call()        core: ModeHandler
1616-1864 handle_plan_task_tool_call()          core: PlanTaskHandler
1866-1922 helper functions                      core: utils
1924-1951 agent state + token DB operations     core: SessionManager
1953-2065 repeated failure detection            core: FailureDetector
2067-2160 plan detection                        core: PlanDetector
2161-2328 thinking config, title gen, push      core: various
```

**Verdict:** ~1,800 lines of chat.rs should be in `krusty-core`, leaving only ~200 lines of HTTP handler glue in the server.

---

## Refactoring Plan

### Phase 1: Core AgenticLoop Extraction

Create `krusty-core/src/agent/loop.rs` — a reusable agentic loop that both TUI and server consume.

```rust
// krusty-core/src/agent/loop.rs

pub struct AgenticLoop {
    ai_client: Arc<AiClient>,
    tool_registry: Arc<ToolRegistry>,
    process_registry: Arc<ProcessRegistry>,
    session_manager: SessionManager,
    plan_manager: PlanManager,
    skills_manager: Arc<RwLock<SkillsManager>>,
    mcp_manager: Arc<McpManager>,
    config: AgenticLoopConfig,
}

pub struct AgenticLoopConfig {
    pub session_id: String,
    pub working_dir: PathBuf,
    pub permission_mode: PermissionMode,
    pub max_iterations: usize,
    pub thinking_level: ThinkingLevel,
    pub user_id: Option<String>,
}

/// Events emitted by the loop — consumed by TUI or server transport
pub enum LoopEvent {
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallComplete { id: String, name: String, arguments: Value },
    ToolExecuting { id: String, name: String },
    ToolOutputDelta { id: String, delta: String },
    ToolResult { id: String, output: String, is_error: bool },
    AwaitingInput { tool_call_id: String, tool_name: String },
    ModeChange { mode: WorkMode, reason: Option<String> },
    PlanUpdate { items: Vec<PlanItem> },
    PlanComplete { tool_call_id: String, title: String },
    TurnComplete { turn: usize, has_more: bool },
    Usage { prompt_tokens: usize, completion_tokens: usize },
    Finished,
    Error(String),
}

/// External input the loop needs from the platform
pub enum LoopInput {
    ToolApproval { tool_call_id: String, approved: bool },
    ToolResult { tool_call_id: String, result: String },
    Cancel,
}

impl AgenticLoop {
    pub fn run(
        &self,
        conversation: Vec<ModelMessage>,
        options: CallOptions,
    ) -> (mpsc::UnboundedReceiver<LoopEvent>, mpsc::UnboundedSender<LoopInput>) {
        // Returns event stream + input channel
        // Loop runs as spawned task
    }
}
```

### Phase 2: Context Injection (Move from TUI to Core)

Move these from `krusty-cli/src/tui/handlers/streaming/context_building.rs` to `krusty-core/src/agent/context.rs`:

- `build_plan_context()` — reads active plan, formats task list
- `build_skills_context()` — reads loaded skills, formats instructions
- `build_project_context()` — reads diagnostics, project hints

The `AgenticLoop` calls these automatically before each AI turn.

### Phase 3: Fix Server Tool Registration

In `krusty-server/src/lib.rs::build_router()`, add:

```rust
// After register_all_tools:
if let Some(ref client) = ai_client {
    register_explore_tool(&tool_registry, client.clone(), cancellation.clone()).await;
    register_build_tool(&tool_registry, client.clone(), cancellation.clone()).await;
}

// Wire user hooks to registry:
tool_registry.add_pre_hook(Arc::new(UserPreToolHook::new(hook_manager.clone())));
tool_registry.add_post_hook(Arc::new(UserPostToolHook::new(hook_manager.clone())));

// Register MCP tools:
register_mcp_tools(&mcp_manager, &tool_registry).await;
```

### Phase 4: Thin Server Layer

Reduce `krusty-server/src/routes/chat.rs` to ~200 lines:

```rust
pub async fn chat(State(state): State<AppState>, req: ChatRequest) -> Sse<...> {
    let loop_instance = AgenticLoop::new(/* from state */);
    let (event_rx, input_tx) = loop_instance.run(conversation, options);

    // Store input_tx for tool-result/tool-approval endpoints

    // Map LoopEvent → SSE AgenticEvent (trivial 1:1 mapping)
    let sse_stream = ReceiverStream::new(event_rx).map(|event| {
        Ok(Event::default().json_data(&AgenticEvent::from(event)).unwrap())
    });

    Sse::new(sse_stream)
}
```

### Phase 5: Thin TUI Consumer

Refactor TUI's `StreamingManager` to consume `LoopEvent` instead of raw `StreamPart`:

```rust
// Instead of managing streaming state machine directly:
let (event_rx, input_tx) = agentic_loop.run(conversation, options);

// TUI just renders events:
match event {
    LoopEvent::TextDelta(d) => append_to_message(d),
    LoopEvent::ToolExecuting { id, name } => show_tool_block(name),
    LoopEvent::AwaitingInput { .. } => show_approval_prompt(),
    // etc.
}
```

### Phase 6: Cleanup

- Delete `acp_bridge.rs` (replaced by core loop)
- Delete 1,800+ lines from `chat.rs`
- Remove all duplicated helper functions
- Add title generation to the core loop
- Add auto-pinch support to the core loop

---

## Migration Order

```
Phase 1: Core AgenticLoop          ← Foundation, biggest value
  └─ Phase 2: Context injection    ← Immediate parity win
  └─ Phase 3: Fix tool registration ← Quick fix, huge impact
Phase 4: Thin server               ← Depends on Phase 1
Phase 5: Thin TUI                  ← Depends on Phase 1
Phase 6: Cleanup                   ← Final polish
```

**Estimated complexity:** Phase 1 is the hardest (extracting the loop with proper abstraction). Phases 2-3 are medium. Phases 4-6 are mechanical refactoring.

---

## Quick Wins (Can Do Before Full Refactor)

These can be done independently to close the worst gaps immediately:

1. **Register explore + build tools on server** — 10 lines in `lib.rs`
2. **Wire user hooks to tool registry** — 4 lines in `lib.rs`
3. **Register MCP tools** — 5 lines in `lib.rs`
4. **Copy context building to server** — Port `context_building.rs` logic to `chat.rs`/`acp_bridge.rs`
5. **Add title generation** — Call `generate_title()` after first assistant message

These quick wins alone would dramatically improve PWA parity without the full refactor.
