//! Krusty - The most elegant coding CLI to ever exist
//!
//! A terminal-based AI coding assistant with:
//! - Zed's LSP extension ecosystem for 100+ language servers
//! - Multi-provider AI with API key authentication
//! - Single-mode Chat UI with slash commands
//! - Clean architecture from day one

use anyhow::Result;
use clap::{Parser, Subcommand};

// Re-export core modules for TUI usage
use krusty_core::{
    acp, agent, ai, constants, extensions, lsp, paths, plan, process, storage, tools,
};

mod tui;

use extensions::WasmHost;

/// Krusty - AI Coding Assistant
#[derive(Parser)]
#[command(name = "krusty")]
#[command(about = "The most elegant coding CLI to ever exist", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Working directory (defaults to current)
    #[arg(short, long)]
    directory: Option<String>,

    /// Theme name
    #[arg(short, long, default_value = "krusty")]
    theme: String,

    /// Run as ACP (Agent Client Protocol) server
    ///
    /// When enabled, Krusty runs as an ACP-compatible agent that communicates
    /// via JSON-RPC over stdin/stdout. This mode is used when Krusty is
    /// spawned by an ACP-compatible editor (Zed, Neovim, etc.).
    #[arg(long)]
    acp: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Start a new chat session
    Chat,

    /// List available themes
    Themes,

    /// Show LSP server status
    Lsp {
        #[command(subcommand)]
        action: Option<LspCommands>,
    },

    /// Authenticate with providers
    Auth,
}

#[derive(Subcommand)]
enum LspCommands {
    /// List installed LSP extensions
    List,
    /// Install an LSP extension from Zed marketplace
    Install { name: String },
    /// Remove an LSP extension
    Remove { name: String },
    /// Show running servers
    Status,
}

