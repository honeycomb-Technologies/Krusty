# Storage & Persistence

Mitsuro uses SQLite as its persistence layer. Everything that needs to survive between sessions -- conversations, plans, credentials, preferences, agent state -- lives in a single database file on disk. There is no external database server to install, no connection string to configure, and no network dependency. The binary ships with SQLite compiled in via the `rusqlite` crate, so persistence works out of the box the moment you run Mitsuro for the first time.

## Why SQLite

The decision to use SQLite comes down to three properties that align with how Mitsuro is designed.

**Local-first.** Mitsuro runs on your machine, not in the cloud. SQLite is an embedded database -- it lives inside the process, reads and writes directly to a file, and never opens a network socket. This means Mitsuro works offline, on airplanes, on machines with no internet at all.

**Zero-config.** There is no setup step. The first time Mitsuro starts, it creates the database file automatically, runs all schema migrations, and is ready to go. Users never interact with the database directly.

**Single-file.** The entire database is one file: `~/.krusty/krusty.db`. Backing it up means copying one file. Moving it to another machine means copying one file. Deleting it resets everything cleanly. This makes the operational model as simple as it gets.

SQLite also brings WAL (Write-Ahead Logging) mode, which Mitsuro enables on every connection. WAL allows concurrent reads while a write is in progress, preventing lock contention when the server and CLI access the database simultaneously. A 5-second busy timeout is configured so that brief lock conflicts resolve automatically rather than failing immediately.

## The Database Wrapper

All database access flows through the `Database` struct in `crates/krusty-core/src/storage/database.rs`. This wrapper handles three responsibilities:

1. **Connection setup.** It opens the SQLite connection, enables WAL mode, turns on foreign key enforcement, and sets the busy timeout.
2. **Migration management.** It tracks the current schema version and runs any outstanding migrations.
3. **Shared access.** It provides a `SharedDatabase` type (`Arc<Mutex<Database>>`) so multiple components can safely share a single connection.

Creating a database is straightforward:

```rust
let db = Database::new(&path)?;
```

The constructor creates the parent directory if it does not exist, opens the connection, configures pragmas, and runs migrations. Everything happens in that one call.

For components that need to share a connection (like the server, where multiple request handlers access the same database), `Database::shared()` wraps the instance in an `Arc<Mutex<_>>`:

```rust
let shared_db = Database::shared(&path)?;
```

### Versioned Migrations

The schema evolves through numbered migrations. A `schema_version` table tracks which migrations have been applied. On startup, `Database` compares the current version against the target version (currently 27) and runs any missing migrations inside a single transaction. If any migration fails, the entire batch rolls back, leaving the database in its previous consistent state.

Migrations handle the full range of schema changes: creating tables, adding columns, creating indexes, renaming tables, and backfilling data. Each migration checks the current version before running, so they are safe to re-run. A helper method `column_exists()` enables safe `ALTER TABLE` operations that skip columns already present.

This approach means upgrading Mitsuro is seamless. The new binary starts, detects an older schema, and migrates it forward automatically.

## The Manager Pattern

Rather than funneling all SQL through a single monolithic class, Mitsuro gives each table domain its own manager struct. Each manager borrows or owns a `Database` and provides typed methods for that domain's operations.

The pattern looks like this:

```rust
pub struct MessageStore<'a> {
    db: &'a Database,
}

impl<'a> MessageStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn save_message(&self, session_id: &str, role: &str, content_json: &str) -> Result<()> {
        // SQL goes here
    }
}
```

Some managers borrow the database (using a lifetime parameter) while others own it outright. The choice depends on usage patterns -- stores that live for the duration of a request borrow, while stores that persist across the application lifetime own their database.

The current set of managers:

| Manager | Domain | Key Operations |
|---------|--------|----------------|
| `SessionManager` | Session lifecycle | Create, list, get, update, delete sessions |
| `MessageStore` | Conversation history | Save, load, replace, paginate messages |
| `PlanStore` | Multi-phase plans | Upsert, retrieve, abandon, update status |
| `Preferences` | User settings | Get/set key-value pairs, theme, model, etc. |
| `CredentialStore` | API keys | Load/save from JSON file with secure permissions |
| `MemoryStore` | Cross-session knowledge | Save, update, delete, list with project scoping |
| `AgentStateStore` | Agent execution tracking | Set/get state, list active sessions |
| `FileActivityTracker` | File access tracking | Record reads/writes/edits, rank by importance |
| `ReportStore` | Research reports | Create, list, search, delete reports |
| `RuntimeTraceStore` | Runtime diagnostics | Append events, load traces, compute summaries |
| `AutonomousTaskStore` | Hive task coordination | Create, claim, complete, fail tasks |
| `PushSubscriptionStore` | Push notification subscriptions | Upsert, remove, mark success/failure |
| `PushDeliveryAttemptStore` | Delivery tracking | Record attempts, compute summaries |
| `MakoRuntimeStateStore` | Hive daemon state | Get/set/upsert runtime state, list recoverable |
| `ProjectSettings` | Per-project overrides | Load from `.krusty/settings.json` |

