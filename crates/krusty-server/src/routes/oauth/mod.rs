//! OAuth authentication endpoints for web and mobile clients.

mod callback;
mod exchange;
mod start;
mod status;

use std::time::Instant;

use axum::{
    routing::{delete, get, post},
    Router,
};

use krusty_core::ai::providers::ProviderId;

use self::exchange::exchange_code;
use self::start::start_oauth;
use self::status::{oauth_status, revoke_oauth};
use crate::error::AppError;
use crate::AppState;

const FLOW_TTL_SECS: u64 = 300;
const OAUTH_RESULT_STORAGE_KEY: &str = "krusty:oauth-result";
const OAUTH_RESULT_CHANNEL: &str = "krusty:oauth";

/// In-flight OAuth flow state stored on the server.
#[derive(Clone)]
pub struct OAuthFlowState {
    pub started_at: Instant,
    pub provider_id: ProviderId,
    pub kind: OAuthFlowKind,
}

#[derive(Clone)]
pub enum OAuthFlowKind {
    PkceVerifier {
        verifier_str: String,
    },
    BrowserCallback {
        state: String,
        verifier_str: String,
        redirect_uri: String,
    },
    DeviceFlow {
        flow_id: String,
    },
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/start", post(start_oauth))
        .route("/exchange", post(exchange_code))
        .route("/status/:provider", get(oauth_status))
        .route("/revoke/:provider", delete(revoke_oauth))
}

pub fn callback_router() -> Router<AppState> {
    Router::new().route(
        "/auth/oauth/callback/:provider",
        get(callback::oauth_callback),
    )
}

fn parse_provider(input: &str) -> Result<ProviderId, AppError> {
    crate::utils::providers::parse_provider(input)
        .ok_or_else(|| AppError::BadRequest(format!("Unknown provider: {}", input)))
}
#[cfg(test)]
mod tests {
    use super::status::load_oauth_token_presence;
    use crate::error::AppError;
    use krusty_core::ai::providers::ProviderId;
    use krusty_core::auth::{OAuthTokenData, OAuthTokenStore};

    fn sample_token() -> OAuthTokenData {
        OAuthTokenData {
            access_token: "token".to_string(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            last_refresh: 0,
            account_id: None,
        }
    }

    #[test]
    fn load_oauth_token_presence_returns_provider_presence() {
        let mut store = OAuthTokenStore::default();
        store.set(ProviderId::OpenAI, sample_token());

        let has_token = load_oauth_token_presence(ProviderId::OpenAI, || Ok(store))
            .unwrap_or_else(|_| panic!("status helper should succeed"));

        assert!(has_token);
    }

    #[test]
    fn load_oauth_token_presence_returns_error_on_store_failure() {
        let result = load_oauth_token_presence(ProviderId::OpenAI, || {
            Err(anyhow::anyhow!("store unavailable"))
        });

        match result {
            Err(AppError::Internal(message)) => assert!(message.contains("store unavailable")),
            Ok(_) => panic!("broken store should not report token absence"),
            Err(_) => panic!("broken store should surface as internal error"),
        }
    }
}
