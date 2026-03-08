//! MCP server management endpoints

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

use krusty_core::mcp::McpServerStatus;

use crate::error::AppError;
use crate::AppState;

/// Build the MCP router
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_servers))
        .route("/reload", post(reload_config))
        .route("/:name/connect", post(connect_server))
        .route("/:name/disconnect", post(disconnect_server))
        .route("/:name/tools", get(list_tools))
}

/// Keep AI-visible MCP tools in sync with current connected MCP servers.
async fn sync_mcp_tool_registry(state: &AppState) {
    // Remove all previously-registered MCP wrappers, then re-register from live connections.
    state.tool_registry.unregister_by_prefix("mcp__").await;
    krusty_core::mcp::tool::register_mcp_tools(state.mcp_manager.clone(), &state.tool_registry)
        .await;
}

/// MCP server info for API response
#[derive(Serialize)]
pub struct McpServerResponse {
    pub name: String,
    pub server_type: String,
    pub status: String,
    pub connected: bool,
    pub tool_count: usize,
    pub tools: Vec<McpToolResponse>,
    pub error: Option<String>,
}

/// MCP tool info
#[derive(Serialize)]
pub struct McpToolResponse {
    pub name: String,
    pub description: Option<String>,
}

/// List all MCP servers and their status
async fn list_servers(
    State(state): State<AppState>,
) -> Result<Json<Vec<McpServerResponse>>, AppError> {
    let servers = state.mcp_manager.list_servers().await;

    let response: Vec<McpServerResponse> = servers
        .into_iter()
        .map(|s| McpServerResponse {
            name: s.name,
            server_type: s.server_type,
            status: s.status.to_string(),
            connected: matches!(s.status, McpServerStatus::Connected),
            tool_count: s.tool_count,
            tools: s
                .tools
                .into_iter()
                .map(|t| McpToolResponse {
                    name: t.name,
                    description: t.description,
                })
                .collect(),
            error: s.error,
        })
        .collect();

    Ok(Json(response))
}

/// Reload MCP configuration from .mcp.json
async fn reload_config(
    State(state): State<AppState>,
) -> Result<Json<Vec<McpServerResponse>>, AppError> {
    // Reload config
    state
        .mcp_manager
        .load_config()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to reload MCP config: {}", e)))?;

    sync_mcp_tool_registry(&state).await;

    // Return updated server list
    list_servers(State(state)).await
}

/// Connect to a specific MCP server
async fn connect_server(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerResponse>, AppError> {
    state
        .mcp_manager
        .connect(&name)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to connect to {}: {}", name, e)))?;

    sync_mcp_tool_registry(&state).await;

    // Get updated server info
    let servers = state.mcp_manager.list_servers().await;
    let server = servers
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| AppError::NotFound(format!("Server {} not found", name)))?;

    Ok(Json(McpServerResponse {
        name: server.name,
        server_type: server.server_type,
        status: server.status.to_string(),
        connected: matches!(server.status, McpServerStatus::Connected),
        tool_count: server.tool_count,
        tools: server
            .tools
            .into_iter()
            .map(|t| McpToolResponse {
                name: t.name,
                description: t.description,
            })
            .collect(),
        error: server.error,
    }))
}

/// Disconnect from a specific MCP server
async fn disconnect_server(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerResponse>, AppError> {
    state.mcp_manager.disconnect(&name).await;
    sync_mcp_tool_registry(&state).await;

    // Get updated server info
    let servers = state.mcp_manager.list_servers().await;
    let server = servers
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| AppError::NotFound(format!("Server {} not found", name)))?;

    Ok(Json(McpServerResponse {
        name: server.name,
        server_type: server.server_type,
        status: server.status.to_string(),
        connected: matches!(server.status, McpServerStatus::Connected),
        tool_count: server.tool_count,
        tools: server
            .tools
            .into_iter()
            .map(|t| McpToolResponse {
                name: t.name,
                description: t.description,
            })
            .collect(),
        error: server.error,
    }))
}

/// List tools for a specific server
async fn list_tools(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<McpToolResponse>>, AppError> {
    let servers = state.mcp_manager.list_servers().await;
    let server = servers
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| AppError::NotFound(format!("Server {} not found", name)))?;

    Ok(Json(
        server
            .tools
            .into_iter()
            .map(|t| McpToolResponse {
                name: t.name,
                description: t.description,
            })
            .collect(),
    ))
}
