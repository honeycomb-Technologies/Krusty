//! OAuth authentication endpoints for web and mobile clients.

mod callback;
mod exchange;
mod start;
mod status;

use axum::{
    routing::{delete, get, post},
    Router,
};

use krusty_core::ai::providers::ProviderId;

pub(crate) use crate::oauth_flow::{OAuthFlowKind, OAuthFlowState};

use self::exchange::exchange_code;
use self::start::start_oauth;
use self::status::{oauth_status, revoke_oauth};
use crate::error::AppError;
use crate::AppState;

const FLOW_TTL_SECS: u64 = 300;
const OAUTH_RESULT_STORAGE_KEY: &str = "krusty:oauth-result";
const OAUTH_RESULT_CHANNEL: &str = "krusty:oauth";

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
    use super::status::provider_has_oauth_token;
    use krusty_core::ai::providers::ProviderId;
    use krusty_core::auth::{OAuthTokenData, OAuthTokenStore};
    use krusty_core::storage::CredentialStore;

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
    fn provider_has_oauth_token_returns_provider_presence() {
        let mut store = OAuthTokenStore::default();
        store.set(ProviderId::OpenAI, sample_token());
        let credentials = CredentialStore::default();

        assert!(provider_has_oauth_token(
            ProviderId::OpenAI,
            &store,
            &credentials
        ));
    }

    #[test]
    fn provider_has_oauth_token_returns_false_without_token() {
        let store = OAuthTokenStore::default();
        let credentials = CredentialStore::default();

        assert!(!provider_has_oauth_token(
            ProviderId::OpenAI,
            &store,
            &credentials
        ));
    }
}
