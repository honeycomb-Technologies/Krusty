# Context, Memory & Summarization

Every time you send a message to Krusty, the system builds a carefully structured context payload before the AI sees anything. That payload includes project instructions, environment details, persistent memories, active plans, available skills, and the conversation history itself. As conversations grow long, the system compresses older content to stay within the model's context window. When a session truly outgrows its limits, it can be handed off to a new session through a structured summarization process called pinch.

This document explains each of those mechanisms: what gets injected, how the system tracks what it has already told the model, how it decides what to keep and what to trim, and how it bridges the gap between sessions.

## The Context Problem

Large language models have a fixed context window. Even the largest models available today cap out at a few hundred thousand tokens. A coding conversation that spans several hours of exploration, editing, and debugging can easily produce hundreds of messages, each carrying file contents, tool outputs, diff previews, and reasoning traces. Without active management, the raw conversation would blow past the context limit long before the work is done.

Krusty addresses this at three levels. First, it controls what goes into the context at the start of every turn through context injection. Second, it compacts the conversation in place when it grows too large, keeping the session alive without starting over. Third, it offers a clean session transition through pinch, which summarizes an entire conversation into a structured artifact that seeds the next session.

## Context Injection

Before every AI call, the `inject_context` function in `context.rs` builds a new message array by prepending system messages ahead of the actual conversation. The model never sees these as user messages or part of the chat history. They're framing: instructions, state, and environmental awareness that shape how the AI responds.

The injection follows a fixed order:

1. **Workspace context** -- Tells the model where it is. In project mode, this identifies the project root and execution directory. In neutral mode (no project selected), it instructs the model not to assume any particular codebase.

2. **Environment context** -- Runtime facts gathered at call time: the working directory, current git branch, file change counts, platform, shell, date, and which model is active. Git commands that fail (no repo, missing binary) are silently skipped rather than producing errors.

3. **Persistent memory** -- Memories that survive across sessions, pulled from the SQLite memory store. These are organized by type (user context, feedback/guidance, project context, external references) and capped at 15 entries per type, 300 characters per entry, and 8KB total. This prevents a large memory collection from dominating the context budget.

4. **Project instructions** -- Content from instruction files discovered in the project directory. The system walks from the project root down to the current working directory, checking each level for files like `KRAB.md`, `AGENTS.md`, `CLAUDE.md`, `.cursorrules`, and others. If multiple levels contain instruction files, all of them are included, creating a layered instruction set where subdirectories can refine or extend the root rules.

5. **Project settings** -- An optional `system_prompt_append` field from the project's settings, allowing per-project prompt customization beyond what the instruction files provide.

6. **Plan state** -- If there's an active plan for the current session, the full plan context is injected. In build mode, this includes the plan markdown, progress counts, ready tasks, blocked tasks, and a workflow protocol for picking and completing tasks. In plan mode, it reminds the model that it can read but not write.

7. **Delegated runs** -- Recent sub-agent investigations (explore, build, planner, verifier runs) for the session. This context tells the model that prior delegated work exists and encourages reusing those results rather than re-exploring the same directories.

8. **Autonomous tasks** -- Any Mako tasks associated with the session, grouped by status (pending, in progress, completed).

9. **Reports** -- Recent investigation reports for the project, shown as title/date/summary with a pointer to use the `ReadReport` tool for full content.

10. **Coordinator context** -- For Mako sessions specifically, a specialized coordinator system prompt is injected.

11. **Skills** -- A listing of all available skills with names, descriptions, and tags, plus instructions on how to invoke them.

Chat sessions get a stripped-down version of this: just a chat-specific system prompt and persistent memories. No workspace, no tools, no plans.

## The Context Ledger

The context ledger, defined in `context_ledger.rs`, is a bookkeeping structure that tracks the state of the conversation from a continuation standpoint. It answers two questions: "What has happened to this conversation's context?" and "Can this session be resumed?"

Every time the conversation changes, the ledger recomputes its counts by scanning all messages:

- **Canonical messages** -- Total messages in the conversation.
- **Pinned messages** -- System messages containing project instructions, which must survive compaction.
- **Summarized messages** -- Tool results tagged with `summarize_after_turn` retention, meaning they've already been condensed.
- **Dropped messages** -- Tool results tagged with `drop_after_compaction`, meaning they can be removed entirely during compaction.
- **Latest user objective** -- The text of the most recent user message, extracted by scanning the conversation in reverse.

