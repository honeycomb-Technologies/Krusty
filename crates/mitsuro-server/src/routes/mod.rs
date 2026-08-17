//! API routes

use axum::{middleware, Router};

use crate::{auth, AppState};

mod apns;
pub(crate) mod browser;
pub(crate) mod chat;
mod credentials;
mod extensions;
mod files;
mod git;
mod hive;
mod hooks;
mod mcp;
mod memories;
mod mobile_diagnostics;
mod models;
pub mod oauth;
pub(crate) mod plugins;
mod ports;
mod preview_settings;
mod processes;
mod push;
mod reports;
mod server;
mod session_access;
mod sessions;
mod skills;
mod tools;

/// Build the API router with all endpoints
pub fn api_router() -> Router<AppState> {
    let shared_extensibility = Router::new()
        .nest("/extensions", extensions::router())
        .nest("/mcp", mcp::router())
        .nest("/plugins", plugins::router())
        .nest("/skills", skills::router())
        .route_layer(middleware::from_fn(
            auth::shared_extensibility_admin_middleware,
        ));

    Router::new()
        .nest("/browser", browser::router())
        .nest("/sessions", sessions::router())
        .nest("/chat", chat::router())
        .nest("/models", models::router())
        .nest("/tools", tools::router())
        .nest("/git", git::router())
        .nest("/files", files::router())
        .nest("/credentials", credentials::router())
        .nest("/hive", hive::router())
        .nest(
            crate::legacy_identity::HIVE_API_PREFIX,
            hive::legacy_router(),
        )
        .nest("/memories", memories::router())
        .nest("/mobile-diagnostics", mobile_diagnostics::router())
        .nest("/processes", processes::router())
        .nest("/ports", ports::router())
        .nest("/settings/preview", preview_settings::router())
        .nest("/hooks", hooks::router())
        .nest("/push", push::router())
        .nest("/apns", apns::router())
        .nest("/reports", reports::router())
        .nest("/server", server::router())
        .nest("/auth/oauth", oauth::router())
        .merge(shared_extensibility)
}

/// Minimal HTTP surface for disposable provider/orchestration evaluation.
///
/// Global credentials are intentionally resolved by `AppState`, but mutation
/// surfaces for credentials, plugins, hooks, MCP, push, Hive, remote access,
/// and server settings are absent from this router.
pub(crate) fn evaluation_api_router() -> Router<AppState> {
    Router::new()
        .nest("/sessions", sessions::router())
        .nest("/chat", chat::router())
        .nest("/models", models::router())
        .nest("/tools", tools::router())
        .nest("/processes", processes::router())
}
