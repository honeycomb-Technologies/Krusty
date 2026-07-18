//! API routes

use axum::{middleware, Router};

use crate::{auth, AppState};

mod apns;
mod chat;
mod credentials;
mod extensions;
mod files;
mod git;
mod hooks;
mod mako;
mod mcp;
mod memories;
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
        .nest("/sessions", sessions::router())
        .nest("/chat", chat::router())
        .nest("/models", models::router())
        .nest("/tools", tools::router())
        .nest("/git", git::router())
        .nest("/files", files::router())
        .nest("/credentials", credentials::router())
        .nest("/mako", mako::router())
        .nest("/memories", memories::router())
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
