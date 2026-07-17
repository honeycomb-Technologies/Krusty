# Multi-Provider AI Layer

Krusty talks to several AI providers -- Anthropic, OpenAI, MiniMax, Z.ai, OpenRouter, and Google -- through a single unified interface. This document explains how that works, from the high-level abstraction down to SSE byte parsing.

## The Problem

Every AI provider invented its own API. Anthropic uses a Messages API with content blocks and `x-api-key` authentication. OpenAI uses Chat Completions (and now a Responses API) with Bearer tokens and a completely different message shape. Google's Gemini API uses `contents` with `parts` and `functionDeclarations`. Even providers that claim Anthropic compatibility -- like MiniMax and Z.ai -- have quirks around thinking blocks, caching, and vision support.

Without an abstraction layer, every feature in Krusty would need provider-specific branches: separate code for sending messages, separate code for parsing streaming responses, separate code for tool calls, separate code for extended thinking. That sprawl would make adding a new provider a multi-week project touching dozens of files.

Krusty solves this with a layered architecture: a unified type system at the bottom, format handlers in the middle, and a single `AiClient` at the top that the rest of the application talks to.

## The Type System

Everything starts with the types defined in `crates/krusty-core/src/ai/types.rs`. These are Krusty's internal representation of conversations, completely independent of any provider's wire format.

The core types are:

- **`ModelMessage`** -- A message in a conversation, with a `Role` (System, User, Assistant, or Tool) and a vector of `Content` blocks.
- **`Content`** -- An enum covering every kind of content block: `Text`, `Image`, `Document`, `ToolUse` (the model requesting a tool call), `ToolResult` (the result coming back), `Thinking` (extended reasoning with a cryptographic signature), and `RedactedThinking`.
- **`AiTool`** -- A tool definition with name, description, JSON Schema for inputs, and an optional extended prompt that gets injected into the system prompt rather than sent to the provider.
- **`AiToolCall`** -- A completed tool invocation with its parsed arguments.

These types flow through the entire system. When the orchestrator builds a conversation, it constructs `ModelMessage` values. When a streaming response arrives, it gets parsed into `StreamPart` events that eventually become `Content` blocks stored in the conversation history.

Supporting types like `Usage` (with cache hit metrics), `ThinkingConfig`, `ContextManagement`, and `WebSearchConfig` round out the type system for provider-specific features that need to be expressed in a provider-neutral way.

## AiClient: The Central Abstraction

`AiClient` (in `crates/krusty-core/src/ai/client/core.rs`) is the single struct that all AI communication flows through. It holds three things: an HTTP client configured for SSE streaming (long timeouts, proper user-agent), an `AiClientConfig` describing which provider and format to use, and an API key.

The client exposes a small surface area. Simple non-streaming calls go through `call_simple()`. Streaming calls go through the streaming module. Extended thinking uses `call_with_thinking()`. Cache-aware conversation calls use `call_with_conversation()`. Each method internally routes to the correct format handler and parser based on the client's configured `ApiFormat`.

Authentication is handled at the request-building level. The `build_request()` method checks whether the provider expects `x-api-key` (Anthropic-style) or `Authorization: Bearer` (OpenAI-style) headers, adds Anthropic API version headers when appropriate, injects OAuth beta headers for Anthropic's Bearer token flow, and appends any custom headers the provider configuration specifies. The same logic extends to WebSocket connections via `build_websocket_request()`.

## AiClientConfig and CallOptions

`AiClientConfig` (in `crates/krusty-core/src/ai/client/config.rs`) captures everything needed to configure a client: the model ID, max tokens, optional base URL override, authentication style, provider ID, API format, and custom headers. Helper constructors like `for_anthropic_with_auth_detection()` and `for_openai_with_auth_detection()` handle the complexity of OAuth vs. API key routing, including choosing the correct endpoint (ChatGPT's Responses API vs. OpenAI's standard API) based on credential type.

`CallOptions` is the per-request configuration: max tokens, temperature, tools, system prompt, thinking config, reasoning format, caching, context management, web search/fetch, and provider-specific knobs like Codex reasoning effort and Anthropic adaptive thinking effort.

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

