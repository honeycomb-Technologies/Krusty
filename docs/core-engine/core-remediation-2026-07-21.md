# Core remediation: one run contract, exact models, semantic progress

This change is a targeted simplification of Krusty's existing core, not a
greenfield rewrite. The failure being corrected was architectural: mutable
model slugs, duplicated entry-point defaults, and output-sensitive loop
heuristics allowed the same apparent action to be prepared differently or to
repeat forever. Replacing Rust with a fresh core would not remove those
contract mistakes; making the contracts explicit does.

## Current comparison baseline

The comparison was refreshed from clean Honey checkouts on 2026-07-21:

| Harness | Revision | Useful core technique |
| --- | --- | --- |
| Pi | [`dd6bea4`](https://github.com/badlogic/pi-mono/commit/dd6bea41efa8caa7a10fe5a6401676dc5699f83f) | A small agent/session loop, a provider/model registry, and transport-specific adapters. Model/runtime selection is resolved before the loop rather than rediscovered inside each tool call. |
| OpenCode | [`4438f69`](https://github.com/anomalyco/opencode/commit/4438f69aac46806c631866489a26b644488a784e) | One session processor owns the turn lifecycle while one LLM request boundary owns provider transforms, tools, prompt layers, and request preparation. |
| Goose | [`3065c97`](https://github.com/block/goose/commit/3065c9701fdccd020f86f263c74ae4934a1333b8) | Agent reply processing, tool execution, context management, and provider implementations are separate modules joined by typed conversation/model contracts. |
| Codex | [`bdd3118`](https://github.com/openai/codex/commit/bdd3118c71a29f26b9df3a47f91efea38a0d58bd) | A session owns immutable turn context, model-family behavior, tool routing/lifecycle, and conversation history; observable events come from those canonical boundaries. |

None of these harnesses is simple because it has fewer files. Their useful
simplicity is that there is one owner for each decision. Krusty legitimately
has more surface area—TUI, ACP, HTTP/mobile, Mako, extensions, multi-tenant
storage, and several provider auth modes—but those surfaces do not need their
own definitions of model identity, request policy, or loop termination.

### Across-the-board mechanism comparison

The comparison below is about runtime ownership, not UI feature count. It
covers the control paths that materially affect whether an agent behaves
correctly: prompt construction, model identity, planning, streaming, tools,
processes, cancellation, delegation, persistence, and client projection.
Source links are pinned to the revisions above.

| Contract | Pi | OpenCode | Goose | Codex | Repaired Krusty |
| --- | --- | --- | --- | --- | --- |
| Turn owner | [`agent-loop.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/agent/src/agent-loop.ts) owns the provider/tool continuation; [`agent-session.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/coding-agent/src/core/agent-session.ts) owns persistence, retry, and compaction around it. | [`prompt.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/prompt.ts) drives the session and defaults agent steps to infinity; [`processor.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/processor.ts) owns one streamed response/tool lifecycle. | [`Agent::reply_internal`](https://github.com/block/goose/blob/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/agents/agent.rs) owns the reply loop. It has an overrideable 1,000-turn resource ceiling, not a 50-turn product default. | Session state and immutable per-turn context are separated in [`state/session.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/state/session.rs) and [`session/turn_context.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/session/turn_context.rs). | `RunSpec` is the only production constructor for the parent streaming kernel. Interactive parent and ACP runs default to unlimited; the separately governed delegated kernel also defaults to unlimited. A typed budget is an explicit resource policy in both. |
| Prompt and model-family instructions | [`system-prompt.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/coding-agent/src/core/system-prompt.ts) builds one compact prompt from the active tools and loaded resources; provider adapters own wire conversion rather than changing agent policy. | [`system.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/system.ts) selects model-family prompt text, while the session request boundary layers instructions and tools in a stable order. | [`prompt_manager.rs`](https://github.com/block/goose/blob/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/agents/prompt_manager.rs) composes the base prompt with enabled extension instructions; provider code receives the resulting typed request. | Session initialization resolves model-owned base instructions in [`session/mod.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/session/mod.rs), then turn context layers workspace, permission, skill, plugin, and mode instructions without rewriting the model identity. | `ModelProfile` selects the prompt family. One prompt-section builder produces a diagnostic manifest, and streaming, simple, and delegated calls use the same immutable model runtime rather than provider-local prompt patches. |
| Planning and work mode | Pi deliberately has no built-in plan mode; its documented [plan-mode extension](https://github.com/badlogic/pi-mono/tree/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/coding-agent/examples/extensions/plan-mode) changes active tools and injects mode context outside the small core. | Plan-agent permissions, [`plan-mode.txt`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/prompt/plan-mode.txt), and plan enter/exit tools make the mode and its mutation restrictions explicit. | The CLI owns an interactive [plan workflow](https://github.com/block/goose/blob/3065c9701fdccd020f86f263c74ae4934a1333b8/documentation/docs/guides/context-engineering/creating-plans.md), including optional planner provider/model selection and an explicit transition back to execution. | [`collaboration_mode_presets.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/models-manager/src/collaboration_mode_presets.rs) supplies mode-specific developer instructions; session/turn settings carry the selected mode. | Persisted `WorkMode` and canonical plan lifecycle drive the prompt and provider-facing tool surface. Every Plan/Build transition rebuilds that surface under the exact run allowlist; Plan mode excludes free-form Bash and mutations. Awaiting approval is a typed durable continuation. |
| Repetition control | No central semantic loop detector was found; the small loop, complete tool results, cancellation, and output guard keep the basic path narrow. | Three identical consecutive tool names plus byte-identical JSON input trigger the `doom_loop` permission check in [`processor.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/processor.ts). Cosmetic argument changes can evade it. | [`RepetitionInspector`](https://github.com/block/goose/blob/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/tool_monitor.rs) compares exact name/parameters, but the default agent installs it without a finite repetition limit; the high turn ceiling remains the fallback. | No generic same-command fingerprint was found in core. Long shell work instead has an explicit process lifecycle, wait/poll tools, cancellation, and typed tool routing, reducing the incentive to relaunch the same command. | `ProgressLedger` hashes normalized intent, strips Bash presentation variants, distinguishes observation/effect/validation, ignores volatile PID/time output, warns, replans, then stops after three no-progress turns. This is stricter than exact-JSON matching without penalizing new evidence. |
| Model identity | [`model-registry.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/coding-agent/src/core/model-registry.ts), [`model-resolver.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/coding-agent/src/core/model-resolver.ts), and [`model-runtime.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/coding-agent/src/core/model-runtime.ts) resolve a provider/model runtime before execution. | Provider/model selection is resolved before [`llm/request.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/llm/request.ts) prepares the request. | `ModelConfig`, canonical model registry, and provider trait travel together through the reply context; provider/model attribution is explicit. | `TurnContext` carries the selected model/provider behavior into client and tool paths. | `ModelKey` includes provider, wire ID, auth scope, and API format; `ResolvedModelRuntime` freezes capabilities/source/revision. Ambiguous bare IDs fail closed. |
| Provider request owner | Provider packages own wire encoding while the agent loop owns continuation and retry policy. | One LLM request boundary layers prompts/tools and delegates provider quirks to [`transform.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/provider/transform.ts). | [`reply_parts.rs`](https://github.com/block/goose/blob/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/agents/reply_parts.rs) prepares sorted tools/prompt/conversation, then calls the selected provider trait. | [`client.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/client.rs) and model/provider info own request preparation; session code owns turn policy. | One immutable runtime feeds one prompt manifest, history transform, sorted/capability-filtered tools, effective options, transport policy, and redacted `ProviderRequestPrepared` event. |
| Streaming and tool-call assembly | [`event-stream.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/ai/src/utils/event-stream.ts) and provider adapters emit typed stream events; `agent-loop.ts` assembles complete tool calls and feeds exactly matched results into the next turn. | `processor.ts` owns streamed message-part state and the tool lifecycle, while [`llm/request.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/llm/request.ts) owns request preparation. | Provider streams are normalized through the provider trait; `reply_parts.rs` and [`tool_execution.rs`](https://github.com/block/goose/blob/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/agents/tool_execution.rs) separate assistant-part collection from execution. | [`stream_events_utils.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/stream_events_utils.rs) and the Responses client normalize incremental events; the typed tool router owns dispatch and lifecycle. | Provider-specific parsers normalize into shared content/tool events. The orchestrator requires a terminal result for every accepted call, rejects extra resumable interactions explicitly, and never lets partial or malformed events silently redefine history. |
| Backpressure and cancellation | One `AbortSignal` propagates through the agent loop, provider stream, and tool execution; foreground Bash kills its process tree on abort and output accumulation is bounded. | The session LLM and tool contexts carry one abort signal; `processor.ts` closes the active response/tool lifecycle, while client transports consume the durable event stream independently. | `CancellationToken` flows through `Agent`, provider/MCP calls, and tool execution; the caller owns frontend delivery and may stop the active reply. | Turn/task cancellation is owned by session tasks, while app-server transports use bounded channels and explicit lag/backpressure handling rather than blocking the core. | Cancellation closes provider streams and propagates into tools and delegated children. Stream idle timeouts are transport-aware; SSE and TUI projections have bounded queues, explicit lag signals, and cannot stall the canonical loop. |
| Retry ownership | [`utils/retry.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/ai/src/utils/retry.ts) is the bounded reusable assistant-call retry owner; compaction handles overflow separately. | [`session/retry.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/retry.ts) classifies provider errors and backoff centrally. | Agent retry manager and provider errors are explicit; the reply loop owns recovery decisions. | Client/Responses retry modules classify transport/auth phases while turn code owns whether execution continues. | Non-streaming retries have one owner; streaming setup retries only classified pre-output failures. Context overflow compacts once and retries once. Calls are never replayed after visible/tool activity. |
| Context and compaction | [`compaction.ts`](https://github.com/badlogic/pi-mono/blob/dd6bea41efa8caa7a10fe5a6401676dc5699f83f/packages/coding-agent/src/core/compaction/compaction.ts) keeps source history durable and builds a summary plus recent tail. | [`session/compaction.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/session/compaction.ts) summarizes older history while retaining a tail and prunes old tool payloads. | [`context_mgmt`](https://github.com/block/goose/tree/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/context_mgmt) computes thresholds and structured compaction from the selected model context. | [`context_manager`](https://github.com/openai/codex/tree/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/context_manager) and compact modules normalize history and reinject required initial context. | Exact runtime context size drives a shared policy; live in-place compaction is default, durable memory flush precedes summarization, checkpoints remain searchable, and UI-facing raw tool output is separated from retained model history. |
| Tools and governance | A deliberately small default coding tool set; extensions wrap definitions/execution. | Tools are filtered by agent permission and the final map is stable; task delegation creates a first-class child session. | Tool inspection composes security, egress, adversary, permission, and optional repetition checks before execution. | [`tools/orchestrator.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/tools/orchestrator.rs), routing, lifecycle, approvals, and sandbox policy are distinct typed layers. | Parent and delegated paths inherit one permission/turn contract at execution time. Filesystem policy stays in `ToolContext`; mutation tools publish structured `changed` evidence. |
| Subagents, concurrency, and resumability | Pi intentionally omits built-in subagents; packages may implement them without enlarging the base loop. | [`task.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/tool/task.ts) creates a linked child session with an explicit agent/permission envelope, making child history and continuation first-class session state. | [`subagent_execution_tool`](https://github.com/block/goose/tree/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/agents/subagent_execution_tool) and `subagent_handler` own scoped child work with explicit task configuration, provider/extensions, turn budget, notifications, and cancellation; it remains a distinct agent path rather than the parent reply loop. | [`session/multi_agents.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/session/multi_agents.rs) creates governed child threads through the session runtime with parent linkage, events, and cancellation. | `AgentScheduler` provides adaptive queued concurrency and inherited permission/path/tool/budget ceilings. Delegated runs persist lifecycle artifacts and can seed a later related run, but execution is a separate non-streaming mini-kernel: it does not yet share the parent `RunSpec`, full streaming trace, or crash-continuation state. |
| Bash and process lifecycle | The built-in Bash tool is foreground-only, bounds output, and terminates the process tree through the turn's abort signal; background Bash is deliberately left to extensions or external tools. | [`shell.ts`](https://github.com/anomalyco/opencode/blob/4438f69aac46806c631866489a26b644488a784e/packages/opencode/src/tool/shell.ts) centralizes shell parsing, permission arity, process spawning, streaming output, and truncation. | The built-in [Developer shell](https://github.com/block/goose/blob/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/agents/platform_extensions/developer/shell.rs) owns shell selection, subprocess execution, output, and cancellation as an extension tool. | Unified exec has typed execute/write/wait handlers backed by [`tools/runtimes/unified_exec.rs`](https://github.com/openai/codex/blob/bdd3118c71a29f26b9df3a47f91efea38a0d58bd/codex-rs/core/src/tools/runtimes/unified_exec.rs), so long work is polled or resumed instead of relaunched. | Foreground Bash publishes only bounded positive state evidence. `ProcessRegistry` owns background launch deduplication, per-owner active/history caps, bounded output tails, process-tree termination, and reuse; Plan mode exposes no free-form shell. |
| Persistence and observation | JSONL session entries, explicit events, usage, compaction, and retry state are owned by `AgentSession`. | Message parts are the durable lifecycle; session processor updates them as streaming/tool states change. | Session manager persists conversation and usage; `AgentEvent` is the frontend boundary. | Session/turn items and tool lifecycle events are canonical; telemetry is emitted from those owners. | Canonical `LoopEvent` is persisted as a compact runtime trace and projected to SSE, TUI, ACP, extensions, and Mako. Model key, request policy, budget source, progress action, and typed stop reason are inspectable without raw prompts or credentials. |
| CLI, server, mobile, and ACP projection | Interactive, print, RPC, and SDK modes consume `AgentSession` events; the small harness does not claim a multi-tenant shared server core. | Durable session/message parts feed the CLI, HTTP server, SDK, and ACP adapters instead of each surface owning another agent loop. | The same `Agent`/session manager events feed CLI, desktop/API, and [`acp/server`](https://github.com/block/goose/tree/3065c9701fdccd020f86f263c74ae4934a1333b8/crates/goose/src/acp/server); adapters translate protocol state rather than provider policy. | Session items and events are projected through app-server and TUI protocol layers; clients do not reconstruct the turn policy. | TUI, server/mobile, and ACP each resolve inputs into `RunSpec`, then consume canonical `LoopEvent`s. Ownership, exact model identity, tool scope, work mode, and continuation state remain core/server contracts rather than client heuristics. |

The upstream review therefore did **not** justify copying one harness or doing a
fresh rewrite. It identified the same recurring technique: keep each mutable
decision behind one narrow owner. Krusty's excess complexity was not Rust, its
number of providers, or its product surfaces; it was allowing those surfaces to
re-resolve the same decisions independently.

### Implemented convergence and deliberate remaining boundary

The repaired parent streaming path implements that ownership model now:
`ResolvedModelRuntime`, `RunSpec`, the prompt/history/request pipeline,
mode-aware tools, `ProgressLedger`, `ProcessRegistry`, durable continuation
claims, and `LoopEvent` each have one canonical owner. Server, TUI, ACP, and
Mako inputs may differ, but they cannot independently redefine those contracts.

Delegated execution is deliberately narrower, not falsely described as the
same kernel. Explorer, plan, verify, and builder workers share the exact parent
AI client, semantic progress policy, history shaping, cancellation, and an
inherited governance ceiling. They currently execute through a separate
non-streaming provider/tool mini-kernel and persist delegated lifecycle
artifacts rather than full child session recovery and canonical streaming
traces. Unifying that boundary may be worthwhile, but it is a bounded follow-up
refactor—not evidence that the provider, storage, tool, and client core should
be rewritten. Mako is the other explicit exception: `RunSpec` resolves its
inner-run contract before the higher-order tick driver owns scheduling.

## Rotten contracts removed

Before this remediation:

- a model could be represented by a bare slug, while provider, credential
  surface, API format, capabilities, and catalog provenance were inferred
  again later;
- OpenAI API-key and ChatGPT OAuth rows with the same slug could overwrite one
  another;
- server, TUI, ACP, and Mako assembled orchestration settings separately;
- the primary loop had a default 50-turn ceiling even when useful work was
  continuing;
- repeated read-only Bash calls could look productive when timestamps, PIDs,
  or log prefixes changed;
- a delegated agent could silently substitute a different model slug while
  reusing the parent's client and credentials;
- prompt, effective request options, and model capability decisions were hard
  to prove from a redacted runtime trace.

## Canonical runtime

The repaired flow is:

1. Resolve one `ModelKey`: provider, wire model ID, auth scope, and API format.
2. Freeze its catalog row as `ResolvedModelRuntime`, including context/output
   limits, tools, vision, reasoning controls, source, and revision.
3. Build one `AiClient` whose configured transport must match that runtime.
4. Resolve a validated `RunSpec` for server, TUI, ACP, or Mako. It owns session
   identity, canonical workspace, permission mode, run budget, timeout, work
   mode, cache/session key, and canonicalized request options.
5. Start the streaming orchestrator only through `RunSpec`. Direct
   orchestrator construction is crate-private. The delegated tool loop remains
   an explicit separate kernel until it can share the streaming engine without
   losing its narrower governance contract.
6. Prepare every provider turn through the same immutable runtime, prompt
   section builder, history transform, sorted tool set, capability filter, and
   retry owner.
7. Emit a redacted `ProviderRequestPrepared` snapshot and canonical loop events
   to persistence, SSE, TUI, ACP, and extension observers.

Request code may no longer change only a model string. Selecting another model
requires another exact runtime and client.

## Budget and convergence

Turn count is now a resource limit, not a loop detector.

- Interactive server, TUI, ACP, and delegated runs are unlimited by default.
- Every surface normalizes any compatibility setting into a typed per-run
  budget before `RunSpec`; that explicit budget overrides the project budget,
  which otherwise resolves to unlimited.
- Mako keeps an explicit finite per-tick budget because it is a scheduler, not
  an interactive session.
- A configured budget of `N` permits exactly `N` provider calls.

Loop convergence is semantic:

- Bash intent is normalized across cosmetic flags, `pwd`, `cd .`, wrappers,
  path spelling, and output-limit variations.
- Repeating the same read-only Bash intent does not become new evidence merely
  because stdout contains a new PID, timestamp, elapsed time, or log prefix.
  Explicit lifecycle status changes remain evidence.
- Write, edit, and patch tools publish a producer-owned `changed` contract.
  Successful opaque mutations get one provisional effect; repeating the same
  opaque intent is not fresh progress.
- Repeated failures and repeated successful validation are tracked by their
  own outcome-aware guards.
- The first no-progress repeat warns, the second injects a change-of-strategy
  instruction, and the third stops with a typed loop-guard reason.
- Steering advances the mutation epoch and resets evidence intentionally.

## Exact model behavior

The registry indexes exact keys and uses bare IDs only as a migration path that
succeeds when exactly one row matches. Ambiguity fails closed. Catalog refresh
preserves same-slug API-key and OAuth variants. Preferences, sessions, ACP
opaque model IDs, server requests, mobile/client state, and durable Mako work
carry exact keys plus catalog revision.

Capability policy comes from the frozen row. Unknown custom models receive a
conservative 32K context, 4K output, and no tools, vision, or reasoning
controls until explicit metadata says otherwise. Static vision/tool support is
declared on the model row, not guessed from a substring. Fractional provider
sampling parameters remain JSON fractions and are gated by provider identity.

OpenAI-compatible transport does not imply OpenAI-identical behavior. Grok now
resolves to its own prompt family before the shared Responses transport family,
including an explicit prohibition on placeholder/no-op tool calls after a tool
result when the latest user steering asks for a direct reply. Automated
validation reminders likewise require a successful producer-owned root
`changed: true`; a conservative "possibly effectful" shell classification is
not treated as proof that files changed.

## Why this is smaller than a rewrite

The retained pieces—provider parsers, tools, SQLite, compaction, permission
governance, clients, and product surfaces—already contain substantial tested
behavior. The simplification is ownership:

- one exact model runtime;
- one production run specification;
- one request/prompt/history policy path;
- one canonical loop-event/trace boundary;
- one semantic progress ledger;
- explicit scheduler and delegated-kernel exceptions.

This removes competing policy implementations while preserving the product's
real capabilities. A fresh core would still need to rediscover and retest all
of those contracts.

## Acceptance contract

Release evidence must include:

- deterministic build, read-only audit, long-run (more than 50 useful turns),
  cosmetic-loop, no-op mutation, repeated-validation, malformed-stream,
  transient-retry, and overflow scenarios;
- the complete repository check/test/clippy/format/web-export gate;
- an exact clean candidate commit built in an isolated Honey worktree;
- a direct Grok 4.5 simple/streaming smoke;
- repeated real Grok 4.5 project builds through the candidate server;
- a real read-only audit with zero mutations and an adversarial repeated-Bash
  run that converges with trace-backed evidence;
- resilience coverage for SSE disconnect, failed-Bash recovery, live steering,
  explicit cancellation, and direct-tool policy rejection. Repeated
  `tool_executing` liveness heartbeats may share one call ID, while start,
  complete, and result identities remain unique;
- cleanup proof for candidate processes and confirmation that production was
  never repointed during acceptance.

The executable commands and evidence format are documented in
`evaluation-and-live-grok.md`.
