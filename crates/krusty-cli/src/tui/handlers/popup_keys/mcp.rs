//! MCP browser popup keyboard handler

use crossterm::event::KeyCode;

use crate::tui::app::{App, Popup};
use crate::tui::utils::McpStatusUpdate;
use krusty_core::mcp::tool::register_mcp_tools;

impl App {
    /// Handle MCP browser popup keyboard events
    pub fn handle_mcp_popup_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => {
                self.ui.popup = Popup::None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.ui.popups.mcp.prev();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.ui.popups.mcp.next();
            }
            KeyCode::Enter => {
                self.ui.popups.mcp.toggle_expand();
            }
            KeyCode::Char('c') => {
                self.mcp_connect();
            }
            KeyCode::Char('d') => {
                self.mcp_disconnect();
            }
            _ => {}
        }
    }

    /// Connect to selected MCP server
    fn mcp_connect(&mut self) {
        if let Some(server) = self.ui.popups.mcp.get_selected() {
            if server.server_type == "remote" {
                self.ui
                    .popups
                    .mcp
                    .set_status("Remote servers handled by API".to_string());
                return;
            }

            let name = server.name.clone();
            let mcp = self.services.mcp_manager.clone();
            let registry = self.services.tool_registry.clone();
            let status_tx = self.services.mcp_status_tx.clone();

            self.ui
                .popups
                .mcp
                .set_status(format!("Connecting to {}...", name));

            tokio::spawn(async move {
                // Disconnect first if already connected (makes this a reconnect)
                mcp.disconnect(&name).await;
                match mcp.connect(&name).await {
                    Ok(()) => {
                        register_mcp_tools(mcp.clone(), &registry).await;
                        let tool_count = if let Some(client) = mcp.get_client(&name).await {
                            client.get_tools().await.len()
                        } else {
                            0
                        };
                        let _ = status_tx.send(McpStatusUpdate {
                            success: true,
                            message: format!("{} connected ({} tools)", name, tool_count),
                        });
                    }
                    Err(e) => {
                        let _ = status_tx.send(McpStatusUpdate {
                            success: false,
                            message: format!("{}: {}", name, e),
                        });
                    }
                }
            });
        }
    }

    /// Disconnect from selected MCP server
    fn mcp_disconnect(&mut self) {
        if let Some(server) = self.ui.popups.mcp.get_selected() {
            if server.server_type == "remote" {
                self.ui
                    .popups
                    .mcp
                    .set_status("Remote servers handled by API".to_string());
                return;
            }

            let name = server.name.clone();
            let mcp = self.services.mcp_manager.clone();
            let status_tx = self.services.mcp_status_tx.clone();

            tokio::spawn(async move {
                mcp.disconnect(&name).await;
                let _ = status_tx.send(McpStatusUpdate {
                    success: true,
                    message: format!("{} disconnected", name),
                });
            });
        }
    }
}