/// List installed extensions by scanning the extensions directory
fn list_installed_extensions() -> Vec<String> {
    let ext_dir = paths::extensions_dir();
    if !ext_dir.exists() {
        return Vec::new();
    }

    let mut installed = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&ext_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                // Check for extension.toml and extension.wasm
                let manifest = path.join("extension.toml");
                let wasm = path.join("extension.wasm");
                if manifest.exists() && wasm.exists() {
                    if let Some(name) = path.file_name() {
                        installed.push(name.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }
    installed
}

/// Restore terminal state - called on panic or unexpected exit
fn restore_terminal() {
    use crossterm::{
        event::DisableMouseCapture,
        execute,
        terminal::{disable_raw_mode, LeaveAlternateScreen},
    };
    let _ = disable_raw_mode();
    let _ = execute!(std::io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
}

#[tokio::main]
async fn main() -> Result<()> {
    // Set up panic hook to restore terminal state
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        restore_terminal();
        original_hook(panic_info);
    }));

    // Initialize logging to file (not stdout/stderr which would mess up TUI)
    let log_dir = paths::logs_dir();
    std::fs::create_dir_all(&log_dir).ok();

    // Create null device path based on platform
    #[cfg(unix)]
    let null_device = "/dev/null";
    #[cfg(windows)]
    let null_device = "NUL";

    let log_file = std::fs::File::create(log_dir.join("krusty.log"))
        .unwrap_or_else(|_| std::fs::File::create(null_device).unwrap());

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::sync::Mutex::new(log_file))
        .with_ansi(false)
        .init();

    // Apply any pending update before starting TUI
    // This ensures updates are applied silently on restart
    if let Ok(Some(version)) = krusty_core::updater::apply_pending_update() {
        // Set env var so TUI can show success toast
        std::env::set_var("KRUSTY_JUST_UPDATED", &version);
        tracing::info!("Applied pending update to v{}", version);
    }

    let cli = Cli::parse();

    // If --acp flag is set, run in ACP server mode
    if cli.acp {
        tracing::info!("Starting Krusty in ACP server mode");

        // ACP mode: run as Agent Client Protocol server
        // All communication happens via stdin/stdout JSON-RPC
        let server = acp::AcpServer::new()?;
        return server.run().await;
    }

    // Verify theme exists
    let theme = tui::THEME_REGISTRY.get_or_default(&cli.theme);
    tracing::info!("Using theme: {} ({})", theme.display_name, theme.name);

    match cli.command {
        Some(Commands::Themes) => {
            println!("Available themes ({}):", tui::THEME_REGISTRY.count());
            for (name, theme) in tui::THEME_REGISTRY.list() {
                println!("  {} - {}", name, theme.display_name);
            }
        }
        Some(Commands::Lsp { action }) => {
            let ext_dir = paths::extensions_dir();

            match action {
                Some(LspCommands::List) => {
                    println!("Installed Zed Extensions:");
                    println!();

                    let installed = list_installed_extensions();
                    if installed.is_empty() {
                        println!("  No extensions installed.");
                        println!();
                        println!("  Install extensions from the Zed marketplace:");
                        println!("    krusty lsp install <extension-name>");
                        println!();
                        println!("  Or use /lsp in the TUI to browse available extensions.");
                    } else {
                        for name in &installed {
                            println!("  ✓ {}", name);
                        }
                        println!();
                        println!("  {} extension(s) installed", installed.len());
                    }
                }
                Some(LspCommands::Install { name }) => {
                    println!("Installing Zed extension: {}", name);
                    println!();

                    // Create extension directory
                    let target_dir = ext_dir.join(&name);
                    std::fs::create_dir_all(&target_dir)?;

                    // Download from Zed API (same as TUI)
                    let download_url =
                        crate::tui::popups::lsp_browser::LspBrowserPopup::download_url(&name);

                    println!("  Downloading from Zed extensions...");
                    println!("  URL: {}", download_url);

                    let client = reqwest::Client::new();
                    match client.get(&download_url).send().await {
                        Ok(response) => {
                            if response.status().is_success() {
                                let bytes = response.bytes().await?;

                                // Extract tar.gz
                                use flate2::read::GzDecoder;
                                use tar::Archive;

                                let decoder = GzDecoder::new(&bytes[..]);
                                let mut archive = Archive::new(decoder);
                                archive.unpack(&target_dir)?;

                                println!("  ✓ Downloaded and extracted");

                                // Try to load the extension to verify it works
                                let wasm_host = WasmHost::new(client.clone(), ext_dir.clone());
                                match wasm_host.load_extension_from_dir(&target_dir).await {
                                    Ok(ext) => {
                                        println!(
                                            "  ✓ Extension loaded: {} v{}",
                                            ext.manifest.name, ext.manifest.version
                                        );
                                        println!();
                                        println!("Extension installed successfully!");
                                        println!("Language servers will start automatically when you open matching files.");
                                    }
                                    Err(e) => {
                                        println!(
                                            "  ⚠ Extension downloaded but failed to load: {}",
                                            e
                                        );
                                        println!();
                                        println!("The extension files are in: {:?}", target_dir);
                                    }
                                }
                            } else {
                                println!("  ✗ Download failed: HTTP {}", response.status());
                                println!();
                                println!(
                                    "Extension '{}' may not exist in the Zed marketplace.",
                                    name
                                );
                                println!("Browse available extensions at: https://github.com/zed-industries/extensions");
                            }
                        }
                        Err(e) => {
                            println!("  ✗ Download failed: {}", e);
                        }
                    }
                }
                Some(LspCommands::Remove { name }) => {
                    println!("Removing extension: {}", name);

                    let target_dir = ext_dir.join(&name);
                    if target_dir.exists() {
                        std::fs::remove_dir_all(&target_dir)?;
                        println!("  ✓ Removed {}", name);
                    } else {
                        println!("  Extension '{}' not found", name);
                    }
                }
                Some(LspCommands::Status) => {
                    println!("LSP Extension Status:");
                    println!();

                    let installed = list_installed_extensions();
                    if installed.is_empty() {
                        println!("  No extensions installed.");
                    } else {
                        // Load each extension to show its info
                        let http_client = reqwest::Client::new();
                        let wasm_host = WasmHost::new(http_client, ext_dir.clone());
                        for name in &installed {
                            let ext_path = ext_dir.join(name);
                            match wasm_host.load_extension_from_dir(&ext_path).await {
                                Ok(ext) => {
                                    let servers: Vec<&String> =
                                        ext.manifest.language_servers.keys().collect();
                                    println!("  ✓ {} v{}", ext.manifest.name, ext.manifest.version);
                                    if !servers.is_empty() {
                                        let server_list: Vec<&str> =
                                            servers.iter().map(|s| s.as_str()).collect();
                                        println!(
                                            "    Language servers: {}",
                                            server_list.join(", ")
                                        );
                                    }
                                }
                                Err(e) => {
                                    println!("  ⚠ {} (failed to load: {})", name, e);
                                }
                            }
                        }
                    }

                    println!();
                    println!("Servers start automatically when files are opened.");
                }
                None => {
                    println!("Language Server Protocol (LSP) Management");
                    println!();
                    println!("Krusty uses Zed's WASM extension system for language servers.");
                    println!();
                    println!("Commands:");
                    println!("  krusty lsp list              List installed extensions");
                    println!(
                        "  krusty lsp install <name>    Install extension from Zed marketplace"
                    );
                    println!("  krusty lsp remove <name>     Remove installed extension");
                    println!("  krusty lsp status            Show extension details");
                    println!();
                    println!("In the TUI, use /lsp to browse and install extensions.");
                    println!();
                    println!("Browse Zed extensions: https://github.com/zed-industries/extensions");
                }
            }
        }
        Some(Commands::Auth) => {
            println!("Authentication");
            println!();
            println!("Use /auth in the TUI to configure API keys for AI providers.");
            println!();
            println!("Supported providers:");
            println!("  - Anthropic: https://console.anthropic.com/");
            println!("  - OpenRouter: https://openrouter.ai/keys");
            println!("  - OpenCode Zen: https://opencode.ai/zen");
        }
        Some(Commands::Chat) | None => {
            // Start TUI chat
            // Use CLI theme only if explicitly provided (not default)
            let theme_override = if cli.theme != "krusty" {
                Some(cli.theme.as_str())
            } else {
                None
            };
            let mut app = tui::App::new(theme_override).await;
            app.run().await?;
        }
    }

    Ok(())
}
