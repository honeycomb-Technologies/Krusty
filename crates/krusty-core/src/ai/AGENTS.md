# AGENTS Guide: /crates/krusty-core/src/ai

## Purpose
AI provider orchestration, parsing, streaming normalization, and retries.

## Guardrails
- Keep provider-specific quirks isolated from shared response models.
- Keep model-family prompt behavior in shared profiles; streaming and simple/conversation calls must build the same instruction layers.
- Keep provider request/stream normalization in the shared AI transform layer; avoid scattering provider patches across individual transport call-sites.
- Streaming behavior must be robust to partial/malformed provider events.
- Parser changes must preserve existing tool/thinking/message semantics.

## Validation
- `cargo check -p krusty-core`
- run targeted parser/streaming tests when touching parsers or streaming code.