The ledger uses this state to make a **continuation decision**: is this session resumable? A session is resumable if it has at least one canonical message and a non-empty latest user objective. It's non-resumable if the conversation is empty or there's no identifiable user goal (for example, if the most recent user message contained only a tool result with no text).

When compaction occurs, the ledger records a snapshot: the reason for compaction, the estimated token counts before and after, and how many messages were replaced. This creates an audit trail that makes context decisions traceable rather than opaque.

The ledger also produces a **continuation contract** -- a serializable record that can be persisted alongside the session, capturing the schema version, the resumability decision, and the latest objective. This is what downstream systems (like pinch) use to determine whether and how to create a continuation.

## Conversation Compaction

Compaction is the mechanism that keeps a long-running session alive when the conversation approaches the model's context limit. It operates in place, modifying the existing conversation rather than creating a new session. The implementation lives in `compaction.rs`.

The `CompactionManager` is configured per model with three token thresholds derived from the model's profile: a trigger threshold (when compaction begins), a target threshold (the goal size after compaction), and a hard failure threshold (critical overrun). Compaction proceeds in stages, from least aggressive to most:

**Stage 1: Strip old thinking.** Extended thinking blocks (the model's internal reasoning traces) are removed from all but the two most recent assistant messages that contain them. These blocks can be large, and older reasoning is rarely relevant to the current turn.

**Stage 2: Compact old tool results.** Tool results outside the most recent six messages are processed based on their retention policy. Results tagged `drop_after_compaction` (bash output, web fetches, explore results) are replaced with a compact stub containing just the summary and a note to re-run the tool if needed. Results tagged `summarize_after_turn` (grep, glob, edit, write results) or results that exceed the preview size limit are similarly compacted. Results tagged `retain_full` (like file reads) are bounded but kept more intact.

**Stage 3: Summary replacement.** If the conversation still exceeds the target after stages 1 and 2, the system builds a summary of the middle portion of the conversation. It keeps the first user message and the most recent N messages (trying 6, then 4, then 2, then 1), replacing everything in between with a single assistant message containing a structured summary. The summary captures user goals, assistant progress, tool activity, and the latest user request.

**Stage 4: Continuation replacement.** If even the most aggressive summary replacement doesn't reach the target, the system falls back to a continuation replacement that summarizes everything except the single most recent message. This is the nuclear option -- it preserves almost nothing of the original conversation, but it keeps the session alive.

The summary text itself is structured with clear sections: a compaction header, the reason, the latest carried-forward user request, user goals still in scope, earlier progress, and important tool activity. This gives the model enough orientation to continue without the full history. Throughout all stages, system messages (especially pinned project instructions) are preserved -- they're never summarized or dropped.

## History Policies

The history policy system in `history_policy.rs` shapes how tool results are stored in conversation history from the moment they're created, before compaction ever runs. Every tool result passes through `build_history_tool_result`, which assigns it a retention classification and builds both a human-readable summary and a compact representation.

The three retention levels are:

- **Retain full** -- The complete result is kept in history, bounded to prevent extreme sizes (16K chars for file content, 6K for diffs, 3K for command output). This is the default, used for `read` results and other tools not explicitly classified.

- **Summarize after turn** -- The result is replaced with a structured summary after the current turn. Used for `grep`, `glob`, `list`, `write`, `edit`, `multiedit`, and `apply_patch`. The summary preserves key facts (match counts, file paths, line counts, diff previews) while discarding the bulk content.

- **Drop after compaction** -- The result can be entirely removed during compaction. Used for `bash`, `processes`, `web_search`, `web_fetch`, `explore`, and `build`. These produce outputs that are either ephemeral (process lists, web pages) or available through re-execution.

Each tool has a specialized summarizer: grep reports match/file counts, write reports line counts with a diff preview, edit reports replacement counts, bash reports exit codes. These summaries become the `summary` field in the history entry, which is what the model sees after compaction replaces the full output. This two-layer approach -- shape results at creation time, then compact them later -- means the conversation history is already leaner than raw tool output before compaction ever fires.

## Pinch: Structured Session Transitions

Pinch is Krusty's mechanism for gracefully ending one session and starting another with preserved context. Unlike compaction, which keeps the same session alive, pinch creates a clean break: a new session seeded with a structured summary of what came before. The implementation spans `pinch_context.rs` and `summarizer.rs`.

### The Summarization Engine

When pinch is triggered (via the `/pinch` command), the summarizer calls the user's current model to analyze the conversation and produce a structured JSON summary with four fields:

- **Work summary** -- Two to three paragraphs describing what was accomplished, focusing on the what and why rather than the mechanics.
- **Key decisions** -- Architectural choices, patterns adopted, trade-offs made. Things the next session needs to understand.
- **Pending tasks** -- Incomplete work, explicitly mentioned TODOs, or logical next steps.
- **Important files** -- The ten most relevant file paths for continuing the work.

The summarization is cache-safe by design. When conversation messages exist, the system reuses the parent conversation's cached prefix (the system prompt and all prior messages) and appends the summarization instruction as a new user message. This means most of the tokens in the API call are cache hits, and only the instruction itself is new. The model sees the full conversation in its native form rather than a flattened text dump.

The user can provide preservation hints -- specific areas to weight heavily in the summary -- and the system also feeds in ranked files (scored by activity during the session) and the contents of key files.

### The Pinch Context

The summarization result is packaged into a `PinchContext` along with metadata: source session ID and title, the ranked file list, preservation hints, user direction for the next phase, project instructions, key file contents, and any active plan.

When the new session starts, `PinchContext::to_system_message` formats all of this into a structured markdown document that becomes the opening system message. It includes: a directive header telling the model not to re-discover what's already documented, the user's priority direction (placed first for salience), the work summary, key decisions, numbered pending tasks, ranked key files, preservation notes, the full project instructions (truncated at 8KB if needed), pre-loaded key file contents, and any active plan. The result is that a pinched session starts with rich context, relevant files already loaded, and clear direction -- without carrying the full weight of the original conversation.

## Skills Injection

Skills are domain-specific instruction sets packaged as directories with a `SKILL.md` file containing YAML frontmatter (name, description, version, author, tags) followed by markdown content. They live in two locations: `~/.krusty/skills/` for global skills and `.krusty/skills/` within a project for project-specific skills. Project skills override global skills with the same name.

The `SkillsManager` in `manager.rs` handles discovery and caching. On first access, it scans both directories, parses each `SKILL.md`, and builds a name-keyed cache. The `build_skills_context` function in `context.rs` reads this cache and formats a listing of available skills for injection into the system context. The listing includes each skill's name, description, and tags, plus instructions on how to invoke them via the `skill` tool.

When the model calls the `skill` tool, the manager loads the full markdown content of the requested skill and injects it into the conversation. This lazy loading means skill content doesn't consume context budget until it's actually needed. The listing in the system prompt is lightweight -- just names and descriptions -- so the model knows what's available without paying the token cost of every skill's full instructions.

Skills are loaded from the filesystem and cached in memory. The cache can be invalidated (when skills are created or deleted) and individual skills can be reloaded (when edited). Path traversal attacks are blocked: loading files from within a skill directory verifies that the resolved path stays within the skill's directory boundary.

## Shared Build Context

When Krusty runs parallel operations -- specifically the builder swarm, where multiple sub-agents work on different parts of a codebase simultaneously -- the `SharedBuildContext` in `build_context.rs` provides the coordination layer.

This is a thread-safe shared state object using `DashMap` (concurrent hash maps) and atomics. It tracks coding conventions (style rules all builders follow), file locks (per-file locking with retry/timeout so agents don't clobber each other's edits), modified files (which agent touched what), line diffs (running add/remove totals), an interface registry (builders publish exported types/functions that others can depend on), and contention metrics (lock wait times per file, flagging hotspots where total wait exceeds one second).

The `generate_context_injection` method produces a text block that gets injected into each builder's prompt, listing current conventions, files in progress (and who's working on them), and available interfaces from other builders. This gives each parallel agent awareness of the swarm's state without requiring direct communication between agents.

## How It All Connects

The context system operates as a pipeline with feedback loops. At the start of each turn, context injection builds the full message array. The orchestrator sends it to the model. When the model calls tools, the history policy shapes the tool results before they enter the conversation. As the conversation grows, the context ledger tracks its state. When tokens cross the trigger threshold, compaction fires and the ledger records what happened. If the session truly runs out of room, pinch offers a structured exit to a new session.

Each layer is designed to be invisible when things are working well. The user doesn't think about context budgets or retention policies. They just have a conversation, and the system quietly ensures the model always has the most relevant context available within its limits.
