# Agent Request Efficiency and Parity

Krusty's efficiency target is behavioral parity with mature coding-agent harnesses: keep the repeated request prefix small and stable, expose enough tools to act without serializing the entire catalog, measure the request that is actually rendered, and compact without losing the active objective. This is a set of implementation contracts and regression gates, not a claim that one synthetic benchmark proves overall agent quality.

The upstream comparison in this document is pinned to Pi commit [`0e6909f`](https://github.com/badlogic/pi-mono/tree/0e6909f050eeb15e8f6c05185511f3788357ddb3) and OpenCode commit [`7255ce9`](https://github.com/anomalyco/opencode/tree/7255ce9a575315e3c5917cb341efc2c6b4a5a6b8), inspected on 2026-07-13. Grok Build observations use the official 0.2.99 binary with exported local traces; the older 0.2.33 reverse-engineering artifact is not treated as current authority.

## Upstream signals

Pi defaults to four coding tools (`read`, `write`, `edit`, and `bash`) and allows an explicit tool allowlist. Its prompt is assembled from the selected tools, concise guidelines, project instructions, skills, date, and working directory. See Pi's [coding-agent README](https://github.com/badlogic/pi-mono/blob/b084d2fb395f0f1aa924cb07b14e5d0edab115e2/packages/coding-agent/README.md) and [system-prompt builder](https://github.com/badlogic/pi-mono/blob/b084d2fb395f0f1aa924cb07b14e5d0edab115e2/packages/coding-agent/src/core/system-prompt.ts).

Pi also combines provider usage with an estimate for messages added after the last usage frame, reserves output space, keeps a recent verbatim tail, and stores the lossy summary without deleting the source JSONL history. Its OpenAI Codex transport uses a session-scoped cache key and reuses a WebSocket with `previous_response_id` only when the next request is an exact extension of the prior input. See Pi's [compaction implementation](https://github.com/badlogic/pi-mono/blob/b084d2fb395f0f1aa924cb07b14e5d0edab115e2/packages/coding-agent/src/core/compaction/compaction.ts), [usage type](https://github.com/badlogic/pi-mono/blob/b084d2fb395f0f1aa924cb07b14e5d0edab115e2/packages/ai/src/types.ts), and [Codex Responses transport](https://github.com/badlogic/pi-mono/blob/b084d2fb395f0f1aa924cb07b14e5d0edab115e2/packages/ai/src/api/openai-codex-responses.ts).

OpenCode chooses a model-family prompt, layers environment, project instructions, skills, and MCP instructions, filters tools through permission rules, and sorts the final tool map before sending it. It applies provider cache markers, uses the session ID as a provider cache key where supported, records uncached input/output/reasoning/cache-read/cache-write buckets, prunes old tool outputs, and summarizes an older head while retaining a recent tail. See OpenCode's [system prompt selection](https://github.com/anomalyco/opencode/blob/5401ebaededec3e2b6c1f2e0d20246ef68574598/packages/opencode/src/session/system.ts), [request preparation](https://github.com/anomalyco/opencode/blob/5401ebaededec3e2b6c1f2e0d20246ef68574598/packages/opencode/src/session/llm/request.ts), [provider transforms](https://github.com/anomalyco/opencode/blob/5401ebaededec3e2b6c1f2e0d20246ef68574598/packages/opencode/src/provider/transform.ts), and [compaction pipeline](https://github.com/anomalyco/opencode/blob/5401ebaededec3e2b6c1f2e0d20246ef68574598/packages/opencode/src/session/compaction.ts).

Krusty follows these principles without copying either product's exact prompt or tool set. Its direct surface is larger than Pi's because user interaction, delegation, patching, search, and plan lifecycle are first-class core behaviors. It avoids paying for the rest of the registry on every turn by making specialist tools lazy. Controlled traces showed that identity/personality text was not the dominant cost: Krusty's base prompt was smaller than Pi's and substantially smaller than OpenCode's, while tool schemas and repeated action turns determined most practical overhead.

## Compact tool surface

`ToolRequestPolicy` selects and alphabetically sorts the function tools placed on the wire:

- Normal code sessions expose at most nine direct tools. GPT/Codex families receive `apply_patch` plus direct discovery tools. Grok, Claude, Gemini, Kimi, and generic families receive `edit` and `write` instead of the GPT-shaped patch grammar; `glob` moves behind `tool_search` on that surface to keep the same fixed tool count.
- Read-only plan mode exposes at most eight tools selected for inspection, questions, delegation, and leaving plan mode.
- An active implementation plan exposes eleven direct tools so canonical task lifecycle operations cannot become unreachable behind a generic dispatcher.
- Chat, ACP, Mako, disabled-tool, permission, and delegation rules apply their own surface-specific registrations and filters at their boundaries.

The registry still owns the complete built-in, MCP, extension, and plugin catalog. `tool_search` provides three bounded operations:

1. `search` returns up to twelve relevant deferred tools with short descriptions and policy metadata.
2. `describe` returns the selected tool's schema and at most 8 KiB of extended guidance.
3. `execute` dispatches the target through the same registry hooks, timeout, filesystem scope, plan-mode rule, inherited delegation policy, and supervised-approval classification as a direct call.

Lifecycle and interactive tools are deliberately non-deferred. Approval text, retry policy, and model-history retention are computed from the effective target of a deferred call, not from the harmless-looking `tool_search` wrapper. The tradeoff is explicit: a rare specialist operation may require a discovery or description round trip, while every ordinary turn avoids serializing dozens of schemas and manuals.

Independent read-only calls execute concurrently while results remain in provider call order. Mutations, interactive operations, approvals, and delegated agents remain serialized; this prevents same-path races while removing avoidable read latency. Autonomous governance uses deterministic local fast paths for safe reads, common project build/test commands, in-workspace edits, and inherited delegated contracts. The small LLM classifier is reserved for ambiguous commands such as unrecognized network or external-system operations, and obvious unsafe payloads still fail closed locally.

Successful edits run bounded mutation diagnostics before their result is returned: JSON/TOML/YAML syntax is parsed and `git diff --check` is run for changed paths. A successful mutation also activates a loop-local verification reminder until a relevant test, build, lint, typecheck, `git diff --check`, or verify-agent call succeeds. These checks provide immediate feedback without silently running a project-wide formatter or expensive test suite after every keystroke.

## Stable prompt and continuation layers

The repeated instruction prefix has three layers:

1. The base coding contract is compact and slow-changing. A small model-family overlay adds only behavior that differs materially by provider family.
2. Project instructions are separated from live session state so they can remain in the reusable prefix.
3. Active plan, Mako coordinator, task, memory, report, and other volatile state is appended as current runtime context. On OpenAI Responses and Codex paths it retains `developer` authority rather than being demoted to user content.

Detailed tool manuals are not duplicated into the base system prompt. Direct tools rely on their function description and JSON Schema; deferred guidance is loaded only by `tool_search.describe`. Tool definitions are sorted by name before provider conversion.

Provider transports preserve this layering:

- Anthropic-compatible caching emits the stable base and project sections before uncached session context, with explicit cacheable system blocks only for providers that advertise support. `KRUSTY_CACHE_RETENTION=long` requests Anthropic's one-hour cache lifetime.
- OpenAI Responses keeps base plus project instructions stable, places runtime context at the tail, supplies a normalized session cache key of at most 64 characters when enabled, and requests low text verbosity. Newer cache-option fields are model-gated. Extended retention requests 24 hours only for model families that support it; GPT-5.6+ keeps its supported 30-minute TTL.
- ChatGPT Codex keeps a bounded session-keyed WebSocket pool with five-minute idle eviction and a maximum 55-minute reuse age. A warm request uses `previous_response_id` and sends only the new input when the stable request fingerprint, exact message prefix, prior assistant output, and runtime-context transition all match. Any incompatible change resets continuation to a full request; a missing previous response is retried once with full context.

Interactive streaming setup retries only typed transient provider statuses (`429`, `500`, `502`, `503`, `504`, and overload `529`) and definite connection-establishment failures. It makes at most three retries with exponential backoff, jitter, and an eight-second cap on provider-supplied `Retry-After`; authentication, payment, permission, malformed-request, and ambiguous post-send timeout failures remain terminal. Once any text, thinking, client tool, or hosted server-tool activity has appeared, Krusty does not replay the provider call. Canceling either request setup or an open HTTP/WebSocket stream drops the upstream response promptly so hidden generation does not continue after the user stops a turn.

Request telemetry records component byte/token estimates, tool count, wire-body size, cache mode, and a SHA-256 request-shape fingerprint. The fingerprint covers stable prompt and tool material but excludes conversation history and volatile session context; logs do not contain the prompt text or schema bodies.

## Rendered-request budgeting and compaction

Compaction pressure is based on the maximum of two estimates:

- A preflight rendered-request estimate built from the same prompt-section builder and provider-specific tool shapes as the outgoing call. It counts base prompt, project context, volatile session context, non-system messages, native hosted tools, and function schemas.
- The last provider-reported logical input plus an estimate for messages appended after that usage frame.

The rendered estimate also separates fixed overhead from reducible conversation history. The compactor receives both values, shrinks the retained-tail budget across bounded attempts, and reports when base instructions, project state, runtime state, and tool schemas alone make the target impossible. The ChatGPT Codex runtime cap is resolved centrally so the orchestrator, TUI, and server pinch route use the same effective context window.

The in-place pipeline then:

1. Removes or summarizes old tool output according to the tool's model-history retention policy.
2. Chooses a safe cut that preserves recent complete turns and does not separate a tool result from its call.
3. Produces a bounded structured continuation summary, carrying prior summary semantics forward without recursively nesting raw summaries or duplicating the canonical active plan.
4. Keeps the recent verbatim tail and inserts an explicit compaction boundary.
5. Stores the removed messages as a versioned typed segment for `search_compaction_segments` recovery.
6. Commits checkpoint, segment, message replacement, and context ledger updates atomically after verifying that the persisted transcript still matches the snapshot that was summarized.

Automatic pressure, manual `/pinch`, and provider-overflow recovery share this pipeline. A provider context-overflow error gets one compaction-and-retry attempt before it becomes terminal.

## Usage semantics

Krusty's normalized input and completion buckets are intentionally non-overlapping. `reasoning_tokens` is the one explicit subset, included for observability:

| Field | Meaning |
| --- | --- |
| `prompt_tokens` | Uncached input only |
| `cache_creation_input_tokens` | Input written to a provider cache |
| `cache_read_input_tokens` | Input restored from a provider cache |
| `completion_tokens` | Generated output, including provider-reported reasoning |
| `reasoning_tokens` | Reasoning/thinking contained within `completion_tokens`; observability only, never added to totals again |
| `input_tokens()` | Sum of the three input buckets; this is the logical context input |
| `logical_total_tokens()` | The larger of the provider total and normalized input plus completion; reasoning is already inside completion |

Streaming snapshots merge by bucket maximum rather than addition because providers may repeat cumulative counters. The Rust client, server SSE contract, TypeScript API client, and shared mobile state all carry logical input, cache, completion, and reasoning metrics. Live cumulative snapshots remain visible to clients, but runtime observability stores them as `usage_snapshot` rows and separately records one final `provider_call` row with a stable call ID. Canonical turns use `call_kind=agent_loop`; classifiers and compaction/pinch summaries use `call_kind=auxiliary` plus a stable operation label. Non-streaming transports return `SimpleCallResult { text, usage: Option<Usage> }`, so a compatible provider that omits usage is represented as missing data rather than a false zero. Parity totals use only final provider-call rows, so Anthropic-style input-at-start/output-at-end frames cannot be counted as two calls or summed twice. Trace writes run off the presentation path in compact transactions, adjacent text/thinking/tool-output deltas are coalesced without crossing lifecycle boundaries, and each session retains its newest 20,000 trace rows. Usage is a must-deliver terminal event on the bounded server stream; if a client receives an explicit lag signal, it reloads canonical session state after completion.

Dynamic request context has both source-local bounds and a 64 KiB aggregate ceiling. When several sources are simultaneously large, work mode, environment, active plan/task state, and project instructions are retained ahead of optional memory/report previews. Persistent memory requires meaningful objective overlap: generic words such as “project,” “code,” or “system” do not pull a catalog of loosely related memories into every turn. Global skills live at `~/.krusty/skills/`, project skills at `.krusty/skills/`, and only bounded metadata is included until a skill is explicitly loaded.

## Regression gates

These tests are intended to make prompt and context growth visible during review:

| Gate | Contract |
| --- | --- |
| `core_prompt_stays_compact_and_keeps_critical_contracts` | Base prompt stays at or below 2,500 bytes and 400 words while retaining safety and completion clauses. |
| `default_wire_surface_is_bounded_but_catalog_remains_reachable` | Default direct tools remain bounded, the lazy catalog remains reachable, and base prompt plus OpenAI Responses tool schemas stay under the configured fixed-request ceiling. |
| `warm_continuation_wire_payload_is_under_ten_percent_of_cold_history` | A deterministic warm Codex continuation sends less than 10% of the equivalent synthetic cold request bytes. |
| Rendered-request estimator tests | Fixed prompt and provider-wire tools are counted; stable fingerprints survive tool reordering and volatile context/history changes. |
| Two-compaction regression | Repeated compaction keeps prior semantics bounded and structured without raw-summary nesting or active-plan duplication. |
| Provider usage parser tests | Cache reads/writes are not double-counted; OpenAI/Gemini reasoning is observable as a completion subset and never added twice. |
| Bounded SSE tests | Lag is explicit and usage/approval/terminal events survive a full client queue. |
| Concurrent read execution | Independent read-only calls overlap while tool results preserve the provider's original call order. |
| Model-family mutation surface | Grok receives `edit`/`write`; GPT/Codex retains `apply_patch`; both surfaces stay bounded. |
| Mutation diagnostics and verification state | Structured syntax/whitespace diagnostics are returned after edits and failed validation cannot clear pending verification. |

For live parity measurement, run `scripts/agent-parity-report.sh [database-path] <session-id>`. It reports each runtime run separately, including exact finalized agent-loop and auxiliary calls, operation labels, calls whose provider omitted usage, tool calls, failures, elapsed time, uncached input, cache writes, cache reads, output/reasoning, and logical total. The report never adds cumulative `usage_snapshot` rows. Session titles are derived locally and consume no hidden provider call. Delegated runs are listed separately so a compact parent-session UI cannot conceal child work. Fair external comparisons must use the same repository snapshot, task, model, reasoning level, permission mode, skill/MCP catalog, and warm/cold-cache condition.

Repository release validation remains broader than these focused gates: `cargo check --workspace`, `cargo test --workspace`, `cargo clippy --workspace -- -D warnings`, `cargo fmt --all`, and `npx expo export --platform web` from `apps/mobile`.

## What these gates do not prove

- The byte-to-token estimator is deliberately conservative and deterministic; it is not a provider tokenizer and is calibrated by real usage only after a provider reports usage.
- The warm-continuation test measures serialized request bytes, not latency, billed tokens, cache-hit rate, or answer quality on a live provider.
- Matching upstream architectural behaviors does not establish that Krusty is objectively better than Pi, OpenCode, Codex, or another agent. That requires repeatable task suites, live-provider measurements, failure-rate tracking, and human evaluation.
- Cache behavior is provider- and model-dependent. A stable prefix and correct cache fields make hits possible; the provider decides whether a request actually hits.
