# Krusty Audit TODO

**Audit Date:** January 18, 2026
**Last Verified:** January 18, 2026
**Overall Score:** 7.8/10 — Production-ready with targeted improvements

---

## ✅ Phase 1 Completed (Jan 18, 2026)

### Grep ReDoS Protection ✅
- [x] Added `validate_pattern()` function with MAX_PATTERN_LENGTH constant
- [x] Rejects patterns > 1000 chars
- [x] Detects nested quantifiers like `(a+)+`, `(a*)*`
- [x] 4 unit tests added

### LSP Health Monitoring ✅
- [x] Added `is_healthy()` to LspClient (uses AtomicBool)
- [x] Added `health_check()` to LspManager
- [x] Added `restart_server()` capability
- [x] Extracted `DIAGNOSTICS_WAIT_MS` constant (150ms)

### SSE Integration Tests ✅
- [x] 28 tests added to sse.rs:
  - `parse_finish_reason` (5 tests)
  - `ToolCallAccumulator` (8 tests)
  - `ServerToolAccumulator` (5 tests)
  - `ThinkingAccumulator` (5 tests)
  - `SseStreamProcessor` (5 tests)

### SSE Performance Optimization ✅
- [x] Replaced `lines().collect()` with peekable iterator (avoids Vec allocation)
- [x] Optimized partial line handling with `std::mem::take()`
- [x] Added `String::with_capacity(256)` in StreamBuffer::new()

---

## ✅ Previously Completed

### Centralized Reasoning Config ✅
- [x] Created `reasoning.rs` with `ReasoningConfig` builder
- [x] Handles Anthropic, OpenAI, DeepSeek formats
- [x] 11 unit tests

### Parser Extraction ✅
- [x] Extracted `AnthropicParser` to `parsers/anthropic.rs`
- [x] Extracted `OpenAIParser` to `parsers/openai.rs`
- [x] `client.rs` reduced from 2381 to 1393 lines

### Path Validation Utility ✅
- [x] Created `tools/path_utils.rs`
- [x] `validate_path()` and `validate_new_path()` functions

### OAuth Removal ✅
- [x] Deleted `auth/oauth.rs` and `auth/token_manager.rs`
- [x] Simplified auth popup to API-key only

---

## ✅ Phase 2 Completed (Jan 18, 2026)

### Use ReasoningConfig in client.rs ✅
- [x] Replaced inline max_tokens calculation with `ReasoningConfig::max_tokens_for_format()`
- [x] Replaced inline reasoning config building with `ReasoningConfig::build()`
- [x] Uses `ReasoningConfig::build_opus_effort()` for Opus 4.5 effort config
- [x] Note: Original audit incorrectly identified transform.rs; actual duplication was in client.rs

### Google API Format for Gemini ✅
- [x] Added `uses_google_format()` helper method to ClientConfig
- [x] Fixed URL endpoint to include `:streamGenerateContent` for streaming
- [x] Added routing for Google format in `call_streaming()`
- [x] Implemented `call_streaming_google()` method
- [x] Implemented `convert_messages_google()` - converts messages to Google contents/parts format
- [x] Implemented `convert_tools_google()` - converts tools to Google function declarations
- [x] Created `parsers/google.rs` with `GoogleParser` for streaming response parsing

---

## 🟡 Phase 2b: Architectural Changes (Requires Planning)

These items are larger architectural changes that should be planned carefully.

### 1. Split App Struct (God Object)
**Location:** `crates/krusty-cli/src/tui/app.rs`
**Problem:** 50+ fields, 1,864 lines
**Impact:** Major refactor affecting entire TUI subsystem
**Recommendation:** Create a detailed plan before implementation

### 2. Complete BlockManager Phase-out
**Location:** `crates/krusty-cli/src/tui/app.rs:226`
**Problem:** "Being phased out in favor of conversation-based rendering"
**Impact:** 12 files use `self.blocks.` - significant migration
**Current Status:** New `block_ui` and `tool_results` structures exist but migration incomplete
**Recommendation:** Plan migration file-by-file, test each change

---

## 🟡 Phase 3: Security Hardening

- [ ] Add file size limits (read.rs, write.rs)
- [ ] Add bounded channels (9 files use unbounded_channel)
- [ ] MCP tool sandboxing
- [ ] Windows file permissions

---

## 📈 Test Coverage

| Area | Status |
|------|--------|
| SSE parsing | ✅ 28 tests |
| reasoning.rs | ✅ 11 tests |
| grep validation | ✅ 4 tests |
| Tool execution | 🔴 No integration tests |
| LSP lifecycle | 🔴 No tests |

---

## Current Build Status

```
cargo test -p krusty-core: 102 tests pass ✅
cargo check: Passes
cargo clippy: Clean
```
