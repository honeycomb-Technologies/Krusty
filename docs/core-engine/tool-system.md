# The Tool System

The tool system is how Krusty does things. When the AI decides it needs to read a file, search the codebase, run a shell command, or edit code, it makes a tool call. The tool system receives that call, checks whether it's allowed, runs any safety checks, executes the tool, and returns the result. Every action the AI takes in your environment flows through this system.

At the center is the **ToolRegistry** -- a HashMap of tool implementations behind an `Arc<RwLock<>>`, with pre-execution and post-execution hooks attached. The registry manages tool registration, lookup, permission enforcement, and the full execution lifecycle. It's defined in `crates/krusty-core/src/tools/registry/runtime.rs`.

## The Tool Trait

Every tool in Krusty implements the same trait:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn prompt(&self) -> Option<&str> { None }
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, params: Value, ctx: &ToolContext) -> ToolResult;
}
```

`name()` is the identifier the AI uses when requesting a tool. `description()` goes into the tool schema that gets sent to the LLM so it knows what the tool does. `prompt()` is optional extended guidance that gets injected into the system prompt -- it contains the detailed usage instructions that would be too long for the schema description. `parameters_schema()` returns a JSON Schema that the AI's output must conform to. `execute()` does the actual work.

Every tool receives a `ToolContext` that carries the working directory, allowed filesystem root, process registry, permissions, streaming channels, and other runtime state. Tools return a `ToolResult` -- a structured JSON envelope with `ok: true/false`, a data payload, optional warnings, optional diffs, and optional metadata. The filesystem root is a path-containment policy for Krusty-owned file tools; it is not an operating-system sandbox.

## Built-in Tools

Krusty registers its tools at startup via `register_all_tools()` in `crates/krusty-core/src/tools/implementations/mod.rs`. They fall into several categories.

### File I/O

**Read** reads file contents with optional line offset and limit. It detects binary files, rejects files over 10 MB, and suggests similar filenames when a path doesn't exist. Output includes line numbers in `cat -n` format.

**Write** creates new files or completely overwrites existing ones. It generates a unified diff when overwriting, creates parent directories automatically, and caps content at 10 MB.

**Edit** performs string replacement with a five-pass fuzzy matching cascade: exact match, trailing whitespace trimmed, whitespace normalized, unicode normalized (smart quotes to ASCII, dashes, NBSP), and finally a block-anchor pass using Levenshtein distance. This means the AI doesn't need to reproduce whitespace or unicode characters perfectly -- the matching system handles minor discrepancies. `replace_all` mode uses exact matching only for safety.

**MultiEdit** applies multiple edits to one file in a single read-write cycle, avoiding the overhead of separate file reads for each change.

**ApplyPatch** handles multi-file patches in a structured format (Begin Patch / Update File / Add File / Delete File / End Patch). It uses the same fuzzy line-seeking system for context matching.

### Search

**Grep** wraps ripgrep for content search. It supports three output modes (matching lines, file paths only, or match counts), regex patterns, file type filtering, glob filtering, and context lines. Patterns are validated for length and nested quantifiers to prevent resource exhaustion.

**Glob** finds files by pattern (e.g., `**/*.rs`) and returns up to 100 paths sorted by modification time, newest first.

**List** shows directory contents with breadth-first traversal, configurable depth, and an entry limit.

### Execution

**Bash** runs shell commands with real-time output streaming. It supports foreground execution with configurable timeouts (default 30 seconds, max 10 minutes), background execution via `run_in_background`, and process group management for clean cleanup on timeout. Output is streamed to the UI as it arrives, then ANSI-stripped and truncated before being sent back to the AI. Krusty validates and scopes the starting working directory, but a shell can still access whatever files, processes, and network resources the server's OS account can access.

This makes Bash a **trusted-host capability**. It is appropriate for a private workstation or tailnet server such as Honey, where the authenticated user intentionally grants the agent that account's authority. It must be disabled for hostile public tenants unless each execution is placed in a real OS boundary such as a container, VM, or separately confined user account. ACP deliberately omits Bash and process tools because an editor connection does not supply such a boundary.

**Processes** manages background processes -- list running processes, check status and recent
combined output, or kill them by ID. API output replay is a bounded 64 KiB tail so a noisy server
cannot grow harness memory without limit; model-facing status narrows that to the most recent
8,000 characters. In authenticated server sessions, process creation, status, output, and control
operations stay scoped to the owning user.

### Agent Delegation

**Agent** is the unified sub-agent dispatcher. It launches specialized child agents that run their own AI conversation loops with scoped tool access:

- **explore** -- read-only codebase investigation. Gets read tools only (no bash, no writes). Inherits the parent conversation context so it understands what you've been discussing.
- **plan** -- implementation planning. Read-only, fresh context. Produces structured plans with steps, critical files, and trade-offs.
- **verify** -- runs tests, builds, and linters to validate changes. Read-only tools plus bash access. Reports a VERDICT: PASS, FAIL, or PARTIAL.
- **build** -- parallel code implementation. Gets full write access (in autonomous mode). Can spawn multiple builders for different components concurrently, each working in its own agent loop.

Sub-agents can also run in the background via `run_in_background: true`, returning a delegated run ID immediately while the work continues asynchronously.

### Interaction & State

**AskUserQuestion** prompts the user for input from within the AI's reasoning flow. **Skill** loads specialized instruction sets from `~/.krusty/skills/` or the project's `.krusty/skills/` directory. **Memory** persists knowledge across sessions -- user preferences, project context, feedback. **SetWorkMode**, **EnterPlanMode**, and **SetWorkspaceContext** let the AI toggle between plan mode and build mode or update workspace metadata.

### Mako (Autonomous Mode)

**CreateTask**, **UpdateTask**, **ListTasks**, **CreateReport**, **ListReports**, **ReadReport** -- these are registered additionally for Mako sessions to support autonomous task tracking and reporting.

## The Execution Lifecycle

When the AI requests a tool call, here's what happens step by step:

1. **Lookup.** The registry searches its HashMap for a tool matching the requested name. If no tool is found, execution returns `None` and the orchestrator reports an unknown tool error to the AI.

2. **Timeout resolution.** The system determines the timeout: per-call override from the context, then tool-specific override from the tool's policy, then the default (2 minutes). Delegated tools like `agent` get a longer default (15 minutes) since sub-agents legitimately need more time.

3. **Pre-hooks.** Every registered `PreToolHook` runs in order. Each hook can return `Continue` (proceed normally) or `Block { reason }` (halt execution with an error). If any hook blocks, the tool never executes. The `SafetyHook` and `PlanModeHook` are the built-in pre-hooks.

4. **Execution.** The tool's `execute()` method runs inside a `tokio::time::timeout`. If it completes within the deadline, the result passes through. If it times out, the registry generates a timeout error.

5. **Post-hooks.** Every registered `PostToolHook` runs. These receive the tool name, parameters, result, and execution duration. The `LoggingHook` uses this to emit structured tracing events for every tool call.

6. **Result return.** The `ToolResult` flows back to the orchestrator, which feeds it into the AI's next turn as a tool result message.

## Permission Modes

Every tool has a `ToolPolicy` that classifies it along several axes:

- **Category**: `ReadOnly` (never modifies state), `Write` (modifies files, runs commands), or `Interactive` (requires user input).
- **requires_supervised_approval**: Whether the tool needs explicit user approval in supervised mode.
- **allowed_in_plan_mode**: Whether the tool can run when plan mode is active.
- **retry_timeout_once**: Whether the system should retry once on timeout (useful for read tools where transient failures are common).
- **timeout_override**: Tool-specific timeout (the `agent` tool gets 15 minutes instead of 2).

Krusty runs in one of two permission modes:

**Supervised** (the default). Read-only tools execute freely. Write tools -- file edits, bash commands, patches -- require the user to approve each call before it runs. The UI presents the tool call parameters and waits for confirmation. This is the mode for interactive sessions where you want oversight.

**Autonomous**. All tools execute without approval. This is the mode for Mako background agents and for users who trust the AI to operate independently. The safety hooks still apply -- autonomous mode doesn't disable the SafetyHook, so genuinely dangerous commands are still blocked.

Sub-agents inherit their parent's permission mode through `DelegationPolicy`. An explore sub-agent spawned from a supervised parent will have its tools restricted accordingly. Build sub-agents in autonomous mode get full write access; in supervised mode, their write tools are blocked entirely (since there's no interactive approval path for a background sub-agent).

## Safety Hooks

The `SafetyHook` (defined in `crates/krusty-core/src/agent/hooks/builtins.rs`) is a pre-execution hook that blocks dangerous bash commands before they run. It fires only for bash/shell/execute tools and checks the command against several categories of dangerous patterns.

**What gets blocked:**

- **Privilege escalation**: `sudo`, `doas`, `su`
- **Destructive file operations**: `rm -rf /`, `rm -rf ~`, `rm -rf $HOME`, and variants targeting `/etc`, `/usr`, `/var`
- **Filesystem formatting**: `mkfs` commands
- **Direct disk access**: `dd` with `of=/dev/` or `if=/dev/` targets
- **Unsafe permissions**: `chmod 777`
- **Network-to-shell pipes**: `curl ... | sh`, `wget ... | bash`
- **Fork bombs**: The classic `:(){ :|:& };:` pattern
- **Raw disk redirection**: `> /dev/sda` and similar

The hook parses commands properly -- it splits on shell operators (`;`, `|`, `&&`), respects quoting (so `echo "rm -rf /"` won't trigger a false positive), strips environment variable prefixes (`DEBUG=1 rm -rf /` is still caught), and handles the full range of shell syntax that could be used to disguise a dangerous command.

The **PlanModeHook** is a separate pre-hook that enforces plan mode restrictions. When plan mode is active, it blocks all write-category tools and any bash commands that modify state (mkdir, mv, cp, git commit, npm install, cargo build, etc.), while allowing read-only operations (ls, cat, git status, git diff). The classification checks each segment of a compound command independently.

## Output Truncation

AI models have limited context windows, and tool output can be enormous -- a `cargo build` might produce thousands of lines, or a file read might return a massive source file. Sending all of that back wastes tokens and can push important context out of the window.

The truncation system (in `crates/krusty-core/src/tools/truncation.rs`) applies dual limits: a maximum number of lines and a maximum number of bytes. It supports two modes:

**Tail truncation** keeps the most recent output. This is used for bash command output, where the end of the output (error messages, final status) is usually more relevant than the beginning. The bash tool defaults to 2,000 lines and 50 KB.

**Head truncation** keeps the beginning of the output. This is used for file reads, where the start of the file is usually what you asked for.

When output is truncated, a notice is appended: `[Output truncated: showed 200 of 5000 lines (48000/250000 bytes)]`. This tells the AI that it's seeing partial output and may need to adjust its approach.

The bash tool additionally strips ANSI escape sequences before truncation, since terminal color codes are meaningless to the AI and waste tokens.

## MCP Tools

Krusty discovers external tools at runtime through the Model Context Protocol. When MCP servers are configured, the `McpManager` connects to them and queries their tool catalogs. Each discovered tool gets wrapped in an `McpTool` struct (in `crates/krusty-core/src/mcp/tool.rs`) that implements the `Tool` trait, making MCP tools indistinguishable from built-in tools at the registry level.

MCP tool names are namespaced as `mcp__{server}_{tool}` to avoid collisions with built-in tools. Their parameter schemas are sanitized on registration -- the wrapper ensures every schema has `additionalProperties: false`, valid `required` arrays, and proper object structure, since external servers don't always produce schemas that meet Anthropic's strict requirements.

One important detail: MCP tools execute on external servers and bypass Krusty's local sandbox. When a sandboxed context invokes an MCP tool, a warning is logged and included in the result metadata. The execution happens on the remote server, not in the local environment, so local path restrictions don't apply.

## Tool Matching

The `matching` module (in `crates/krusty-core/src/tools/matching.rs`) provides the fuzzy matching cascade used by the Edit, MultiEdit, and ApplyPatch tools. It's not about matching tool calls to tools (that's a simple HashMap lookup by name) -- it's about matching the text strings the AI provides against the actual file content.

AI models frequently produce text that doesn't exactly match the file: trailing whitespace differences, smart quotes instead of ASCII quotes, collapsed whitespace, or minor typos. The five-pass cascade handles all of these:

1. **Exact match** -- direct string search
2. **Line-trimmed** -- strip trailing whitespace from each line before comparing
3. **Whitespace-normalized** -- collapse all whitespace runs to single spaces
4. **Unicode-normalized** -- convert smart quotes to ASCII, em/en dashes to hyphens, non-breaking spaces to regular spaces, strip zero-width characters
5. **Block-anchor** -- anchor on the first and last lines (which must match exactly after trimming), then allow the middle lines to differ with a Levenshtein similarity threshold of 0.5

The system always uses the matched region from the original content for replacement, not the normalized version. This means the edit preserves the file's actual formatting even when fuzzy matching was needed to find the location.

For `replace_all` operations, only exact matching is used -- fuzzy bulk replacement would be too risky.

For the ApplyPatch tool, a related `seek_sequence` function provides four-pass line seeking (exact, trailing-trimmed, fully-trimmed, unicode-normalized) to locate context lines in the target file, with support for both forward scanning and reverse scanning from the end of file.

## Tool Registration Order

Tools are registered in a deterministic order at startup. This matters because the tool list becomes part of the system prompt sent to the AI provider, and Anthropic's prompt caching is sensitive to exact content. If tools were registered in random order (from HashMap iteration), the cached prompt prefix would break between API calls, wasting money on re-processing. The registry sorts tools alphabetically by name before generating the AI tool definitions.

Different contexts get different tool sets. The full TUI session registers everything. ACP mode excludes interactive tools like AskUserQuestion and EnterPlanMode (since the editor can't render those interactions). Mako sessions add autonomous task and report tools on top of the standard set. Sub-agents get a filtered view based on their `DelegationPolicy` -- explore agents see only read-only tools, build agents see write tools too (if the permission mode allows), and no sub-agent gets the `agent` tool itself (preventing recursive delegation loops).
