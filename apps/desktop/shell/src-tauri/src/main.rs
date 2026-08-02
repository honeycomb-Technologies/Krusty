#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop_identity_compatibility;

use mitsuro_core::server_instance;
use tauri::Manager;

const DEFAULT_PORT: u16 = 3000;
const OFFLINE_IDENTITY_MIGRATION_COMMAND: &str = "mitsuro migrate-identity --confirm-offline";

fn main() {
    apply_linux_webkit_workarounds();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    let startup_identity = mitsuro_core::identity::require_startup_identity().and_then(|state| {
        embedded_server_identity_decision(state)
            .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidData, message))
    });
    if let Err(error) = startup_identity {
        tracing::error!("Mitsuro desktop cannot establish runtime state authority: {error}");
        eprintln!("Mitsuro desktop cannot establish runtime state authority: {error}");
        std::process::exit(1);
    }

    match desktop_identity_compatibility::migrate_legacy_desktop_data() {
        Ok(desktop_identity_compatibility::DesktopDataMigration::Imported { from, to }) => {
            tracing::info!(
                "Imported prior desktop web data from {} to {}; prior data was preserved",
                from.display(),
                to.display()
            );
            tracing::warn!(
                "Rollback desktop data is recovery-only. After cutover, do not launch a previous-generation desktop app except through the coordinated rollback procedure. No continuous lock can stop a directly invoked archived app; running one can create split authority or invalidate the next Mitsuro start."
            );
        }
        Ok(_) => {}
        Err(error) => {
            tracing::error!("Mitsuro desktop cannot establish desktop web-data authority: {error}");
            eprintln!("Mitsuro desktop cannot establish desktop web-data authority: {error}");
            std::process::exit(1);
        }
    }

    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let port = match rt.block_on(ensure_server_running()) {
        Ok(port) => port,
        Err(error) => {
            tracing::error!("Mitsuro desktop cannot start: {error}");
            eprintln!("Mitsuro desktop cannot start: {error}");
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .setup(move |app| {
            if let Some(window) = app.webview_windows().values().next() {
                // Inject server URL so the Expo web app can auto-connect
                let js = desktop_identity_compatibility::injected_connection_globals(port);
                let _ = window.eval(&js);
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Mitsuro desktop shell");
}

/// Ensure a Mitsuro server is running — reuse existing or start a new one.
async fn ensure_server_running() -> std::io::Result<u16> {
    // Establish the state authority before reusing or spawning any server. A
    // healthy canonical PID must not bypass detection of a concurrently live
    // previous generation after an identity migration.
    let discovery = mitsuro_core::identity::require_startup_identity()?;

    // Check for already-running server
    if let Some(instance) = server_instance::detect_running_server().await {
        tracing::info!(
            "Reusing existing Mitsuro server on port {} (PID {})",
            instance.port,
            instance.pid
        );
        return Ok(instance.port);
    }

    // Starting the embedded server against an unresolved old state would make
    // it fail in the background and leave the shell waiting on an API that can
    // never become healthy. Refuse before spawning, without moving any data.
    if let Err(message) = embedded_server_identity_decision(discovery) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message,
        ));
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

    let config = mitsuro_server::ServerConfig {
        port,
        ..Default::default()
    };

    // Write PID file before spawning
    if let Err(e) = server_instance::write_pid_file(port) {
        tracing::warn!("Failed to write PID file: {}", e);
    }

    tokio::spawn(async move {
        if let Err(e) = mitsuro_server::start_server(config).await {
            tracing::error!("Embedded server failed: {}", e);
            remove_embedded_pid_file_if_owned(port);
        }
    });

    // Wait for server to become healthy
    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        if server_instance::probe_health(port).await {
            tracing::info!("Embedded server is ready");
            return Ok(port);
        }
    }

    remove_embedded_pid_file_if_owned(port);
    Err(embedded_server_timeout_error(port))
}

fn embedded_server_timeout_error(port: u16) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        format!("embedded Mitsuro server on port {port} did not become healthy within 5 seconds"),
    )
}

fn remove_embedded_pid_file_if_owned(port: u16) {
    let _ = server_instance::remove_pid_file_if_matches(std::process::id(), port);
}

fn embedded_server_identity_decision(
    discovery: mitsuro_core::identity::ConfigDiscovery,
) -> Result<(), String> {
    use mitsuro_core::identity::ConfigDiscovery;

    match discovery {
        ConfigDiscovery::LegacyOnly => Err(format!(
            "only legacy Mitsuro state was found; stop every Mitsuro/Hive process, take a backup, run `{OFFLINE_IDENTITY_MIGRATION_COMMAND}`, then restart Mitsuro desktop"
        )),
        ConfigDiscovery::UnreconciledCoexistence => Err(format!(
            "canonical and legacy Mitsuro roots coexist without a migration receipt; back up and reconcile the roots first, then complete `{OFFLINE_IDENTITY_MIGRATION_COMMAND}` before restarting Mitsuro desktop"
        )),
        ConfigDiscovery::Empty
        | ConfigDiscovery::CanonicalOnly
        | ConfigDiscovery::MigratedWithRollback => Ok(()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use mitsuro_core::identity::ConfigDiscovery;

    #[test]
    fn embedded_server_refuses_unmigrated_identity_before_spawn() {
        for discovery in [
            ConfigDiscovery::LegacyOnly,
            ConfigDiscovery::UnreconciledCoexistence,
        ] {
            let error = embedded_server_identity_decision(discovery).unwrap_err();
            assert!(error.contains(OFFLINE_IDENTITY_MIGRATION_COMMAND));
        }

        for discovery in [
            ConfigDiscovery::Empty,
            ConfigDiscovery::CanonicalOnly,
            ConfigDiscovery::MigratedWithRollback,
        ] {
            assert_eq!(embedded_server_identity_decision(discovery), Ok(()));
        }
    }

    #[test]
    fn embedded_server_health_timeout_is_terminal() {
        let error = embedded_server_timeout_error(4317);
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(error.to_string().contains("4317"));
        assert!(error.to_string().contains("did not become healthy"));
    }
}
