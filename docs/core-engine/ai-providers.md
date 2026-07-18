# Multi-Provider AI Layer

Krusty exposes six selectable AI providers -- MiniMax, Anthropic, OpenAI, Grok, Z.ai, and OpenRouter -- through a single unified interface. Google/Gemini remains a supported wire format inside the abstraction layer, but it is not a selectable `ProviderId`. This document explains how the system works, from the high-level abstraction down to SSE byte parsing.

## The Problem

Every AI provider invented its own API. Anthropic uses a Messages API with content blocks and `x-api-key` authentication. OpenAI uses Chat Completions (and now a Responses API) with Bearer tokens and a completely different message shape. Google's Gemini API uses `contents` with `parts` and `functionDeclarations`. Compatibility surfaces still have provider-specific contracts: MiniMax is Anthropic-compatible, Z.ai's Coding Plan is OpenAI Chat Completions-compatible, OpenRouter uses its own Messages schema, and Grok's subscription proxy uses an OpenAI Responses-style transport.

Without an abstraction layer, every feature in Krusty would need provider-specific branches: separate code for sending messages, separate code for parsing streaming responses, separate code for tool calls, separate code for extended thinking. That sprawl would make adding a new provider a multi-week project touching dozens of files.

Krusty solves this with a layered architecture: a unified type system at the bottom, format handlers in the middle, and a single `AiClient` at the top that the rest of the application talks to.

## The Type System

Everything starts with the types defined under `crates/krusty-core/src/ai/types/`. These are Krusty's internal representation of conversations, completely independent of any provider's wire format.

The core types are:

- **`ModelMessage`** -- A message in a conversation, with a `Role` (System, User, Assistant, or Tool) and a vector of `Content` blocks.
- **`Content`** -- An enum covering every kind of content block: `Text`, `Image`, `Document`, `ToolUse` (the model requesting a tool call), `ToolResult` (the result coming back), `Thinking` (extended reasoning with a cryptographic signature), and `RedactedThinking`.
- **`AiTool`** -- A tool definition with name, description, JSON Schema for inputs, and an optional extended prompt that gets injected into the system prompt rather than sent to the provider.
- **`AiToolCall`** -- A completed tool invocation with its parsed arguments.

These types flow through the entire system. When the orchestrator builds a conversation, it constructs `ModelMessage` values. When a streaming response arrives, it gets parsed into `StreamPart` events that eventually become `Content` blocks stored in the conversation history.

Supporting types like `Usage` (with cache hit metrics), `ThinkingConfig`, `ContextManagement`, and `WebSearchConfig` round out the type system for provider-specific features that need to be expressed in a provider-neutral way.

## AiClient: The Central Abstraction

`AiClient` (in `crates/krusty-core/src/ai/client/core/client.rs`) is the single struct that all AI communication flows through. It holds three things: an HTTP client configured for SSE streaming (long timeouts, proper user-agent), an `AiClientConfig` describing which provider and format to use, and an API key.

The client exposes a small surface area. Legacy text-only non-streaming calls go through `call_simple()` and `call_with_conversation()`; their usage-bearing counterparts return `SimpleCallResult`, whose optional normalized usage distinguishes an omitted provider field from a real zero. Streaming calls go through the streaming module. Extended thinking uses `call_with_thinking()`. Each method internally routes to the correct format handler and parser based on the client's configured `ApiFormat`, and streaming/non-streaming paths share the same usage normalizers.

Authentication is handled at the request-building level. The `build_request()` method checks whether the provider expects `x-api-key` (Anthropic-style) or `Authorization: Bearer` (OpenAI-style) headers, adds Anthropic API version headers when appropriate, injects OAuth beta headers for Anthropic's Bearer token flow, and appends any custom headers the provider configuration specifies. The same logic extends to WebSocket connections via `build_websocket_request()`.

## AiClientConfig and CallOptions

`AiClientConfig` (in `crates/krusty-core/src/ai/client/config/ai_client.rs`) captures everything needed to configure a client: the model ID, max tokens, optional base URL override, authentication style, provider ID, API format, and custom headers. Helper constructors like `for_anthropic_with_auth_detection()` and `for_openai_with_auth_detection()` handle the complexity of OAuth vs. API key routing, including choosing the correct endpoint (ChatGPT's Responses API vs. OpenAI's standard API) based on credential type.

