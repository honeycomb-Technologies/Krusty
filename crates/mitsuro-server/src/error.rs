//! Unified error handling for the API

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

/// API error response body
#[derive(Serialize)]
pub struct ApiError {
    pub error: String,
    pub code: String,
}

/// Application error types
#[derive(Debug)]
pub enum AppError {
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    ServiceUnavailable(String),
    BadGateway(String),
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "NOT_FOUND", msg),
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, "BAD_REQUEST", msg),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "CONFLICT", msg),
            AppError::ServiceUnavailable(msg) => {
                (StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE", msg)
            }
            AppError::BadGateway(msg) => (StatusCode::BAD_GATEWAY, "BAD_GATEWAY", msg),
            AppError::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", msg),
        };

        (
            status,
            Json(ApiError {
                error: message,
                code: code.to_string(),
            }),
        )
            .into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        // Log full error chain for debugging, return sanitized message to client
        tracing::error!("Internal error: {:?}", err);
        AppError::Internal(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::BadRequest(format!("JSON error: {}", err))
    }
}
