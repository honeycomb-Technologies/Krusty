# Krusty Development Guide

## Philosophy

Krusty aims to be best-in-class: elegant, performant, organized, modular, and idiomatic Rust. Every component should feel polished and professional.

## Code Style

- **Minimal comments** - Code should be self-documenting. Comments only for complex logic or non-obvious decisions.
- **Idiomatic Rust** - Follow Rust conventions. Use iterators over loops, Option/Result over nulls/exceptions.
- **No over-engineering** - YAGNI principle. Don't add abstractions until needed.
- **Avoid unsafe** - No unsafe code unless absolutely necessary with clear justification.
- **Minimize dependencies** - Prefer stdlib, only add deps when they provide clear value.

## Error Handling

Use `anyhow::Result` everywhere with `.context()` for debugging:
```rust
let file = std::fs::read_to_string(&path)
    .context("Failed to read config file")?;
```

## Architecture

- **Trait-based extensibility** - `Tool`, `PreToolHook`, `PostToolHook` patterns
- **Arc/RwLock for shared state** - Thread-safe by default
- **Async-first** - tokio runtime, `#[async_trait]`
- **Tracing for logging** - Debug-friendly spans and events

## Making Changes

Be **proactive**: refactor when beneficial, modernize patterns, optimize freely. Don't just fix the immediate issue - improve the surrounding code if it makes sense.

## Testing

Tests on request only. When adding tests:
- Inline tests: `#[cfg(test)] mod tests { }` at file end
- Use descriptive test names that explain the scenario

## Branching

- **`main`** - Stable, release-ready code only. Merges from `dev` via PR when releasing.
- **`dev`** - Day-to-day development. All work happens here (or in feature branches merged into `dev`).
- Always work on `dev`. Never commit directly to `main`.

## Before Committing

All four checks MUST pass before committing, pushing, or creating a release. If any check fails, fix the issue and re-run ALL checks until they all pass. Never push or release with failing checks.

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo build --workspace
cargo test --workspace
```

Use conventional commits:
- `feat:` - New features
- `fix:` - Bug fixes
- `refactor:` - Code changes that don't add features or fix bugs
- `chore:` - Maintenance tasks
- `docs:` - Documentation only

## Project Structure

- `crates/krusty-cli/` - CLI application with TUI
- `crates/krusty-core/` - Core library (AI providers, tools, ACP, storage)

## Building & Testing

```bash
cargo build --workspace
cargo test --workspace
```

## ACP Mode

Editor integration via Agent Client Protocol:
```bash
krusty acp
```

Environment variables:
- `KRUSTY_PROVIDER` - Provider name (anthropic, openrouter, opencodezen, zai, minimax, kimi)
- `KRUSTY_API_KEY` - API key for the provider
- `KRUSTY_MODEL` - Optional model override