`CallOptions` is the per-request configuration: max tokens, temperature, tools, system prompt, thinking config, reasoning format, caching, context management, web search/fetch, and provider-specific knobs like Codex reasoning effort, Anthropic adaptive thinking effort, and the model-specific Fast implementation.

The key method is `canonicalized_for()`, which normalizes a `CallOptions` for a specific provider/model combination. It strips features the provider does not support (web search, context management, parallel tool calls), aligns the reasoning format with what the model actually uses (Anthropic thinking vs. OpenAI reasoning vs. DeepSeek), and removes conflicting effort controls. This prevents configuration drift between different call sites -- everyone gets the same canonicalized options.

## The Format Abstraction Layer

The format layer (in `crates/krusty-core/src/ai/format/`) is where provider differences get absorbed. The `FormatHandler` trait defines three methods:

- `convert_messages()` -- Transform Krusty's `ModelMessage` values into provider-specific JSON.
- `convert_tools()` -- Transform `AiTool` definitions into the provider's tool format.
- `build_request_body()` -- Assemble the complete request JSON with model, messages, options, and provider-specific fields.

Three implementations exist:

**AnthropicFormat** handles the Anthropic Messages API. This is the most complex handler because Anthropic requires strict user/assistant message alternation and has detailed rules around thinking blocks. The handler inserts filler messages when consecutive same-role messages would violate alternation, manages thinking block preservation (MiniMax wants all thinking blocks preserved; Anthropic only wants the last one with a valid signature), strips images for providers without vision support, and runs a post-processing sanitization pass that repairs orphaned tool results and injects stub results for interrupted tool calls.

**OpenAIFormat** handles both the Chat Completions API and the newer Responses API, selected by the `ApiFormat` variant. It translates tool calls into OpenAI's `function` wrapper format, omits durable thinking blocks instead of replaying them as assistant plaintext, detects orphaned tool calls from interrupted sessions and adds placeholder results, and handles the structural differences between Chat Completions (messages with `content`) and Responses (input with `input_text`/`input_image`).

**GoogleFormat** converts to Google's `contents`/`parts` structure, mapping tool calls to `functionCall` and tool results to `functionResponse`, and routing images through either `inline_data` (base64) or `file_data` (URL).

The factory function `get_format_handler()` selects the right implementation based on `ApiFormat`.

## Provider Registry

The provider registry (under `crates/krusty-core/src/ai/providers/registry/`) is a lazily initialized, statically cached list of `ProviderConfig` entries. Each entry specifies the provider's ID, display name, base URL, authentication style, curated fallback models, and capabilities. Live catalog results replace those fallback rows when discovery succeeds.

Six selectable providers are built in:

| Provider | Live catalog | Curated fallback | Important transport behavior |
| --- | --- | --- | --- |
| **OpenAI** | API keys query `/v1/models`; ChatGPT OAuth queries the account-scoped Codex catalog. If both identities exist, Krusty merges them by model ID and prefers the richer ChatGPT metadata. | GPT-5.6 Sol/Terra/Luna, GPT-5.5/5.5 Pro, GPT-5.4/5.4 Pro/5.4 Mini/5.4 Nano, Chat Latest, and GPT-5.3 Codex/Spark. | Successful live API/OAuth discovery is authoritative because each identity can expose different entitlements, context limits, reasoning presets, and Fast eligibility. |
| **Anthropic** | Paginates `/v1/models` with either API-key or OAuth authentication. | Claude Opus 4.8, Fable 5, Sonnet 5, and Haiku 4.5. | Catalog capabilities are used when present; curated family overlays fill sparse thinking, context, vision, and Fast metadata. |
| **MiniMax** | Paginates the Anthropic-compatible `/anthropic/v1/models` endpoint. | MiniMax M3, M2.7, and M2.7 Highspeed. | The live list supplies availability while curated family overlays make M3 adaptive-thinking and Priority-capable; M2 reasoning is mandatory, and Highspeed remains a distinct model ID. |
| **Grok** | Queries the authenticated Grok CLI subscription proxy's `/models` endpoint. | Grok Build and Composer 2.5. | This is the subscription CLI transport, not the public xAI API; reasoning output can be displayed, but the curated proxy contract does not expose an effort selector. |
| **OpenRouter** | Queries `/api/v1/models`, keeps every usable vendor model rather than applying a family allowlist, and consumes catalog reasoning, service-tier, modality, and pricing metadata. | A small tool-capable set covering current Claude and GPT families. | The OpenRouter Messages schema owns the request shape even when the routed upstream model is OpenAI or Anthropic. |
| **Z.ai** | No supported model-list endpoint; this provider is static. | GLM 5.2, GLM 5 Turbo, and GLM 4.7. | The Coding Plan uses OpenAI-compatible Chat Completions with Bearer auth. Thinking is encoded with top-level `thinking.type`; `reasoning_effort` is sent only for models whose metadata exposes graded effort. |

