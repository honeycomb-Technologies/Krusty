use std::time::Instant;

use mitsuro_core::ai::providers::ProviderId;

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
