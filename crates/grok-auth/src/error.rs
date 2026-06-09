use thiserror::Error;

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("no credentials available and no login method succeeded")]
    NoCredentials,

    #[error("token expired and refresh failed: {0}")]
    RefreshFailed(String),

    #[error("OIDC discovery failed for issuer {issuer}: {msg}")]
    DiscoveryFailed { issuer: String, msg: String },

    #[error("PKCE code exchange failed: {0}")]
    CodeExchangeFailed(String),

    #[error("external auth provider failed: {0}")]
    ExternalProvider(String),

    #[error("device code flow failed: {0}")]
    DeviceCode(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("HTTP error: {0}")]
    Http(String),

    #[error("JSON error: {0}")]
    Json(String),

    #[error("URL parse error: {0}")]
    Url(String),

    #[error("invalid header value: {0}")]
    InvalidHeader(String),

    #[error("lock error on auth file: {0}")]
    Lock(String),

    #[error("callback server error: {0}")]
    CallbackServer(String),

    #[error("browser open failed: {0}")]
    Browser(String),

    #[error("configuration error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, AuthError>;

// Manual conversions to keep thiserror happy
impl From<std::io::Error> for AuthError {
    fn from(e: std::io::Error) -> Self {
        AuthError::Io(e.to_string())
    }
}
impl From<reqwest::Error> for AuthError {
    fn from(e: reqwest::Error) -> Self {
        AuthError::Http(e.to_string())
    }
}
impl From<serde_json::Error> for AuthError {
    fn from(e: serde_json::Error) -> Self {
        AuthError::Json(e.to_string())
    }
}
impl From<url::ParseError> for AuthError {
    fn from(e: url::ParseError) -> Self {
        AuthError::Url(e.to_string())
    }
}
impl From<reqwest::header::InvalidHeaderValue> for AuthError {
    fn from(e: reqwest::header::InvalidHeaderValue) -> Self {
        AuthError::InvalidHeader(e.to_string())
    }
}
