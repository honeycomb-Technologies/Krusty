# Krusty Mako — Complete System Roadmap

## Vision

Mako transforms Krusty from a reactive coding assistant into a persistent, always-on development daemon. Named after the mako shark — an obligate ram ventilator that never stops swimming.

Three session modes:

- **Chat** — Conversational AI with web search and toggleable research (produces Reports). No file tools. Like claude.ai or ChatGPT.
- **Code** — Full agentic coding assistant. Current Krusty experience. TUI-first.
- **Mako** — Autonomous daemon. Dispatched tasks, tick engine, swarm of teammates, auto-classifier. Fire and forget.

Research is a capability within Chat and Mako, not a separate mode. When research completes, it produces **Reports** — structured markdown documents stored in `.krusty/reports/` that persist as project knowledge.

```
┌──────────────────────────────────────────────────────────────────┐
│                        Mako Daemon                               │
│                        (always running)                          │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Session Manager                                            │  │
│  │  Chat sessions   (conversation + web search + research)    │  │
│  │  Code sessions   (full tools, orchestrator, plans)         │  │
│  │  Mako sessions   (tick engine + swarm + classifier)        │  │
│  └────────────────────────────────────────────────────────────┘  │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────────┐ │
│  │ Dispatch     │  │ Project      │  │ Report Store           │ │
│  │ Queue        │  │ Registry     │  │ (.krusty/reports/)     │ │
│  └──────────────┘  └──────────────┘  └────────────────────────┘ │
│                                                                  │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────────────┐ │
│  │ Swarm        │  │ Tick         │  │ Auto-Classifier        │ │
│  │ Manager      │  │ Engines      │  │ (safety gate)          │ │
│  └──────────────┘  └──────────────┘  └────────────────────────┘ │
│                                                                  │
│  ┌────────────────────────────────────────────────────────────┐  │
│  │ Transport: Unix socket + HTTP/SSE + ACP stdio              │  │
│  └────────────────────────────────────────────────────────────┘  │
└──────────────────────────────────────────────────────────────────┘
       ▲           ▲            ▲           ▲          ▲
   ┌───┴──┐   ┌───┴───┐   ┌───┴───┐   ┌───┴──┐   ┌──┴───┐
   │ TUI  │   │ Expo  │   │  Zed  │   │Tauri │   │ CLI  │
   │(code)│   │(chat+ │   │  ACP  │   │(desk)│   │ mako │
   │      │   │ mako) │   │       │   │      │   │      │
   └──────┘   └───────┘   └───────┘   └──────┘   └──────┘
```

## Existing Infrastructure

- **Orchestrator**: `AgenticOrchestrator` with `LoopEvent`/`LoopInput` protocol
- **Tool System**: `ToolRegistry`, `Tool` trait, `ToolPolicy`, hooks
- **SubAgent System**: `SubAgentPool`, `DelegatedRunStore`, explore/build/plan/verify
- **ACP Server**: Permission forwarding, heartbeat, plan streaming (just shipped)
- **HTTP Server** (`krusty-server`): REST API + SSE for sessions, chat, tools, models, git, files, credentials
- **Expo App** (`apps/mobile`): Chat UI with streaming, session mgmt, voice. **Stub tabs for Chat and Mako already exist.**
- **Tauri Desktop** (`apps/desktop/shell`): Wraps frontend, embeds krusty-server
- **Shared API Client** (`packages/api`): `KrustyClient` with HTTP+SSE
- **Storage**: SQLite with sessions, messages, memories, plans, delegated runs, credentials

---

## Phase 0 — Daemon + Session Types + Chat Mode

**Delivers**: Daemon process, session type system, Chat mode in Expo app.

### 0A. Session Type System

```rust
// storage/sessions.rs
pub enum SessionType {
    Chat,   // conversation + web search + research toggle
    Code,   // full tools, current orchestrator
    Mako,   // autonomous tick engine + swarm
}
```

- `POST /api/sessions` accepts `session_type`
- Session type determines tool registration
- Chat: web_search, web_fetch (+ research tools when toggled)
- Code: all current tools
- Mako: all tools + SendUserMessage + Sleep + task + teammate tools

### 0B. Chat Mode

Chat mode = orchestrator with restricted tools.

```rust
// tools/chat_tools.rs
pub async fn register_chat_tools(registry: &ToolRegistry) {
    // web_search, web_fetch only
    // When research toggled: add research agent tool
}
```

### 0C. Daemon Process

```
krusty daemon start     Start daemon (singleton)
krusty daemon stop      Stop daemon
krusty daemon status    Health + running sessions
```

Evolves `krusty-server` into always-running process with PID management, project registry, session ownership. Tauri connects to daemon instead of managing its own server.

### Expo App Changes
- Wire Chat tab stub → create `session_type: "chat"` sessions
- Wire Mako tab stub → create `session_type: "mako"` sessions
- Session drawer shows type badges
- Research toggle button in ChatBar (Chat mode only)

---

## Phase 1 — Foundation Tools (SendUserMessage + Sleep)

**Delivers**: Explicit output channel and idle signaling for Mako.