`ProviderCapabilities` is a parallel structure that tracks feature support per provider: prompt caching, web search, web fetch, context management, web plugins (OpenRouter-style), and vision. This is what `CallOptions::canonicalized_for()` checks to strip unsupported features.

Adding a new provider means adding a `ProviderConfig` entry to the `BUILTIN_PROVIDERS` list, a `ProviderCapabilities` match arm, and (if it uses a new API format) a `FormatHandler` implementation. A dynamic provider also needs a catalog-listing adapter. Existing compatible formats can be reused: MiniMax uses the Anthropic handler, while Z.ai uses the OpenAI handler.

## Model Profiles and Capabilities

The model system has two layers. `ModelMetadata` (in `crates/krusty-core/src/ai/models/metadata.rs`) stores factual data about a model: context window, max output, reasoning format, exact selectable reasoning levels, provider default, whether reasoning is mandatory, request control type, Fast implementation, pricing, and vision support. The `ModelRegistry` is a thread-safe store (`Arc<RwLock>`) that holds models from all providers, supports O(1) lookup by ID via an index, and tracks recently used models. Server, TUI, mobile, and desktop clients prefer this metadata over guessing capabilities from model names; compatibility heuristics remain for older responses that do not include it.

`ModelProfile` (in `crates/krusty-core/src/ai/model_profile/profile/mod.rs`) captures behavioral characteristics tied to a model family. It determines the prompt family (AnthropicClaude, OpenAiCodex, OpenAiReasoning, GoogleGemini, or GenericCoding), context utilization ratios for compaction, stream drain policies, and whether the model supports reasoning summaries. Profiles are resolved from the provider, API format, and model ID using pattern matching on the model name.

