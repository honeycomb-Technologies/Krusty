//! MCP server management and capability endpoints.

use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use mitsuro_core::mcp::{McpConfigSource, McpServerInfo, McpServerStatus, McpToolApproval};

use crate::error::AppError;
use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_servers))
        .route("/reload", post(reload_config))
        .route("/:name/connect", post(connect_server))
        .route("/:name/disconnect", post(disconnect_server))
        .route("/:name/oauth/status", get(oauth_status))
        .route("/:name/oauth/start", post(oauth_start))
        .route(
            "/:name/oauth/callback",
            get(oauth_callback_get).post(oauth_callback_post),
        )
        .route("/:name/oauth/logout", post(oauth_logout))
        .route("/:name/tools", get(list_tools))
        .route("/:name/tools/refresh", post(refresh_tools))
        .route("/:name/resources", get(list_resources))
        .route("/:name/resource-templates", get(list_resource_templates))
        .route("/:name/resources/read", post(read_resource))
        .route("/:name/prompts", get(list_prompts))
        .route("/:name/prompts/get", post(get_prompt))
}

async fn oauth_status(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<mitsuro_core::mcp::McpOAuthStatus>, AppError> {
    state
        .mcp_manager
        .oauth_status(&name)
        .await
        .map(Json)
        .map_err(|error| AppError::BadRequest(format!("MCP OAuth status failed: {error}")))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthStartRequest {
    pub redirect_uri: String,
}

async fn oauth_start(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<McpOAuthStartRequest>,
) -> Result<Json<mitsuro_core::mcp::McpOAuthStart>, AppError> {
    state
        .mcp_manager
        .start_oauth(&name, &request.redirect_uri)
        .await
        .map(Json)
        .map_err(|error| AppError::BadRequest(format!("Failed to start MCP OAuth: {error}")))
}

#[derive(Deserialize)]
pub struct McpOAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthCallbackRequest {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

async fn oauth_callback_get(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Query(request): Query<McpOAuthCallbackQuery>,
) -> Result<Json<mitsuro_core::mcp::McpOAuthStatus>, AppError> {
    finish_oauth_callback(
        state,
        name,
        request.code,
        request.state,
        request.error,
        request.error_description,
    )
    .await
}

async fn oauth_callback_post(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<McpOAuthCallbackRequest>,
) -> Result<Json<mitsuro_core::mcp::McpOAuthStatus>, AppError> {
    finish_oauth_callback(
        state,
        name,
        request.code,
        request.state,
        request.error,
        request.error_description,
    )
    .await
}

async fn finish_oauth_callback(
    state: AppState,
    name: String,
    code: Option<String>,
    csrf_state: Option<String>,
    authorization_error: Option<String>,
    error_description: Option<String>,
) -> Result<Json<mitsuro_core::mcp::McpOAuthStatus>, AppError> {
    if let Some(error) = authorization_error {
        let csrf_state =
            csrf_state.ok_or_else(|| AppError::BadRequest("Missing OAuth state".into()))?;
        state
            .mcp_manager
            .cancel_oauth_callback(&name, &csrf_state)
            .await
            .map_err(|error| {
                AppError::BadRequest(format!(
                    "MCP OAuth error callback validation failed: {error}"
                ))
            })?;
        let description = error_description.unwrap_or_else(|| "authorization was denied".into());
        return Err(AppError::BadRequest(format!(
            "MCP OAuth provider returned {error}: {description}"
        )));
    }
    let code = code.ok_or_else(|| AppError::BadRequest("Missing OAuth code".into()))?;
    let csrf_state =
        csrf_state.ok_or_else(|| AppError::BadRequest("Missing OAuth state".into()))?;
    let status = state
        .mcp_manager
        .complete_oauth(&name, &code, &csrf_state)
        .await
        .map_err(|error| AppError::BadRequest(format!("MCP OAuth callback failed: {error}")))?;
    sync_mcp_tool_registry(&state).await;
    Ok(Json(status))
}

async fn oauth_logout(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<mitsuro_core::mcp::McpOAuthStatus>, AppError> {
    let status = state
        .mcp_manager
        .logout_oauth(&name)
        .await
        .map_err(|error| AppError::BadRequest(format!("MCP OAuth logout failed: {error}")))?;
    sync_mcp_tool_registry(&state).await;
    Ok(Json(status))
}

async fn sync_mcp_tool_registry(state: &AppState) {
    state.tool_registry.unregister_by_prefix("mcp__").await;
    mitsuro_core::mcp::tool::register_mcp_tools(state.mcp_manager.clone(), &state.tool_registry)
        .await;
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerResponse {
    pub name: String,
    pub server_type: String,
    pub source: McpConfigSource,
    pub enabled: bool,
    pub required: bool,
    pub status: String,
    pub connected: bool,
    pub tool_count: usize,
    pub tools: Vec<McpToolResponse>,
    pub instructions: Option<String>,
    pub server_info: Option<Value>,
    pub error: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolResponse {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub input_schema: Value,
    pub output_schema: Option<Value>,
    pub annotations: Option<Value>,
    pub approval: McpToolApproval,
}

async fn list_servers(
    State(state): State<AppState>,
) -> Result<Json<Vec<McpServerResponse>>, AppError> {
    if state.mcp_manager.refresh_changed_tools().await {
        sync_mcp_tool_registry(&state).await;
    }
    let response = state
        .mcp_manager
        .list_servers()
        .await
        .into_iter()
        .map(server_response)
        .collect();
    Ok(Json(response))
}

async fn reload_config(
    State(state): State<AppState>,
) -> Result<Json<Vec<McpServerResponse>>, AppError> {
    let load_result = state.mcp_manager.load_config().await;
    // The manager invalidates its snapshot on parse failure; mirror that
    // fail-closed transition in the registry before returning the error.
    sync_mcp_tool_registry(&state).await;
    load_result
        .map_err(|error| AppError::Internal(format!("Failed to reload MCP config: {error}")))?;
    state
        .mcp_manager
        .connect_all()
        .await
        .map_err(|error| AppError::Internal(format!("Required MCP server failure: {error}")))?;
    sync_mcp_tool_registry(&state).await;
    list_servers(State(state)).await
}

async fn connect_server(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerResponse>, AppError> {
    state
        .mcp_manager
        .connect_explicit(&name)
        .await
        .map_err(|error| AppError::Internal(format!("Failed to connect to {name}: {error}")))?;
    sync_mcp_tool_registry(&state).await;
    one_server(&state, &name).await
}

async fn disconnect_server(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<McpServerResponse>, AppError> {
    state.mcp_manager.disconnect(&name).await;
    sync_mcp_tool_registry(&state).await;
    one_server(&state, &name).await
}

async fn list_tools(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<McpToolResponse>>, AppError> {
    if state.mcp_manager.refresh_changed_tools().await {
        sync_mcp_tool_registry(&state).await;
    }
    let server = find_server(&state, &name).await?;
    Ok(Json(server.tools.into_iter().map(tool_response).collect()))
}

async fn refresh_tools(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Vec<McpToolResponse>>, AppError> {
    state
        .mcp_manager
        .refresh_tools(&name)
        .await
        .map_err(|error| AppError::Internal(format!("Failed to refresh {name}: {error}")))?;
    sync_mcp_tool_registry(&state).await;
    list_tools(State(state), Path(name)).await
}

async fn list_resources(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let resources = state
        .mcp_manager
        .list_resources(&name)
        .await
        .map_err(|error| mcp_request_error(&name, error))?;
    json_response(resources)
}

async fn list_resource_templates(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let templates = state
        .mcp_manager
        .list_resource_templates(&name)
        .await
        .map_err(|error| mcp_request_error(&name, error))?;
    json_response(templates)
}

#[derive(Deserialize)]
pub struct ReadResourceRequest {
    pub uri: String,
}

async fn read_resource(
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(request): Json<ReadResourceRequest>,
) -> Result<Json<Value>, AppError> {
    let resource = state
        .mcp_manager
        .read_resource(&name, &request.uri)
        .await
        .map_err(|error| mcp_request_error(&name, error))?;
    json_response(resource)
}

async fn list_prompts(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<Json<Value>, AppError> {
    let prompts = state
        .mcp_manager
        .list_prompts(&name)
        .await
        .map_err(|error| mcp_request_error(&name, error))?;
    json_response(prompts)
}

#[derive(Deserialize)]
pub struct GetPromptRequest {
    pub name: String,
    #[serde(default)]
    pub arguments: Option<Value>,
}

async fn get_prompt(
    State(state): State<AppState>,
    Path(server): Path<String>,
    Json(request): Json<GetPromptRequest>,
) -> Result<Json<Value>, AppError> {
    let prompt = state
        .mcp_manager
        .get_prompt(&server, &request.name, request.arguments)
        .await
        .map_err(|error| mcp_request_error(&server, error))?;
    json_response(prompt)
}

async fn one_server(state: &AppState, name: &str) -> Result<Json<McpServerResponse>, AppError> {
    Ok(Json(server_response(find_server(state, name).await?)))
}

async fn find_server(state: &AppState, name: &str) -> Result<McpServerInfo, AppError> {
    state
        .mcp_manager
        .list_servers()
        .await
        .into_iter()
        .find(|server| server.name == name)
        .ok_or_else(|| AppError::NotFound(format!("Server {name} not found")))
}

fn server_response(server: McpServerInfo) -> McpServerResponse {
    let connected = matches!(server.status, McpServerStatus::Connected);
    McpServerResponse {
        name: server.name,
        server_type: server.server_type,
        source: server.source,
        enabled: server.enabled,
        required: server.required,
        status: server.status.to_string(),
        connected,
        tool_count: server.tool_count,
        tools: server.tools.into_iter().map(tool_response).collect(),
        instructions: server.instructions,
        server_info: server.server_info,
        error: server.error,
    }
}

fn tool_response(tool: mitsuro_core::mcp::McpToolDef) -> McpToolResponse {
    McpToolResponse {
        name: tool.name,
        title: tool.title,
        description: tool.description,
        input_schema: tool.input_schema,
        output_schema: tool.output_schema,
        annotations: tool.annotations,
        approval: tool.approval,
    }
}

fn json_response<T: Serialize>(value: T) -> Result<Json<Value>, AppError> {
    serde_json::to_value(value)
        .map(Json)
        .map_err(|error| AppError::Internal(format!("Failed to serialize MCP response: {error}")))
}

fn mcp_request_error(server: &str, error: anyhow::Error) -> AppError {
    AppError::Internal(format!("MCP request to {server} failed: {error}"))
}