The provider registry (in `crates/krusty-core/src/ai/providers.rs`) is a lazily initialized, statically cached list of `ProviderConfig` entries. Each entry specifies the provider's ID, display name, base URL, authentication style, available models, and capabilities.

Five providers are built in:

- **MiniMax** -- Anthropic-compatible endpoint at `api.minimax.io`, uses `x-api-key` auth, offers MiniMax M2.5 with interleaved thinking.
- **OpenRouter** -- Anthropic-compatible at `openrouter.ai`, uses Bearer auth, supports 100+ dynamic models from multiple upstream providers.
- **Z.ai** -- Anthropic-compatible at `api.z.ai`, offers GLM-5.
- **Anthropic** -- Direct Anthropic API with OAuth or API key, offers Claude Opus 4.6 and Haiku 4.5.
- **OpenAI** -- Direct OpenAI API with OAuth or API key, offers GPT-5.3 Codex, GPT-5.4, and GPT-5.4 Mini.

`ProviderCapabilities` is a parallel structure that tracks feature support per provider: prompt caching, web search, web fetch, context management, web plugins (OpenRouter-style), and vision. This is what `CallOptions::canonicalized_for()` checks to strip unsupported features.

Adding a new provider means adding a `ProviderConfig` entry to the `BUILTIN_PROVIDERS` list, a `ProviderCapabilities` match arm, and (if it uses a new API format) a `FormatHandler` implementation. For Anthropic-compatible providers like MiniMax and Z.ai, only the first two are needed.

## Model Profiles and Capabilities

The model system has two layers. `ModelMetadata` (in `crates/krusty-core/src/ai/models.rs`) stores factual data about a model: context window, max output, reasoning format, pricing, vision support. The `ModelRegistry` is a thread-safe store (`Arc<RwLock>`) that holds models from all providers, supports O(1) lookup by ID via an index, and tracks recently used models.

`ModelProfile` (in `crates/krusty-core/src/ai/model_profile.rs`) captures behavioral characteristics tied to a model family. It determines the prompt family (AnthropicClaude, OpenAiCodex, OpenAiReasoning, GoogleGemini, or GenericCoding), context utilization ratios for compaction, stream drain policies, and whether the model supports reasoning summaries. Profiles are resolved from the provider, API format, and model ID using pattern matching on the model name.