Each profile also controls the layered system prompt: a base prompt (Krusty's operating contract), a provider guidance overlay (Anthropic gets "keep tool and plan state explicit"; OpenAI gets "preserve exact task continuity"), a model family overlay (Codex gets "continue through tool-use loops"; Gemini gets "ground decisions in explicit file evidence"), and a capability overlay based on context window size and API format. When a custom system prompt is provided, it replaces the entire layered stack.

## Streaming: SSE Parsing and Buffer Management

Streaming is where most of the complexity lives. The pipeline works like this:

1. The client sends an HTTP POST with `stream: true` and gets back a byte stream.
2. `SseStreamProcessor` (in `crates/krusty-core/src/ai/sse/processor/mod.rs`) receives byte chunks and handles SSE framing -- splitting on newlines, accumulating partial lines across chunk boundaries, stripping SSE comments and empty lines, and extracting `data:` payloads. It caps partial line buffers at 1MB to prevent unbounded memory growth.
3. Each SSE data payload is parsed as JSON and handed to a provider-specific `SseParser` implementation. Three parsers exist in `crates/krusty-core/src/ai/parsers/`: `AnthropicParser`, `OpenAIParser`, and `GoogleParser`. Each knows how to interpret its provider's event types and convert them into `SseEvent` values.
4. `SseEvent` values are mapped to `StreamPart` values (text deltas, tool call starts/deltas/completions, thinking events, usage, finish) and sent through an unbounded channel to the orchestrator.
5. Text deltas pass through a `StreamBuffer` (in `crates/krusty-core/src/ai/stream_buffer.rs`) that breaks text into 64-character chunks and flushes every 16ms, targeting 60fps rendering in the TUI. Non-text events bypass the buffer and go directly to the channel.

The `StreamDrainPolicy` from `ModelProfile` governs how the orchestrator drains the event channel: smooth mode processes small batches, moderate mode kicks in when the backlog exceeds a threshold, and catch-up mode activates for large backlogs. Codex models get more aggressive drain policies because they produce higher-volume output.

Tool calls are accumulated using `ToolCallAccumulator`, which collects argument fragments across multiple SSE events and attempts JSON parsing after each delta. When the stream finishes with a `tool_calls` finish reason, all accumulated tool calls are emitted as complete events. Thinking blocks use a similar `ThinkingAccumulator` that collects thinking text and cryptographic signatures across deltas.

## Extended Thinking

Extended thinking (in `crates/krusty-core/src/ai/client/thinking.rs`) lets reasoning models return `Thinking` content blocks alongside regular text. The model catalog, rather than a global hard-coded cycle, determines which of `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, and `max` are selectable. Requests for an unavailable level are normalized to the catalog default (or the first valid level), and models marked as mandatory reasoning cannot be switched off. `ultra` is retained only for backward-compatible parsing; it is never advertised as selectable and a legacy `ultra` request normalizes to `max`.

The request encoding is provider-specific:

- **OpenAI/ChatGPT** uses OpenAI reasoning effort. Entitlement-specific ChatGPT catalog levels are authoritative.
- **Anthropic adaptive families** use `thinking: { type: "adaptive" }` plus `output_config.effort`; Sonnet 5 sends `thinking: { type: "disabled" }` for an explicit Off selection because omission enables thinking on that family, while always-on Fable 5 cannot be disabled. Older families retain budget-based thinking and the interleaved-thinking beta.
- **OpenRouter Messages** uses `thinking: { type: "enabled" }` plus `output_config.effort`, regardless of the routed upstream provider.
- **MiniMax M3** uses the Anthropic-compatible `thinking: { type: "adaptive" }` toggle without an effort or token budget. M2-family reasoning is mandatory, so Krusty omits a conflicting optional thinking object.
- **Z.ai** uses top-level `thinking: { type: "enabled" | "disabled" }`; `reasoning_effort` accompanies it only for models such as GLM 5.2 whose catalog metadata exposes graded effort. It never receives an Anthropic budget object.
- **Grok's subscription proxy** can return reasoning output, but explicit effort controls are suppressed for the output-only transport contract.

The streaming path handles thinking through `ThinkingStart`, `ThinkingDelta`, `SignatureDelta`, and `ThinkingComplete` events. The signature is a cryptographic value that Anthropic uses to validate thinking block integrity -- it must be preserved when sending thinking blocks back in subsequent requests. MiniMax uses the same thinking block structure but without signatures.

## Retry and Backoff

The retry module (in `crates/krusty-core/src/ai/retry/`) implements exponential backoff with jitter for transient API errors. `RetryConfig` specifies max retries, initial delay, max delay, and whether to add jitter. Three presets exist: default (5 retries, 1-32s), aggressive (8 retries, 2-60s), and gentle (3 retries, 0.5-8s).

The `with_retry()` function wraps any async operation. On failure, it checks the `IsRetryable` trait to determine if the error is transient (HTTP 429, 500, 502, 503, 504), respects `Retry-After` headers when the server specifies a wait time, adds random jitter (0-1000ms) to prevent thundering herd problems, and doubles the delay on each attempt up to the configured maximum.

## Format Auto-Detection

`detect_api_format()` (in `crates/krusty-core/src/ai/format_detection.rs`) provides the canonical mapping from provider to API format. OpenAI and Z.ai use `ApiFormat::OpenAI`; Grok uses `ApiFormat::OpenAIResponses`; Anthropic, MiniMax, and OpenRouter use `ApiFormat::Anthropic`. This is the fallback used when the caller does not specify a format explicitly. `ApiFormat::Google` is available to explicit internal/custom configurations, but there is no selectable Google provider.

More nuanced detection happens in `AiClientConfig::for_openai_with_auth_detection()`, which examines the credential type and model name to choose between Chat Completions, Responses API, and ChatGPT's backend API. GPT-5+ models and Codex models prefer the Responses API. ChatGPT OAuth tokens require the ChatGPT backend endpoint. Everything else uses Chat Completions.

## Model Translation and Standard/Fast Mode

The provider registry includes a model translation system. `ModelFamily` defines canonical model families and `MODEL_MAPPINGS` maps them to provider-specific IDs. `translate_model_id()` converts between providers; `translate_model_or_default()` falls back to the target provider's default model when no mapping exists. Translation is separate from live catalog metadata and must not be used to infer reasoning or Fast support.

Krusty also keeps model identity separate from request speed. Standard is represented by omitting a speed override. Fast is enabled only when the selected model advertises an implementation:

- OpenAI, OpenRouter, and MiniMax models whose catalog metadata advertises Priority send `service_tier: "priority"`.
- Anthropic Fast Mode sends `speed: "fast"` and the `fast-mode-2026-02-01` beta header.
- MiniMax M3 supports the per-request Priority tier; MiniMax Highspeed entries remain distinct model IDs.
- Models without a catalog `fast_mode` value keep the control disabled.

## Dynamic Model Discovery

Providers marked with `dynamic_models: true` (OpenAI, Anthropic, MiniMax, Grok, and OpenRouter) support runtime model discovery. The catalog module (`crates/krusty-core/src/ai/catalog.rs`) resolves provider credentials, routes to the provider-specific listing adapter, and parses each result into the shared `ModelMetadata` contract. Z.ai remains on curated static metadata because it does not expose a supported model-list endpoint.

Catalog startup and refresh are deliberately stale-safe:

1. Seed every provider with curated fallback models so model selection works without network access.
2. Restore the last-known-good cached snapshots immediately, then reapply custom models.
3. Force one credential-backed revalidation sweep concurrently at startup, without delaying router startup, then use provider TTLs for later refreshes. OpenAI's API-key and ChatGPT OAuth identities are fetched independently, and both must complete before their entitlement-scoped rows are merged.
4. Replace and persist a provider catalog only after a complete, non-empty success. Malformed pagination, an empty identity catalog, or any failed identity leaves the last-known-good snapshot untouched.
5. Check for stale catalogs every five minutes while the server runs; TTLs gate the actual network calls.

Credential changes invalidate the affected provider's cache before a canonical refresh. Provider-specific singleflight locks prevent duplicate fetches, and an authentication generation check discards results that began under older credentials. OpenAI catalog rows also retain API-key or ChatGPT OAuth provenance so the selected model is routed through the transport whose capabilities were advertised instead of guessing from its slug.

ChatGPT catalog requests identify themselves with a Codex protocol compatibility version rather than Krusty's package version. The stable default is `0.144.4`; set `KRUSTY_CODEX_CLIENT_VERSION` when a newer server contract requires an explicit compatibility override.

The current TTLs are 5 minutes for OpenAI, 6 hours for Anthropic and MiniMax, 12 hours for OpenRouter, and 24 hours for Grok. Cache metadata records fetch time, model count, and a fingerprint over model capabilities, auth provenance, and pricing, so missing or corrupted snapshots are treated as stale. This is a last-known-good cache, not a source of model truth: the live provider catalog wins whenever a refresh succeeds, while curated fallbacks remain the safety net when discovery is unavailable.

## How It All Fits Together

When Krusty needs to send a message:

1. The orchestrator builds a `Vec<ModelMessage>` from the conversation history.
2. `CallOptions` are canonicalized for the current provider and model.
3. The system prompt is assembled from the model profile's layered sections.
4. A `FormatHandler` converts messages and tools into provider-specific JSON.
5. `AiClient` builds the HTTP request with correct auth headers and sends it.
6. The SSE stream is parsed by the provider-specific parser, buffered for smooth rendering, and forwarded as `StreamPart` events.
7. The orchestrator processes events -- accumulating text, executing tool calls, storing thinking blocks -- until the stream finishes or the model requests tool execution.

The entire provider surface is contained within the `ai/` module. The rest of Krusty never sees Anthropic JSON, OpenAI message formats, or Google content blocks. It works exclusively with `ModelMessage`, `Content`, `StreamPart`, and `AiTool`.