- `SendUserMessage`: `{ message, title?, level? }` → `LoopEvent::UserMessage`
- `Sleep`: `{ duration_secs?, reason? }` → signal for tick engine to pause

New SSE events: `user_message`, `agent_sleeping`
Expo app: Render UserMessage as highlighted card in Mako view

---

## Phase 2 — Tick Engine

**Delivers**: Proactive loop. Mako sessions keep working between user messages.

`TickEngine::run()` wraps orchestrator. On `Finished(Completed)`, waits tick interval, injects `<tick>`, starts new run. On Sleep signal, pauses. On Cancel, stops.

Server creates TickEngine for Mako sessions. SSE stream stays open across ticks.

New: `LoopEvent::TickInjected`, `LoopStopReason::Sleeping`

Config: `.krusty/settings.json` → `mako.tick_interval_secs`, `mako.max_ticks`

---

## Phase 3 — Auto-Classifier

**Delivers**: AI safety gate for Mako tool execution.

`PreToolHook` implementation. Two-stage: fast (64 tokens) then thinking (4096 tokens). Safe tool allowlist bypasses. Falls back to deny on error.

Registered on ToolRegistry for Mako sessions only.

New: `LoopEvent::ClassifierDecision`

---

## Phase 4 — Task List

**Delivers**: Lightweight work tracking for Mako coordination.

SQLite `autonomous_tasks` table. Tools: `CreateTask`, `UpdateTask`, `ListTasks`. Context injection shows pending/active/completed work.

Server: `GET /api/sessions/:id/tasks`
Expo: Task list panel in Mako view

---

## Phase 5 — Team / Swarm

**Delivers**: Named teammates with independent cancellation.

`TeamManager` spawns tokio tasks that auto-claim from task list. Each teammate has its own `CancellationToken` and `DelegationPolicy`.

Tool: `Teammate { action: "spawn"|"cancel"|"list", name, role, delegation }`

New events: `TeammateSpawned`, `TeammateTaskCompleted`, `TeammateTaskFailed`, `TeammateCancelled`

Server: `GET /api/sessions/:id/teammates`
Expo: Teammate status cards

---

## Phase 6 — Coordinator Prompt

**Delivers**: System prompt making AI act as project coordinator.

Injected when `session_type == Mako`. Defines phases (Research → Synthesis → Implementation → Verification), communication rules, delegation patterns.

---

## Phase 7 — Dispatch Interface

**Delivers**: Fire-and-forget task submission from any surface.

### CLI
```
krusty mako "refactor the auth module"     Submit task
krusty mako status                          Show running sessions
krusty mako pause / resume / cancel         Control
```

### Server
```
POST   /api/mako/dispatch                   Submit task
GET    /api/mako/sessions                   List Mako sessions
POST   /api/mako/sessions/:id/pause
POST   /api/mako/sessions/:id/resume
DELETE /api/mako/sessions/:id               Cancel
GET    /api/mako/sessions/:id/events        SSE observe stream
```

### Expo App
- Mako tab: dispatch input + session list
- Push notifications on milestones

---

## Phase 8 — Research + Reports

**Delivers**: Deep investigation that produces structured, persistent documents.

### Research Flow
1. User toggles research ON in Chat, or Mako decides to research
2. Spawns explorer agents (existing `SubAgentPool`)
3. Optionally does web search
4. Synthesizes findings into a Report
5. Stores in `.krusty/reports/` and SQLite

### Report Structure
```markdown
---
title: "Authentication Architecture Analysis"
created: 2026-04-01T10:30:00Z
session_id: "sess_abc123"
tags: ["auth", "architecture"]
---

## Summary
## Analysis
## Recommendations
## Sources
```

### Storage
```rust
pub struct Report {
    pub id: String,
    pub title: String,
    pub session_id: String,
    pub content: String,    // full markdown
    pub summary: String,    // for context injection
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub created_at: DateTime<Utc>,
}
```

Reports also written to `.krusty/reports/{date}-{slug}.md` for direct access.

### Context Injection
Recent report summaries injected into context when relevant (tag/title matching). Full content via `ReadReport` tool.

### Tools
- `CreateReport` — persist research findings
- `ListReports` — query existing
- `ReadReport` — load full report

### Server + Client
- `GET /api/reports`, `GET /api/reports/:id`
- Expo: Reports list view, markdown rendering, "Research" toggle in ChatBar

---

## Phase Dependencies

```
Phase 0 (Daemon + Session Types + Chat)
    │
    ├── Phase 1 (SendUserMessage + Sleep)
    │       │
    │       ├── Phase 2 (Tick Engine)
    │       │       │
    │       │       ├── Phase 3 (Classifier)    ← can parallel with 4
    │       │       │
    │       │       └── Phase 4 (Task List)     ← can parallel with 3
    │       │               │
    │       │               └── Phase 5 (Swarm)
    │       │                       │
    │       │                       └── Phase 6 (Coordinator)
    │       │                               │
    │       │                               └── Phase 7 (Dispatch)
    │       │
    │       └── Phase 8 (Research + Reports)    ← can parallel with 2-7
    │
    └── Expo app wiring (continuous alongside each phase)
```

---

## Verification

After each phase:
```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace
```
