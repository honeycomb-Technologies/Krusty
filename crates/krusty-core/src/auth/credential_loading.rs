use anyhow::Result;
use base64::Engine;

use super::{refresh::try_refresh_oauth_token_blocking, OAuthTokenStore};
use crate::ai::providers::ProviderId;

/// Check if a credential is an Anthropic OAuth token
pub fn is_anthropic_oauth_token(key: &str) -> bool {
    key.starts_with("sk-ant-oat")
}

/// Extract ChatGPT account id from OpenAI JWT-like tokens.
///
/// Expected claim path:
/// `https://api.openai.com/auth.chatgpt_account_id`
pub fn extract_openai_account_id(token: &str) -> Option<String> {
    let payload = decode_jwt_payload(token)?;
    let auth_obj = payload.get("https://api.openai.com/auth")?;
    let account_id = auth_obj.get("chatgpt_account_id")?.as_str()?;
    if account_id.is_empty() {
        None
    } else {
        Some(account_id.to_string())
    }
}

fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let _signature = parts.next()?;

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| {
            let mut padded = payload_b64.to_string();
            while padded.len() % 4 != 0 {
                padded.push('=');
            }
            base64::engine::general_purpose::URL_SAFE.decode(padded)
        })
        .ok()?;

    serde_json::from_slice::<serde_json::Value>(&decoded).ok()
}

pub(crate) fn load_oauth_store_or_none(
    context: &'static str,
    load: impl FnOnce() -> Result<OAuthTokenStore>,
) -> Option<OAuthTokenStore> {
    match load() {
        Ok(store) => Some(store),
        Err(error) => {
            tracing::warn!(context, error = %error, "Failed to load OAuth token store");
            None
        }
    }
}

fn load_optional_oauth_store(context: &'static str) -> Option<OAuthTokenStore> {
    load_oauth_store_or_none(context, OAuthTokenStore::load)
}

pub(crate) fn load_openai_oauth_credential() -> Option<(String, Option<String>)> {
    let oauth_store = load_optional_oauth_store("loading OpenAI OAuth credential")?;
    let token = oauth_store.get(&ProviderId::OpenAI)?;

    if token.is_expired() {
        if token.refresh_token.is_some() {
            let refreshed = try_refresh_oauth_token_blocking(ProviderId::OpenAI)?;
            let account_id = refreshed
                .account_id
                .clone()
                .or_else(|| extract_openai_account_id(&refreshed.access_token))
                .or_else(|| {
                    refreshed
                        .id_token
                        .as_deref()
                        .and_then(extract_openai_account_id)
                });
            return Some((refreshed.access_token, account_id));
        }
        return None;
    }

    let account_id = token
        .account_id
        .clone()
        .or_else(|| extract_openai_account_id(&token.access_token))
        .or_else(|| {
            token
                .id_token
                .as_deref()
                .and_then(extract_openai_account_id)
        });
    Some((token.access_token.clone(), account_id))
}

pub(crate) fn load_anthropic_oauth_credential() -> Option<(String, Option<String>)> {
    let oauth_store = load_optional_oauth_store("loading Anthropic OAuth credential")?;
    let token = oauth_store.get(&ProviderId::Anthropic)?;

    if token.is_expired() {
        if token.refresh_token.is_some() {
            let refreshed = try_refresh_oauth_token_blocking(ProviderId::Anthropic)?;
            return Some((refreshed.access_token, refreshed.account_id));
        }
        return None;
    }

    Some((token.access_token.clone(), token.account_id.clone()))
}

#[cfg(test)]
mod tests {
    use super::load_oauth_store_or_none;
    use crate::ai::providers::ProviderId;
    use crate::auth::OAuthTokenStore;

    #[test]
    fn load_oauth_store_or_none_returns_store_on_success() {
        let store = load_oauth_store_or_none("test", || Ok(OAuthTokenStore::default()));

        assert!(store.is_some());
        assert!(!store
            .as_ref()
            .is_some_and(|loaded| loaded.has_token(&ProviderId::OpenAI)));
    }

    #[test]
    fn load_oauth_store_or_none_returns_none_on_failure() {
        let store = load_oauth_store_or_none("test", || Err(anyhow::anyhow!("boom")));

        assert!(store.is_none());
    }
}