Each profile also controls the layered system prompt: a base prompt (Krusty's operating contract), a provider guidance overlay (Anthropic gets "keep tool and plan state explicit"; OpenAI gets "preserve exact task continuity"), a model family overlay (Codex gets "continue through tool-use loops"; Gemini gets "ground decisions in explicit file evidence"), and a capability overlay based on context window size and API format. When a custom system prompt is provided, it replaces the entire layered stack.

## Streaming: SSE Parsing and Buffer Management

Streaming is where most of the complexity lives. The pipeline works like this:

1. The client sends an HTTP POST with `stream: true` and gets back a byte stream.
2. `SseStreamProcessor` (in `crates/krusty-core/src/ai/sse.rs`) receives byte chunks and handles SSE framing -- splitting on newlines, accumulating partial lines across chunk boundaries, stripping SSE comments and empty lines, and extracting `data:` payloads. It caps partial line buffers at 1MB to prevent unbounded memory growth.
3. Each SSE data payload is parsed as JSON and handed to a provider-specific `SseParser` implementation. Three parsers exist in `crates/krusty-core/src/ai/parsers/`: `AnthropicParser`, `OpenAIParser`, and `GoogleParser`. Each knows how to interpret its provider's event types and convert them into `SseEvent` values.
4. `SseEvent` values are mapped to `StreamPart` values (text deltas, tool call starts/deltas/completions, thinking events, usage, finish) and sent through an unbounded channel to the orchestrator.
5. Text deltas pass through a `StreamBuffer` (in `crates/krusty-core/src/ai/stream_buffer.rs`) that breaks text into 64-character chunks and flushes every 16ms, targeting 60fps rendering in the TUI. Non-text events bypass the buffer and go directly to the channel.

The `StreamDrainPolicy` from `ModelProfile` governs how the orchestrator drains the event channel: smooth mode processes small batches, moderate mode kicks in when the backlog exceeds a threshold, and catch-up mode activates for large backlogs. Codex models get more aggressive drain policies because they produce higher-volume output.

Tool calls are accumulated using `ToolCallAccumulator`, which collects argument fragments across multiple SSE events and attempts JSON parsing after each delta. When the stream finishes with a `tool_calls` finish reason, all accumulated tool calls are emitted as complete events. Thinking blocks use a similar `ThinkingAccumulator` that collects thinking text and cryptographic signatures across deltas.

## Extended Thinking

Extended thinking (in `crates/krusty-core/src/ai/client/thinking.rs`) lets models "think out loud" before responding. Krusty sends a `thinking` configuration with a budget in tokens, and the response includes `Thinking` content blocks alongside regular text.

For Anthropic, this requires beta headers (`interleaved-thinking-2025-05-14`) and, for Opus 4.5 models, an effort parameter with its own beta flag. The thinking budget is set independently of max output tokens, and max tokens must exceed the budget.

The streaming path handles thinking through `ThinkingStart`, `ThinkingDelta`, `SignatureDelta`, and `ThinkingComplete` events. The signature is a cryptographic value that Anthropic uses to validate thinking block integrity -- it must be preserved when sending thinking blocks back in subsequent requests. MiniMax uses the same thinking block structure but without signatures.

## Retry and Backoff

The retry module (in `crates/krusty-core/src/ai/retry/`) implements exponential backoff with jitter for transient API errors. `RetryConfig` specifies max retries, initial delay, max delay, and whether to add jitter. Three presets exist: default (5 retries, 1-32s), aggressive (8 retries, 2-60s), and gentle (3 retries, 0.5-8s).

The `with_retry()` function wraps any async operation. On failure, it checks the `IsRetryable` trait to determine if the error is transient (HTTP 429, 500, 502, 503, 504), respects `Retry-After` headers when the server specifies a wait time, adds random jitter (0-1000ms) to prevent thundering herd problems, and doubles the delay on each attempt up to the configured maximum.

## Format Auto-Detection

`detect_api_format()` (in `crates/krusty-core/src/ai/format_detection.rs`) provides the canonical mapping from provider to API format. OpenAI gets `ApiFormat::OpenAI`; everyone else gets `ApiFormat::Anthropic`. This is the fallback used when the caller does not specify a format explicitly.

More nuanced detection happens in `AiClientConfig::for_openai_with_auth_detection()`, which examines the credential type and model name to choose between Chat Completions, Responses API, and ChatGPT's backend API. GPT-5+ models and Codex models prefer the Responses API. ChatGPT OAuth tokens require the ChatGPT backend endpoint. Everything else uses Chat Completions.

## Model Translation and Fast Mode

The provider registry includes a model translation system. `ModelFamily` defines canonical model families (Claude Opus 4.6, Claude Sonnet 4, etc.) and `MODEL_MAPPINGS` maps them to provider-specific IDs. `translate_model_id()` converts between providers -- for example, `claude-opus-4-6` on Anthropic becomes `anthropic/claude-opus-4.6` on OpenRouter. `translate_model_or_default()` falls back to the target provider's default model when no mapping exists.

Krusty keeps model identity separate from request speed. Mini/Haiku models are explicit model selections, while the TUI/mobile fast toggle requests a provider service tier through `CallOptions::service_tier_for_provider()` without mutating the selected model ID.

## Dynamic Model Discovery

Providers marked with `dynamic_models: true` (OpenRouter and OpenAI) support runtime model discovery. The catalog module (`crates/krusty-core/src/ai/catalog.rs`) routes to provider-specific fetch functions that query each provider's model listing API, parse the response into `ModelMetadata`, and populate the `ModelRegistry`. Cached catalogs include a fingerprint hash and TTL (6 hours for OpenAI, 12 for OpenRouter) to avoid unnecessary refetches.

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
