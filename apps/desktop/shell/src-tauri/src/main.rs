#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use krusty_core::server_instance;
use tauri::Manager;

const DEFAULT_PORT: u16 = 3000;

fn main() {
    apply_linux_webkit_workarounds();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let port = rt.block_on(ensure_server_running());

    tauri::Builder::default()
        .setup(move |app| {
            if let Some(window) = app.webview_windows().values().next() {
                // Inject server URL so the Expo web app can auto-connect
                let js = format!(
                    "window.__KRUSTY_SERVER_URL='http://localhost:{}';window.__KRUSTY_SERVER_TOKEN='local';",
                    port
                );
                let _ = window.eval(&js);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Mitsuro desktop shell");
}

/// Ensure a Mitsuro server is running — reuse existing or start a new one.
async fn ensure_server_running() -> u16 {
    // Check for already-running server
    if let Some(instance) = server_instance::detect_running_server().await {
        tracing::info!(
            "Reusing existing Mitsuro server on port {} (PID {})",
            instance.port,
            instance.pid
        );
        return instance.port;
    }

    // No server running — start one in the background
    let port = choose_server_port(DEFAULT_PORT);
    if port != DEFAULT_PORT {
        tracing::warn!(
            "Default port {} is unavailable; starting embedded server on fallback port {}",
            DEFAULT_PORT,
            port
        );
    }
    tracing::info!("Starting embedded Mitsuro server on port {}", port);

    let config = krusty_server::ServerConfig {
        port,
        ..Default::default()
    };

    // Write PID file before spawning
    if let Err(e) = server_instance::write_pid_file(port) {
        tracing::warn!("Failed to write PID file: {}", e);
    }

    tokio::spawn(async move {
        if let Err(e) = krusty_server::start_server(config).await {
            tracing::error!("Embedded server failed: {}", e);
            server_instance::remove_pid_file();
        }
    });

    // Wait for server to become healthy
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if server_instance::probe_health(port).await {
            tracing::info!("Embedded server is ready");
            return port;
        }
    }

    tracing::warn!("Server health check timed out, proceeding anyway");
    port
}

#[cfg(target_os = "linux")]
fn apply_linux_webkit_workarounds() {
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
}

#[cfg(not(target_os = "linux"))]
fn apply_linux_webkit_workarounds() {}

fn choose_server_port(preferred: u16) -> u16 {
    if std::net::TcpListener::bind(("127.0.0.1", preferred)).is_ok() {
        return preferred;
    }

    match std::net::TcpListener::bind(("127.0.0.1", 0)) {
        Ok(listener) => listener
            .local_addr()
            .map(|addr| addr.port())
            .unwrap_or(preferred),
        Err(_) => preferred,
    }
}
