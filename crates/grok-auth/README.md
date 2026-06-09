# grok-auth — Modular xAI / Grok authentication for other Rust harnesses

This crate extracts the authentication ("X sub login") subsystem from the Grok Build CLI into a reusable, well-behaved Rust library.

Goal: let projects like **Krusty** (and any other harness) obtain valid Grok/xAI credentials, share the login state with the official `grok` CLI, and make authenticated calls to Grok models / composer / agentic endpoints **without** running inside the Grok Build TUI.

## Features
- All login flows the official client supports:
  - Browser OIDC (Authorization Code + PKCE)
  - Device code (`grok login --device-auth`)
  - External auth provider (the most powerful one for other harnesses — your script/binary prints the token on stdout)
  - Direct `XAI_API_KEY`
- **First-class caching & concurrency safety** (see below)
- Exact `~/.grok/auth.json` format → your harness and `grok` CLI can log in once and share the session.
- Ready-to-use `reqwest::Client` that injects:
  - `Authorization: Bearer ...`
  - `x-grok-client-version`
  - Proper User-Agent and other identification headers the backend cares about.
- Proactive token refresh with configurable buffer (matches `GROK_AUTH_EARLY_INVALIDATION_SECS`).

## Caching Strategy (read this)

The official Grok client is very careful because multiple things can touch `auth.json` at the same time (the TUI, subagents, `grok` commands, your own tools).

This library mirrors that discipline:

1. **Two-level cache**
   - Hot in-memory token (protected by `tokio::sync::RwLock`).
   - On-disk `auth.json` is the source of truth for cross-process sharing.

2. **Proactive refresh**
   - Before returning a token we check `expires_at - early_invalidation_buffer` (default 5 minutes).
   - If close, we do a refresh using the `refresh_token` (standard OIDC) and atomically update the file.

3. **Locking**
   - Short advisory exclusive/shared locks via `fs2` around every read/write of `auth.json`.
   - Prevents the "two processes wrote at the same time and one lost its refresh_token" problem.

4. **Atomic writes**
   - Write to `auth.json.tmp`, `fsync`, then `rename`. POSIX atomic replace.

5. **In-process safety**
   - Only one refresh happens at a time even if 50 tasks call `ensure_fresh()` simultaneously.

You get a valid token (or a clear error) with almost zero disk traffic on the hot path.

## Usage in Krusty (or any other harness)

```toml
# Cargo.toml
[dependencies]
grok-auth = { path = "../grok-auth" }   # or git = "..."
```

```rust
use grok_auth::{AuthConfig, authenticated_client};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cfg = AuthConfig::from_env()?;
    // optionally merge your own config.toml snippet
    // cfg.merge_toml(&std::fs::read_to_string("krusty-grok.toml")?)?;

    let client = authenticated_client(cfg).await?;

    // Now use client.inner() for any HTTP call to the Grok backend.
    // The Authorization header and x-grok-client-* headers are already set.
    Ok(())
}
```

Run `cargo run --example krusty_auth` after filling a real `XAI_API_KEY` or doing a login.

### Sharing login state with the official `grok` CLI

Just use the default `~/.grok/auth.json` path (the library does this by default).
After `grok login` (or `grok-auth` login), both sides see the same tokens and profile info.

### External provider (great for Krusty)

If you already have a fancy login flow in Krusty (corporate SSO, hardware key, etc.), implement the documented contract:

```bash
export GROK_AUTH_PROVIDER_COMMAND="/usr/local/bin/krusty-auth-provider"
export GROK_AUTH_PROVIDER_LABEL="Krusty Corp"
```

Your binary prints the token (bare or `{"access_token": "...", "refresh_token": "...", "expires_in": 3600}`) on **stdout** and human messages / login URLs on **stderr**.

The library will call it again with `GROK_AUTH_EXPIRED=1` when it wants a proactive refresh.

This is the cleanest way to integrate custom auth while still getting the full Grok model surface.

## Using Grok Build / "composer" models outside the TUI

The auth library solves the hard/tedious part (tokens, refresh, headers, sharing with the official CLI).

What you get after that:
- A valid `Bearer` token + correct `x-grok-client-version` etc.
- The ability to call the same CLI proxy endpoints the TUI uses, including `/v1/models` and `/v1/responses` with `X-XAI-Token-Auth: xai-grok-cli` and `x-grok-model-override: grok-build`.

**Limitations today** (as of the reverse engineering):
- The full "Grok Build" experience (subagents, `todo_write`, `enter_plan_mode`, `search_replace` with apply-patch, computer use, image/video gen tools, the whole effects + generated tool schema system, ACP over WebSocket, etc.) lives in the TUI runtime.
- You can still do very powerful agentic work by:
  1. Using the raw model with tool calling (if the backend exposes the same schemas).
  2. Implementing a lightweight version of the tool registry + loop on top of this auth client.
  3. Using the external-provider hook + the same `auth.json` so the official `grok` and Krusty can even hand off sessions in the future.

If you want to go deeper (reverse more of the WS ACP protocol, the generated tool events, the exact chat proxy shape), we can continue the RE on the binary and extend this library.

## Environment variables (same as official grok where possible)

- `XAI_API_KEY`
- `GROK_OIDC_ISSUER`, `GROK_OIDC_CLIENT_ID`
- `GROK_AUTH_PROVIDER_COMMAND`, `GROK_AUTH_PROVIDER_LABEL`
- `GROK_AUTH_EARLY_INVALIDATION_SECS`
- `GROK_CLI_CHAT_PROXY_BASE_URL`
- `GROK_CLIENT_VERSION` (set this to identify your harness)

## Building / Running the example

```bash
cd grok-auth
cargo run --example krusty_auth
```

## Status & future

This is a clean-room reimplementation based on:
- The shipped `02-authentication.md`
- Reverse engineering of the 0.2.33 binary (strings, AuthManager paths, external provider contract, auth.json format, refresh watcher, etc.)
- Local `~/.grok/auth.json` and config inspection

It should be good enough for real use in Krusty today for model access and simple agent loops.

Missing (but easy to add later):
- Full WebSocket ACP client for the richest "Grok Build" session experience.
- More of the generated tool schema / event types (we have them from the binary strings).

Contributions / PRs for Krusty integration welcome.

## License

MIT or Apache-2.0 at your option (same spirit as most of the Rust ecosystem pieces used here).
