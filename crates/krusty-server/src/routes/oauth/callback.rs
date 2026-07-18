use axum::{
    extract::{Path, Query, State},
    response::{Html, IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;

use krusty_core::ai::providers::ProviderId;
use krusty_core::auth::{
    openai_oauth_config, HostedBrowserOAuthFlow, OAuthTokenStore, PkceVerifier,
};

use super::start::refresh_openai_models;
use super::{
    parse_provider, OAuthFlowKind, FLOW_TTL_SECS, OAUTH_RESULT_CHANNEL, OAUTH_RESULT_STORAGE_KEY,
};
use crate::AppState;

#[derive(Deserialize)]
pub(super) struct OAuthCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

pub(super) async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Response {
    let provider_id = match parse_provider(&provider) {
        Ok(provider_id) => provider_id,
        Err(_) => {
            return callback_error_page(
                provider,
                "Unknown provider for OAuth callback.".to_string(),
            );
        }
    };

    if provider_id != ProviderId::OpenAI {
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "This callback route is not enabled for that provider.".to_string(),
        );
    }

    let flow_state = {
        let flows = state.oauth_flows.lock().await;
        flows.get(provider_id.storage_key()).cloned()
    };

    let Some(flow_state) = flow_state else {
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "No active sign-in is waiting for this callback.".to_string(),
        );
    };

    if flow_state.started_at.elapsed().as_secs() >= FLOW_TTL_SECS {
        state
            .oauth_flows
            .lock()
            .await
            .remove(provider_id.storage_key());
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "This sign-in session expired. Start the login again from Krusty.".to_string(),
        );
    }

    let (expected_state, verifier_str, redirect_uri) = match flow_state.kind {
        OAuthFlowKind::BrowserCallback {
            state,
            verifier_str,
            redirect_uri,
        } => (state, verifier_str, redirect_uri),
        OAuthFlowKind::PkceVerifier { .. } | OAuthFlowKind::DeviceFlow { .. } => {
            return callback_error_page(
                provider_id.storage_key().to_string(),
                "The active sign-in for this provider is not using a browser callback.".to_string(),
            );
        }
    };

    if query.state.as_deref() != Some(expected_state.as_str()) {
        state
            .oauth_flows
            .lock()
            .await
            .remove(provider_id.storage_key());
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "The returned sign-in state did not match the active request.".to_string(),
        );
    }

    if let Some(error_code) = query.error.as_deref() {
        state
            .oauth_flows
            .lock()
            .await
            .remove(provider_id.storage_key());
        let description = query
            .error_description
            .as_deref()
            .unwrap_or("Sign-in was canceled or denied.");
        return callback_error_page(
            provider_id.storage_key().to_string(),
            format!("{} ({})", description, error_code),
        );
    }

    let Some(code) = query.code.as_deref() else {
        state
            .oauth_flows
            .lock()
            .await
            .remove(provider_id.storage_key());
        return callback_error_page(
            provider_id.storage_key().to_string(),
            "The provider callback did not include an authorization code.".to_string(),
        );
    };

    let verifier = PkceVerifier::from_string(verifier_str);
    let flow = HostedBrowserOAuthFlow::new(openai_oauth_config(), redirect_uri);
    let exchange_result = flow.exchange_code(code, &verifier).await;

    state
        .oauth_flows
        .lock()
        .await
        .remove(provider_id.storage_key());

    match exchange_result {
        Ok(token_data) => {
            if let Err(error) = OAuthTokenStore::set_persisted(provider_id, token_data) {
                return callback_error_page(
                    provider_id.storage_key().to_string(),
                    format!("Failed to save your sign-in: {}", error),
                );
            }

            let registry = state.model_registry.clone();
            tokio::spawn(async move {
                refresh_openai_models(registry).await;
            });

            callback_success_page(provider_id.storage_key().to_string())
        }
        Err(error) => callback_error_page(provider_id.storage_key().to_string(), error.to_string()),
    }
}

fn callback_success_page(provider: String) -> Response {
    callback_page(provider, true, "Sign-in complete".to_string(), None)
}

fn callback_error_page(provider: String, error: String) -> Response {
    callback_page(
        provider,
        false,
        "Sign-in did not finish".to_string(),
        Some(error),
    )
}

