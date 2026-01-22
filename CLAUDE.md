# Krusty Development Guide

## Before Committing

Always run these checks before committing changes:

```bash
# Format all code
cargo fmt --all

# Check for clippy warnings (must pass with no warnings)
cargo clippy --workspace -- -D warnings
```

Both commands must pass without errors for CI to succeed.

## Project Structure

- `crates/krusty-cli/` - CLI application with TUI
- `crates/krusty-core/` - Core library (AI providers, tools, ACP, storage)
- `krusty-zed-extension/` - Zed editor extension

## Building

```bash
cargo build --workspace
```

## Testing

```bash
cargo test --workspace
```

## ACP Mode

Krusty supports the Agent Client Protocol (ACP) for editor integrations:

```bash
krusty --acp
```

Environment variables for ACP:
- `KRUSTY_PROVIDER` - Provider name (anthropic, openrouter, opencodezen, zai, minimax, kimi)
- `KRUSTY_API_KEY` - API key for the provider
- `KRUSTY_MODEL` - Optional model override
- Or use provider-specific keys: `ANTHROPIC_API_KEY`, `OPENROUTER_API_KEY`, etc.
