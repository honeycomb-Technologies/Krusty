//! Example of using the modular grok-auth library from another Rust harness (Krusty).
//!
//! Compile with: cargo run --example krusty_auth

use grok_auth::{authenticated_client, AuthConfig};
use std::env;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    // 1. Load config the same way the official grok CLI does (env + your own toml)
    let mut cfg = AuthConfig::from_env()?;

    // Optional: merge settings from your own Krusty config.toml
    if let Ok(toml) = env::var("KRUSTY_GROK_CONFIG") {
        let _ = cfg.merge_toml(&toml);
    }

    // 2. Get a fully authenticated client (handles load, refresh, re-login if needed)
    println!("Obtaining Grok credentials (will share ~/.grok/auth.json with the official CLI)...");
    let client = authenticated_client(cfg.clone()).await?;

    println!(
        "Got token for issuer: {:?}",
        client.current_token().map(|t| &t.issuer_key)
    );
    println!("Client version header we will send: {}", cfg.client_version);

    // 3. Example: call a Grok model endpoint.
    // The real base URL the TUI uses is often https://cli-chat-proxy.grok.com/v1
    // or whatever you have configured. For public xAI API use the normal endpoint.
    let base = cfg
        .chat_proxy_base_url
        .unwrap_or_else(|| "https://cli-chat-proxy.grok.com/v1".to_string());

    let model = "grok-build";
    let body = serde_json::json!({
        "model": model,
        "input": [{"role": "user", "content": "Say hello from Krusty via modular grok-auth!"}],
        "max_output_tokens": 50,
    });

    let endpoint_base = base
        .trim_end_matches('/')
        .trim_end_matches("/chat/completions")
        .trim_end_matches("/responses");
    let resp = client
        .inner()
        .post(format!("{endpoint_base}/responses"))
        .header("X-XAI-Token-Auth", "xai-grok-cli")
        .header("x-grok-model-override", model)
        .json(&body)
        .send()
        .await?;

    println!("Response status: {}", resp.status());
    let text = resp.text().await?;
    println!("Body (truncated): {}", &text[..text.len().min(800)]);

    // 4. For full "Grok Build" experience (tools, subagents, plan mode, apply_patch, etc.)
    // you would need to speak the higher-level agent protocol the TUI uses
    // (ACP over WebSocket + the generated tool schemas).
    // The auth library gives you the hard part (valid Bearer + correct client headers).
    // You can then layer your own agent loop or reuse pieces from the open parts of the protocol.

    Ok(())
}