fn callback_page(
    provider: String,
    success: bool,
    headline: String,
    error: Option<String>,
) -> Response {
    let message = if success {
        "You can return to Krusty now. This tab will close if your browser allows it.".to_string()
    } else {
        error
            .clone()
            .unwrap_or_else(|| "Something went wrong during sign-in.".to_string())
    };
    let payload = json!({
        "type": "krusty-oauth-complete",
        "provider": provider,
        "success": success,
        "error": error,
        "issued_at": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0),
    });
    let payload_json = script_safe_json(
        &serde_json::to_string(&payload).unwrap_or_else(|_| "{\"success\":false}".to_string()),
    );
    let return_url =
        script_safe_json(&serde_json::to_string("/").unwrap_or_else(|_| "\"/\"".to_string()));
    let headline_class = if success { "status ok" } else { "status error" };
    let headline_text = escape_html(&headline);
    let message_text = escape_html(&message);

    let html = format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <meta http-equiv="Cache-Control" content="no-store">
  <title>{headline_text}</title>
  <style>
    :root {{
      color-scheme: light;
      font-family: ui-sans-serif, system-ui, sans-serif;
      background: #f4efe7;
      color: #1f1a17;
    }}
    body {{
      margin: 0;
      min-height: 100vh;
      display: grid;
      place-items: center;
      background:
        radial-gradient(circle at top, rgba(217, 119, 6, 0.18), transparent 35%),
        linear-gradient(180deg, #fbf7f0 0%, #f1ebe0 100%);
    }}
    main {{
      width: min(92vw, 30rem);
      padding: 1.5rem;
      border-radius: 1.5rem;
      background: rgba(255, 255, 255, 0.9);
      box-shadow: 0 20px 60px rgba(61, 42, 24, 0.16);
      border: 1px solid rgba(95, 72, 53, 0.12);
    }}
    h1 {{
      margin: 0 0 0.75rem;
      font-size: 1.5rem;
      line-height: 1.2;
    }}
    p {{
      margin: 0;
      color: #5c4a3b;
      line-height: 1.5;
    }}
    .status {{
      display: inline-flex;
      align-items: center;
      gap: 0.5rem;
      margin-bottom: 1rem;
      font-size: 0.8rem;
      font-weight: 700;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }}
    .ok {{
      color: #166534;
    }}
    .error {{
      color: #b42318;
    }}
    a {{
      display: inline-flex;
      margin-top: 1.25rem;
      padding: 0.75rem 1rem;
      border-radius: 999px;
      text-decoration: none;
      color: white;
      background: #111827;
    }}
  </style>
</head>
<body>
  <main>
    <div class="{headline_class}">{provider}</div>
    <h1>{headline_text}</h1>
    <p>{message_text}</p>
    <a href="/">Return to Krusty</a>
  </main>
  <script>
    const payload = {payload_json};
    const returnUrl = {return_url};

    try {{
      localStorage.setItem({storage_key:?}, JSON.stringify(payload));
    }} catch (error) {{
      console.debug('oauth localStorage signal failed', error);
    }}

    try {{
      if (window.opener && !window.opener.closed) {{
        window.opener.postMessage(payload, window.location.origin);
        window.opener.focus();
      }}
    }} catch (error) {{
      console.debug('oauth opener signal failed', error);
    }}

    try {{
      if ('BroadcastChannel' in window) {{
        const channel = new BroadcastChannel({channel_name:?});
        channel.postMessage(payload);
        channel.close();
      }}
    }} catch (error) {{
      console.debug('oauth broadcast signal failed', error);
    }}

    setTimeout(() => window.close(), 150);
    setTimeout(() => {{
      if (window.location.pathname !== returnUrl) {{
        window.location.replace(returnUrl);
      }}
    }}, 1200);
  </script>
</body>
</html>"#,
        provider = escape_html(&provider),
        headline_class = headline_class,
        headline_text = headline_text,
        message_text = message_text,
        payload_json = payload_json,
        return_url = return_url,
        storage_key = OAUTH_RESULT_STORAGE_KEY,
        channel_name = OAUTH_RESULT_CHANNEL,
    );

    Html(html).into_response()
}

fn script_safe_json(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use axum::body;

    use super::*;

    #[test]
    fn script_safe_json_escapes_html_script_breakouts() {
        let input = r#"{"error":"</script><script>alert(1)</script>&\u2028\u2029"}"#;
        let escaped = script_safe_json(input);

        assert!(!escaped.contains("</script>"));
        assert!(!escaped.contains("<script>"));
        assert!(escaped.contains(r#"\u003c/script\u003e"#));
        assert!(escaped.contains(r#"\u003cscript\u003e"#));
        assert!(escaped.contains(r#"\u0026"#));
    }

    #[tokio::test]
    async fn callback_page_does_not_embed_raw_script_end_tag_in_payload() -> anyhow::Result<()> {
        let response = callback_error_page(
            "openai".to_string(),
            "</script><script>window.__krusty_xss=1</script> (access_denied)".to_string(),
        );
        let bytes = body::to_bytes(response.into_body(), usize::MAX).await?;
        let html = String::from_utf8(bytes.to_vec())?;

        assert!(html.contains("&lt;/script&gt;&lt;script&gt;window.__krusty_xss=1&lt;/script&gt;"));
        assert!(html.contains(
            r#"\u003c/script\u003e\u003cscript\u003ewindow.__krusty_xss=1\u003c/script\u003e"#
        ));
        assert!(!html.contains("const payload = {\"error\":\"</script>"));
        Ok(())
    }
}
