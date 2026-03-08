//! API routes

use axum::Router;

use crate::AppState;

mod chat;
mod credentials;
mod files;
mod git;
mod hooks;
mod mcp;
mod models;
pub mod oauth;
mod ports;
mod preview_settings;
mod processes;
mod push;
mod sessions;
mod tools;

/// Build the API router with all endpoints
pub fn api_router() -> Router<AppState> {
    Router::new()
        .nest("/sessions", sessions::router())
        .nest("/chat", chat::router())
        .nest("/models", models::router())
        .nest("/tools", tools::router())
        .nest("/git", git::router())
        .nest("/files", files::router())
        .nest("/credentials", credentials::router())
        .nest("/mcp", mcp::router())
        .nest("/processes", processes::router())
        .nest("/ports", ports::router())
        .nest("/settings/preview", preview_settings::router())
        .nest("/hooks", hooks::router())
        .nest("/push", push::router())
        .nest("/auth/oauth", oauth::router())
        .merge(Router::new())
}