This structure keeps each file focused. Adding a new storage domain means creating a new file with a new manager struct, not modifying a central class.

## Session Lifecycle

Sessions are the backbone of persistence. Every conversation, every plan, every agent run is tied to a session.

### Creation

A session starts with `SessionManager::create_session()`, which generates a UUID, records the title, model, working directory, and timestamps, then inserts the row. Sessions carry metadata about their context:

- **`session_type`** -- One of `chat`, `code`, or `mako`, distinguishing the product surface.
- **`work_mode`** -- Either `build` (the agent writes code) or `plan` (the agent plans only).
- **`workspace_mode`** -- Whether the session is `neutral` (no project), `selected` (user picked a project), or `created` (the session spawned a new workspace).
- **`project_dir`** -- The active project directory, when one exists.
- **`target_branch`** -- An optional git branch the session targets.
- **`user_id`** -- For multi-tenant deployments where sessions belong to specific users.

### Persistence

As the conversation progresses, messages are saved via `MessageStore`, plans via `PlanStore`, file activity via `FileActivityTracker`, and agent state via `AgentStateStore`. Each of these tables has a foreign key back to the session, with `ON DELETE CASCADE` so that deleting a session cleans up all related data automatically.

### Resumption

When you reopen Mitsuro and select an existing session, `SessionManager::get_session()` loads the session metadata, then `MessageStore::load_session_messages()` reconstructs the conversation history. Messages are stored as JSON-serialized content arrays, preserving full fidelity of text, tool calls, and structured content blocks.

Message loading supports pagination through `load_session_messages_paginated()`, which accepts offset and limit parameters for sessions with long histories.

### Recovery

If Mitsuro crashes or is interrupted mid-stream, the recovery system kicks in. A `SessionRecoveryState` is stored as JSON in the session's `recovery_json` column. This captures:

- What the agent was doing when interrupted (streaming, executing a tool).
- Any partial assistant output (text, thinking, in-flight tool calls).
- A decision about whether the session can be safely resumed, along with the user's last objective if resumption is possible.

On next startup, Mitsuro reads this state and either auto-resumes the session or explains why it cannot. This keeps work from being lost to crashes.

## Message Storage

The `messages` table stores every message in every session. Each row records the session ID, role (user, assistant, system), content as a JSON string, and a timestamp. The content field holds a serialized array of content blocks, which can include text, tool use requests, tool results, and other structured types. This format preserves the full richness of the conversation without flattening it to plain text.

`MessageStore` provides several key operations:

- **`save_message()`** -- Appends a new message and bumps the session's `updated_at` timestamp.
- **`load_session_messages()`** -- Returns all messages in insertion order.
- **`replace_session_messages()`** -- Atomically replaces all messages in a session (used during context compaction when the conversation is summarized to fit within token limits).
- **`update_last_message()`** -- Updates the most recent message of a given role in place (used when streaming assistant responses are finalized).
- **`delete_session_messages()`** -- Removes all messages, though this typically happens automatically via cascade when a session is deleted.

## Credentials

API key storage uses a different approach from the rest of the persistence layer. Instead of SQLite, `CredentialStore` reads and writes a JSON file at `~/.krusty/tokens/credentials.json`. The file is a flat map of provider keys to API key strings.

Security measures:

- **Atomic writes.** Credentials are written to a temporary file first, then renamed over the original. This prevents corruption if the process is killed mid-write.
- **Restrictive permissions.** On Unix, the file is set to mode `0600` (owner read/write only) before it is moved into place.
- **OAuth fallback.** When no API key is stored for a provider, `CredentialStore::get_auth()` checks for an OAuth token and attempts a refresh if the token has expired. This provides a seamless authentication experience across both key-based and OAuth-based providers.

The store tracks which providers have keys configured and provides a unified `has_auth()` check that considers both API keys and OAuth tokens.

## Preferences

`Preferences` wraps the `user_preferences` table, a simple key-value store with timestamps. It provides typed accessors for common settings:

- **Theme** -- The active UI theme (defaults to `"krusty"`).
- **Current model** -- The last-used model ID.
- **Recent models** -- An ordered list of up to 10 recently used models, stored as JSON.
- **Model cache** -- Cached model catalogs from dynamic providers like OpenRouter, with TTL-based staleness detection and fingerprint validation to detect catalog drift.
- **Custom models** -- User-defined model entries for any provider, persisted as JSON arrays.
- **Git identity** -- How Mitsuro identifies itself in commits (co-author mode by default).
- **Active plugin** -- The currently selected plugin ID.

Preferences support multi-tenant mode through an optional `user_id` parameter. When set, all reads and writes are scoped to that user. When unset (single-tenant mode), preferences are global.

## Plans

The `plans` table stores multi-phase execution plans with a strict one-to-one relationship to sessions. A UNIQUE constraint on `session_id` enforces that each session can have at most one active plan.

Plans are stored as Markdown in the `content` column, with the title and status tracked separately for efficient queries. `PlanStore` provides:

- **`upsert_plan()`** -- Creates or replaces the plan for a session. If a plan already exists, it preserves the original `created_at` timestamp.
- **`get_plan_for_session()`** -- Loads and parses the plan from Markdown back into a structured `PlanFile`.
- **`update_content()`** and **`update_status()`** -- Partial updates without replacing the entire plan.
- **`abandon_plan()`** -- Deletes the plan, allowing a fresh one to be created for the same session.

When a session is deleted, its plan is automatically removed via cascade.

## Push Notifications

Mitsuro supports Web Push notifications for alerting users when background tasks complete. Two stores handle this:

**`PushSubscriptionStore`** manages subscription records. Each subscription holds the Web Push endpoint, encryption keys (`p256dh` and `auth`), and health metadata. Subscriptions are upserted by endpoint, so re-subscribing from the same browser replaces the old record and resets failure counters. The store tracks success and failure timestamps, along with a failure count, allowing the system to identify degraded subscriptions.

**`PushDeliveryAttemptStore`** provides delivery observability. Every push attempt is recorded with the outcome (success or failure), HTTP status code, error message, and latency in milliseconds. Endpoints are hashed with SHA-256 for privacy in the logs. The store can produce delivery summaries showing the last attempt, last success, last failure, and failure count over the past 24 hours.

APNs device tokens for iOS push notifications are tracked in a separate `apns_devices` table with similar health metadata.

## Hive-Specific Storage

Hive is Mitsuro's autonomous agent system, and it has three dedicated storage domains.

### Autonomous Tasks

`AutonomousTaskStore` manages the task list that the Hive orchestrator works through. Each task has a subject, description, status (`pending`, `in_progress`, `completed`, `failed`), an optional owner (which sub-agent claimed it), and a list of blocker task IDs.

The key scheduling method is `get_available_tasks()`, which returns pending tasks whose blockers have all completed. This allows the orchestrator to execute tasks in dependency order without manual scheduling.

### Runtime Traces

`RuntimeTraceStore` records structured events from agent execution runs. Each trace event captures the run ID, a sequence number, the turn count, an event type, a JSON payload, and optional failure categorization. Failure categories include things like `provider_error`, `budget_exhausted`, `loop_guard_triggered`, and `tool_denied`.

Traces serve two purposes: post-mortem diagnostics (understanding what went wrong) and replay gating (deciding whether a previously failed run should be retried based on its failure pattern).

### Hive Runtime State

`MakoRuntimeStateStore` persists the daemon-level runtime state for autonomous sessions. Each Hive session has a status (`idle`, `running`, `sleeping`, `awaiting_input`, `paused`, `error`, `cancelled`), an optional next wake time, a sleep reason, the current run ID, and the last wake reason.

The `list_recoverable_states()` method returns sessions in `running` or `sleeping` status, which the daemon uses on startup to resume work that was interrupted when the process last stopped.

## File Activity Tracking

`FileActivityTracker` records which files the agent reads, writes, and edits during a session. Each operation increments the corresponding counter, and files explicitly referenced by the user get a bonus flag.

This data feeds an importance scoring algorithm used during context preservation (the "pinch" system). The score weighs writes (3 points), edits (2 points), reads (1 point), and user references (5 point bonus), then applies a recency multiplier that decays over 24 hours. The scoring can be computed either in Rust or in SQL -- the SQL path is preferred for large datasets since it avoids loading all rows into memory.

## Reports

`ReportStore` persists research reports produced by Chat sessions (with the research toggle) and Hive sessions. Each report has a title, content, summary, tags, sources, and an optional project directory. Reports are stored both in SQLite and as Markdown files on disk -- in `.krusty/reports/` within the project directory when one exists, or in `~/.krusty/reports/` otherwise.

Reports support listing by project directory and searching by title or tags.

## Project Settings

`ProjectSettings` loads per-project overrides from `.krusty/settings.json` within any project directory. Unlike the other storage domains, this is a read-only JSON file rather than a database table. It supports overriding the model, permission mode, system prompt, subagent turn limits, conventions, and disabled tools. All fields are optional -- only specified values override the defaults.

The loading is deliberately forgiving: a missing file returns defaults, invalid JSON returns defaults, and unknown fields are silently ignored. This matches the graceful-degradation pattern used throughout Mitsuro's configuration loading.

## File Paths

Everything lives under `~/.krusty/`:

```
~/.krusty/
  krusty.db              # The main SQLite database
  tokens/
    credentials.json     # API keys (mode 0600)
    active_provider.json # Currently selected provider
    mcp_keys.json        # MCP server API keys
    vapid_key.pem        # Web Push signing key
  logs/
    krusty.log           # Application logs
  plans/                 # Plan files in plan mode
  reports/               # Global reports (when no project is active)
  extensions/            # Extension scripts
  plugins/               # Installable plugins
```

Per-project state lives under `<project>/.krusty/`:

```
<project>/.krusty/
  settings.json          # Project-specific overrides
  reports/               # Project-scoped reports
  mailbox/               # Inter-agent messaging (delegated runs)
```

The `Database` constructor automatically creates the parent directory if it does not exist, so there is no manual setup step. The first time Mitsuro runs, the directory tree is created on demand.
