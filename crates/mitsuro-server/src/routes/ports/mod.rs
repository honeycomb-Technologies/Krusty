//! Preview / port-forwarding endpoints.

mod discovery;
mod probe;
mod proxy;

use axum::{
    routing::{any, get},
    Router,
};

use self::discovery::list_ports;
use self::proxy::{proxy_path, proxy_root};
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_ports))
        .route("/:port/proxy", any(proxy_root))
        .route("/:port/proxy/", any(proxy_root))
        .route("/:port/proxy/*path", any(proxy_path))
}
