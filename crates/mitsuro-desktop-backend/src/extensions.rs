//! MCP servers + plugins product surface types.
//!
//! Typed Codex app-server extension methods:
//! - `mcpServerStatus/list`
//! - `mcpServer/tool/call` (fixture-only safe offline)
//! - `plugin/list`, `plugin/read`, `plugin/installed`
//!
//! Includes MCP status/tool-call and plugin catalog shapes.
//! `McpServerToolCall*.json`, `PluginList*.json`, `PluginRead*.json`,
//! `PluginInstalled*.json`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// mcpServerStatus/list
// ---------------------------------------------------------------------------

/// How much MCP inventory data to fetch per server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum McpServerStatusDetail {
    #[default]
    Full,
    ToolsAndAuthOnly,
}

/// Auth posture advertised for an MCP server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum McpAuthStatus {
    #[default]
    Unsupported,
    NotLoggedIn,
    BearerToken,
    #[serde(rename = "oAuth")]
    OAuth,
}

impl McpAuthStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::NotLoggedIn => "notLoggedIn",
            Self::BearerToken => "bearerToken",
            Self::OAuth => "oAuth",
        }
    }

    /// Short UI status chip for Extensions panel.
    pub fn status_label(self) -> &'static str {
        match self {
            Self::Unsupported => "ready",
            Self::NotLoggedIn => "auth required",
            Self::BearerToken => "bearer",
            Self::OAuth => "oauth",
        }
    }
}

/// Params for `mcpServerStatus/list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListMcpServerStatusParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<McpServerStatusDetail>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

/// Presentation metadata from an initialized MCP server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website_url: Option<String>,
}

/// One MCP server row from `mcpServerStatus/list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_info: Option<McpServerInfo>,
    /// Tool inventory keyed by tool name (wire `tools` object).
    #[serde(default)]
    pub tools: BTreeMap<String, Value>,
    #[serde(default)]
    pub resources: Vec<Value>,
    #[serde(default)]
    pub resource_templates: Vec<Value>,
    pub auth_status: McpAuthStatus,
}

impl McpServerStatus {
    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn display_title(&self) -> &str {
        self.server_info
            .as_ref()
            .and_then(|i| i.title.as_deref())
            .filter(|t| !t.is_empty())
            .unwrap_or(self.name.as_str())
    }

    /// Combined status string for the Extensions UI (name + auth).
    pub fn status_label(&self) -> String {
        let auth = self.auth_status.status_label();
        let n = self.tool_count();
        if n == 0 {
            auth.to_string()
        } else {
            format!("{auth} · {n} tool(s)")
        }
    }
}

/// Response for `mcpServerStatus/list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ListMcpServerStatusResponse {
    pub data: Vec<McpServerStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// mcpServer/tool/call
// ---------------------------------------------------------------------------

/// Params for `mcpServer/tool/call`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerToolCallParams {
    pub thread_id: String,
    pub server: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

impl McpServerToolCallParams {
    pub fn new(
        thread_id: impl Into<String>,
        server: impl Into<String>,
        tool: impl Into<String>,
    ) -> Self {
        Self {
            thread_id: thread_id.into(),
            server: server.into(),
            tool: tool.into(),
            arguments: None,
            meta: None,
        }
    }
}

/// Response for `mcpServer/tool/call`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpServerToolCallResponse {
    pub content: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
    pub meta: Option<Value>,
}

// ---------------------------------------------------------------------------
// plugin/list · plugin/installed · plugin/read
// ---------------------------------------------------------------------------

/// Plugin package source (wire tagged union).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum PluginSource {
    Local {
        path: String,
    },
    Git {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ref_name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sha: Option<String>,
    },
    Npm {
        package: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        registry: Option<String>,
    },
    Remote,
}

impl PluginSource {
    pub fn label(&self) -> String {
        match self {
            Self::Local { path } => format!("local:{path}"),
            Self::Git { url, .. } => format!("git:{url}"),
            Self::Npm { package, .. } => format!("npm:{package}"),
            Self::Remote => "remote".into(),
        }
    }
}

/// Install policy for a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PluginInstallPolicy {
    NotAvailable,
    #[default]
    Available,
    InstalledByDefault,
}

/// When auth is required for a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PluginAuthPolicy {
    #[default]
    OnInstall,
    OnUse,
}

/// Availability for install/use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PluginAvailability {
    #[default]
    Available,
    DisabledByAdmin,
}

/// UI-facing interface metadata subset (optional on summary).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginInterface {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub developer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Marketplace / catalog plugin summary.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub source: PluginSource,
    pub installed: bool,
    pub enabled: bool,
    pub install_policy: PluginInstallPolicy,
    pub auth_policy: PluginAuthPolicy,
    #[serde(default)]
    pub availability: PluginAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<PluginInterface>,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Extra wire fields (shareContext, installPolicySource, …) ignored on deserialize.
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

impl PluginSummary {
    pub fn display_name(&self) -> &str {
        self.interface
            .as_ref()
            .and_then(|i| i.display_name.as_deref())
            .filter(|s| !s.is_empty())
            .unwrap_or(self.name.as_str())
    }

    pub fn short_description(&self) -> Option<&str> {
        self.interface
            .as_ref()
            .and_then(|i| i.short_description.as_deref())
            .filter(|s| !s.is_empty())
    }

    /// Marketplace category (`featured`, `productivity`, `creativity`, …).
    pub fn category(&self) -> &str {
        self.interface
            .as_ref()
            .and_then(|i| i.category.as_deref())
            .filter(|s| !s.is_empty())
            .unwrap_or("other")
    }

    pub fn status_label(&self) -> &'static str {
        if !self.installed {
            "not installed"
        } else if self.enabled {
            "installed · enabled"
        } else {
            "installed · disabled"
        }
    }
}

/// One marketplace bucket in `plugin/list` / `plugin/installed`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginMarketplaceEntry {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default)]
    pub plugins: Vec<PluginSummary>,
    /// Marketplace interface blob — keep flexible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface: Option<Value>,
}

/// Params for `plugin/list`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginListParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwds: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_kinds: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub force_refetch: Option<bool>,
}

/// Response for `plugin/list`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginListResponse {
    pub marketplaces: Vec<PluginMarketplaceEntry>,
    #[serde(default)]
    pub marketplace_load_errors: Vec<Value>,
    #[serde(default)]
    pub featured_plugin_ids: Vec<String>,
}

impl PluginListResponse {
    /// Flatten all plugins across marketplaces.
    pub fn all_plugins(&self) -> Vec<&PluginSummary> {
        self.marketplaces
            .iter()
            .flat_map(|m| m.plugins.iter())
            .collect()
    }

    pub fn plugin_count(&self) -> usize {
        self.marketplaces.iter().map(|m| m.plugins.len()).sum()
    }

    pub fn installed_count(&self) -> usize {
        self.marketplaces
            .iter()
            .flat_map(|m| m.plugins.iter())
            .filter(|p| p.installed)
            .count()
    }
}

/// Params for `plugin/installed`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstalledParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwds: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_suggestion_plugin_names: Option<Vec<String>>,
}

/// Response for `plugin/installed` (marketplaces of installed plugins).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginInstalledResponse {
    pub marketplaces: Vec<PluginMarketplaceEntry>,
    #[serde(default)]
    pub marketplace_load_errors: Vec<Value>,
}

impl PluginInstalledResponse {
    pub fn all_plugins(&self) -> Vec<&PluginSummary> {
        self.marketplaces
            .iter()
            .flat_map(|m| m.plugins.iter())
            .collect()
    }

    pub fn plugin_count(&self) -> usize {
        self.marketplaces.iter().map(|m| m.plugins.len()).sum()
    }
}

/// Params for `plugin/read`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginReadParams {
    pub plugin_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_marketplace_name: Option<String>,
}

impl PluginReadParams {
    pub fn new(plugin_name: impl Into<String>) -> Self {
        Self {
            plugin_name: plugin_name.into(),
            marketplace_path: None,
            remote_marketplace_name: None,
        }
    }
}

/// Detailed plugin payload from `plugin/read` (UI subset + flexible extras).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginDetail {
    pub marketplace_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marketplace_path: Option<String>,
    pub summary: PluginSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub skills: Vec<Value>,
    #[serde(default)]
    pub hooks: Vec<Value>,
    #[serde(default)]
    pub apps: Vec<Value>,
    #[serde(default)]
    pub app_templates: Vec<Value>,
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled_tasks: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_url: Option<String>,
}

/// Response for `plugin/read`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PluginReadResponse {
    pub plugin: PluginDetail,
}

// ---------------------------------------------------------------------------
// Fixture demos
// ---------------------------------------------------------------------------

fn tool_def(name: &str, description: &str) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": { "type": "object", "properties": {} },
    })
}

/// Offline demo MCP servers (2–3) for fixture + Extensions panel.
pub fn fixture_demo_mcp_servers() -> ListMcpServerStatusResponse {
    let mut filesystem_tools = BTreeMap::new();
    filesystem_tools.insert(
        "read_file".into(),
        tool_def("read_file", "Read a file from the workspace"),
    );
    filesystem_tools.insert(
        "list_dir".into(),
        tool_def("list_dir", "List directory entries"),
    );

    let mut github_tools = BTreeMap::new();
    github_tools.insert(
        "search_issues".into(),
        tool_def("search_issues", "Search repository issues"),
    );
    github_tools.insert(
        "get_pr".into(),
        tool_def("get_pr", "Fetch a pull request summary"),
    );

    let mut docs_tools = BTreeMap::new();
    docs_tools.insert(
        "search_docs".into(),
        tool_def("search_docs", "Search product documentation"),
    );

    ListMcpServerStatusResponse {
        data: vec![
            McpServerStatus {
                name: "fixture-filesystem".into(),
                server_info: Some(McpServerInfo {
                    name: "fixture-filesystem".into(),
                    version: "1.0.0".into(),
                    title: Some("Filesystem".into()),
                    description: Some("Offline demo MCP · virtual project files".into()),
                    website_url: None,
                }),
                tools: filesystem_tools,
                resources: vec![json!({
                    "name": "readme",
                    "uri": "fixture://project/README.md",
                })],
                resource_templates: vec![],
                auth_status: McpAuthStatus::Unsupported,
            },
            McpServerStatus {
                name: "fixture-github".into(),
                server_info: Some(McpServerInfo {
                    name: "fixture-github".into(),
                    version: "0.9.0".into(),
                    title: Some("GitHub".into()),
                    description: Some("Offline demo MCP · requires OAuth in live mode".into()),
                    website_url: Some("https://example.com/fixture-github".into()),
                }),
                tools: github_tools,
                resources: vec![],
                resource_templates: vec![],
                auth_status: McpAuthStatus::NotLoggedIn,
            },
            McpServerStatus {
                name: "fixture-docs".into(),
                server_info: Some(McpServerInfo {
                    name: "fixture-docs".into(),
                    version: "2.1.0".into(),
                    title: Some("Docs".into()),
                    description: Some("Offline demo MCP · docs search with bearer token".into()),
                    website_url: None,
                }),
                tools: docs_tools,
                resources: vec![],
                resource_templates: vec![],
                auth_status: McpAuthStatus::BearerToken,
            },
        ],
        next_cursor: None,
    }
}

fn plugin_summary(
    id: &str,
    name: &str,
    display: &str,
    short: &str,
    installed: bool,
    enabled: bool,
    version: &str,
    source: PluginSource,
    category: &str,
) -> PluginSummary {
    PluginSummary {
        id: id.into(),
        name: name.into(),
        source,
        installed,
        enabled,
        install_policy: if installed {
            PluginInstallPolicy::InstalledByDefault
        } else {
            PluginInstallPolicy::Available
        },
        auth_policy: PluginAuthPolicy::OnInstall,
        availability: PluginAvailability::Available,
        version: Some(version.into()),
        local_version: if installed {
            Some(version.into())
        } else {
            None
        },
        remote_plugin_id: None,
        interface: Some(PluginInterface {
            display_name: Some(display.into()),
            short_description: Some(short.into()),
            long_description: None,
            developer_name: Some("OpenAI".into()),
            category: Some(category.into()),
            capabilities: vec!["skills".into()],
        }),
        keywords: vec![category.into()],
        extra: Map::new(),
    }
}

/// Offline marketplace catalog for bar-density Plugins surface.
///
/// Visible grids cap at 6 per section with a "See N more" overflow in the UI;
/// installed strip aims for ~12 brand chips like bar-plugins-real.
/// Internal `tools` entries stay available for `plugin/read` but are hidden
/// from the marketplace chrome (no trademark bitmaps — geometric brand-mark chips).
pub fn fixture_demo_plugins() -> PluginListResponse {
    let plugins = vec![
        // —— Featured (6 visible: Chrome…Outlook + overflow starting GitHub/SharePoint) ——
        plugin_summary(
            "plugin.chrome",
            "chrome",
            "Chrome",
            "Control Chrome with ChatGPT",
            true,
            true,
            "1.4.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.spreadsheets",
            "spreadsheets",
            "Spreadsheets",
            "Create and edit spreadsheet files",
            true,
            true,
            "1.1.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.presentations",
            "presentations",
            "Presentations",
            "Create and edit presentations",
            true,
            true,
            "1.0.2",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.gmail",
            "gmail",
            "Gmail",
            "Read and manage Gmail",
            true,
            true,
            "2.0.1",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.google-drive",
            "google-drive",
            "Google Drive",
            "Work across Drive, Docs, Sheets…",
            true,
            true,
            "1.0.3",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.outlook",
            "outlook",
            "Outlook Email",
            "Triage Microsoft Outlook inboxes…",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "featured",
        ),
        // Featured overflow (bar: "See GitHub, SharePoint, and N more")
        plugin_summary(
            "plugin.github",
            "github",
            "GitHub",
            "Issues, PRs, and repo workflows",
            true,
            true,
            "1.6.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.sharepoint",
            "sharepoint",
            "SharePoint",
            "Browse and update SharePoint sites",
            false,
            false,
            "0.8.1",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.teams",
            "teams",
            "Microsoft Teams",
            "Chat and channels for Teams",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.onedrive",
            "onedrive",
            "OneDrive",
            "Files across OneDrive",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.slack",
            "slack",
            "Slack",
            "Search and post in Slack",
            false,
            false,
            "1.2.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.zoom",
            "zoom",
            "Zoom",
            "Meetings and transcripts",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.discord",
            "discord",
            "Discord",
            "Servers, channels, and DMs",
            false,
            false,
            "0.9.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.whatsapp",
            "whatsapp",
            "WhatsApp",
            "Message and media workflows",
            false,
            false,
            "0.8.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.linkedin",
            "linkedin",
            "LinkedIn",
            "Posts and professional inbox",
            false,
            false,
            "0.7.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.youtube",
            "youtube",
            "YouTube",
            "Transcripts and channel research",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.spotify",
            "spotify",
            "Spotify",
            "Playlists and listening data",
            false,
            false,
            "0.6.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.reddit",
            "reddit",
            "Reddit",
            "Search and summarize threads",
            false,
            false,
            "0.5.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.x-twitter",
            "x-twitter",
            "X",
            "Posts and lists on X",
            false,
            false,
            "0.5.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.safari",
            "safari",
            "Safari",
            "Browse and control Safari",
            false,
            false,
            "0.4.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.firefox",
            "firefox",
            "Firefox",
            "Browse and control Firefox",
            false,
            false,
            "0.4.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.edge",
            "edge",
            "Edge",
            "Browse and control Edge",
            false,
            false,
            "0.4.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.arc",
            "arc",
            "Arc",
            "Spaces and tabs in Arc",
            false,
            false,
            "0.3.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.bitwarden",
            "bitwarden",
            "Bitwarden",
            "Vault lookups with approval",
            false,
            false,
            "0.3.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.1password",
            "1password",
            "1Password",
            "Secrets with user approval",
            false,
            false,
            "0.3.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.superhuman",
            "superhuman",
            "Superhuman",
            "Fast email triage",
            false,
            false,
            "0.2.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.front",
            "front",
            "Front",
            "Shared inboxes for teams",
            false,
            false,
            "0.2.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.intercom",
            "intercom",
            "Intercom",
            "Customer conversations",
            false,
            false,
            "0.2.0",
            PluginSource::Remote,
            "featured",
        ),
        plugin_summary(
            "plugin.zendesk",
            "zendesk",
            "Zendesk",
            "Tickets and help center",
            false,
            false,
            "0.2.0",
            PluginSource::Remote,
            "featured",
        ),
        // —— Productivity (6 visible + Documents/PDF + deep overflow) ——
        plugin_summary(
            "plugin.notion",
            "notion",
            "Notion",
            "Notion workflows for specs…",
            true,
            true,
            "1.2.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.google-calendar",
            "google-calendar",
            "Google Calendar",
            "Manage Google Calendar events…",
            true,
            true,
            "1.1.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.linear",
            "linear",
            "Linear",
            "Plan and build products",
            false,
            false,
            "0.9.4",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.clickup",
            "clickup",
            "ClickUp",
            "Turn Codex into your ClickUp…",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.dropbox",
            "dropbox",
            "Dropbox",
            "Access, save and share files",
            true,
            true,
            "1.3.2",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.asana",
            "asana",
            "Asana",
            "Turn chats into actions",
            false,
            false,
            "1.0.4",
            PluginSource::Remote,
            "productivity",
        ),
        // Productivity overflow (bar: "See Documents, PDF, and N more")
        plugin_summary(
            "plugin.documents",
            "documents",
            "Documents",
            "Create and edit document files",
            true,
            true,
            "1.0.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.pdf",
            "pdf",
            "PDF",
            "Read and annotate PDF files",
            true,
            true,
            "1.0.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.jira",
            "jira",
            "Jira",
            "Issues and sprints",
            false,
            false,
            "1.1.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.confluence",
            "confluence",
            "Confluence",
            "Docs and spaces",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.trello",
            "trello",
            "Trello",
            "Boards and cards",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.monday",
            "monday",
            "Monday.com",
            "Work boards and timelines",
            false,
            false,
            "0.9.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.airtable",
            "airtable",
            "Airtable",
            "Bases and automations",
            false,
            false,
            "0.9.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.todoist",
            "todoist",
            "Todoist",
            "Tasks and projects",
            false,
            false,
            "1.0.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.evernote",
            "evernote",
            "Evernote",
            "Notes and notebooks",
            false,
            false,
            "0.8.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.coda",
            "coda",
            "Coda",
            "Docs as apps",
            false,
            false,
            "0.8.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.smartsheet",
            "smartsheet",
            "Smartsheet",
            "Sheets and portfolios",
            false,
            false,
            "0.7.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.box",
            "box",
            "Box",
            "Secure file storage",
            false,
            false,
            "0.7.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.miro",
            "miro",
            "Miro",
            "Whiteboards and sticky notes",
            false,
            false,
            "0.8.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.lucid",
            "lucid",
            "Lucid",
            "Diagrams and flows",
            false,
            false,
            "0.6.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.basecamp",
            "basecamp",
            "Basecamp",
            "Projects and message boards",
            false,
            false,
            "0.6.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.height",
            "height",
            "Height",
            "Autonomous project management",
            false,
            false,
            "0.5.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.shortcut",
            "shortcut",
            "Shortcut",
            "Stories and iterations",
            false,
            false,
            "0.5.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.pagerduty",
            "pagerduty",
            "PagerDuty",
            "Incidents and on-call",
            false,
            false,
            "0.5.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.datadog",
            "datadog",
            "Datadog",
            "Metrics, logs, traces",
            false,
            false,
            "0.5.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.sentry",
            "sentry",
            "Sentry",
            "Error tracking",
            false,
            false,
            "0.5.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.hubspot",
            "hubspot",
            "HubSpot",
            "CRM and marketing",
            false,
            false,
            "0.6.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.salesforce",
            "salesforce",
            "Salesforce",
            "CRM records and flows",
            false,
            false,
            "0.6.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.zapier",
            "zapier",
            "Zapier",
            "Automations between apps",
            false,
            false,
            "0.7.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.make",
            "make",
            "Make",
            "Visual automation scenarios",
            false,
            false,
            "0.5.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.obsidian",
            "obsidian",
            "Obsidian",
            "Local knowledge graphs",
            false,
            false,
            "0.6.0",
            PluginSource::Remote,
            "productivity",
        ),
        plugin_summary(
            "plugin.raycast",
            "raycast",
            "Raycast",
            "Launcher and extensions",
            false,
            false,
            "0.4.0",
            PluginSource::Remote,
            "productivity",
        ),
        // —— Creativity (6 visible like bar) ——
        plugin_summary(
            "plugin.canva",
            "canva",
            "Canva",
            "Create, review, edit designs",
            true,
            true,
            "2.1.0",
            PluginSource::Remote,
            "creativity",
        ),
        plugin_summary(
            "plugin.figma",
            "figma",
            "Figma",
            "Design-to-code workflows…",
            false,
            false,
            "1.5.0",
            PluginSource::Remote,
            "creativity",
        ),
        plugin_summary(
            "plugin.gamma",
            "gamma",
            "Gamma",
            "Create presentations and docs",
            false,
            false,
            "1.0.1",
            PluginSource::Remote,
            "creativity",
        ),
        plugin_summary(
            "plugin.descript",
            "descript",
            "Descript",
            "Edit video by chatting",
            false,
            false,
            "1.2.0",
            PluginSource::Remote,
            "creativity",
        ),
        plugin_summary(
            "plugin.adobe",
            "adobe",
            "Adobe (formerly Photoshop)",
            "Edit images with chat",
            false,
            false,
            "0.9.0",
            PluginSource::Remote,
            "creativity",
        ),
        plugin_summary(
            "plugin.product-design",
            "product-design",
            "Product Design",
            "Wireframes and design systems",
            false,
            false,
            "0.4.0",
            PluginSource::Remote,
            "creativity",
        ),
        // Internal read/test targets — not shown in marketplace chrome
        plugin_summary(
            "plugin.review",
            "fixture-review",
            "Review Checklist",
            "Code review checklist for PRs",
            true,
            true,
            "1.0.0",
            PluginSource::Local {
                path: "/tmp/mitsuro-fixture-home/plugins/fixture-review".into(),
            },
            "tools",
        ),
        plugin_summary(
            "plugin.mcp-bridge",
            "fixture-mcp-bridge",
            "MCP Bridge",
            "Bundles MCP servers into a plugin",
            true,
            true,
            "0.3.1",
            PluginSource::Npm {
                package: "@mitsuro/mcp-bridge".into(),
                version: Some("0.3.1".into()),
                registry: None,
            },
            "tools",
        ),
    ];

    PluginListResponse {
        marketplaces: vec![PluginMarketplaceEntry {
            name: "public".into(),
            path: Some("/tmp/mitsuro-fixture-home/marketplaces/public.json".into()),
            plugins,
            interface: None,
        }],
        marketplace_load_errors: vec![],
        featured_plugin_ids: vec![
            "plugin.chrome".into(),
            "plugin.spreadsheets".into(),
            "plugin.presentations".into(),
            "plugin.gmail".into(),
            "plugin.google-drive".into(),
            "plugin.outlook".into(),
        ],
    }
}

/// Installed-only subset of [`fixture_demo_plugins`].
pub fn fixture_demo_plugins_installed() -> PluginInstalledResponse {
    let full = fixture_demo_plugins();
    let marketplaces = full
        .marketplaces
        .into_iter()
        .map(|mut m| {
            m.plugins.retain(|p| p.installed);
            m
        })
        .filter(|m| !m.plugins.is_empty())
        .collect();
    PluginInstalledResponse {
        marketplaces,
        marketplace_load_errors: vec![],
    }
}

/// Lookup a demo plugin by name for `plugin/read`.
pub fn fixture_demo_plugin_read(plugin_name: &str) -> Option<PluginReadResponse> {
    let list = fixture_demo_plugins();
    for marketplace in &list.marketplaces {
        if let Some(summary) = marketplace
            .plugins
            .iter()
            .find(|p| p.name == plugin_name || p.id == plugin_name)
        {
            let mcp_servers = if summary.name == "fixture-mcp-bridge" {
                fixture_demo_mcp_servers()
                    .data
                    .iter()
                    .map(|s| s.name.clone())
                    .collect()
            } else {
                vec![]
            };
            return Some(PluginReadResponse {
                plugin: PluginDetail {
                    marketplace_name: marketplace.name.clone(),
                    marketplace_path: marketplace.path.clone(),
                    description: summary
                        .short_description()
                        .map(str::to_string)
                        .or_else(|| Some(format!("Plugin · {}", summary.display_name()))),
                    summary: summary.clone(),
                    skills: if summary.installed {
                        vec![json!({
                            "name": format!("{}-skill", summary.name),
                            "description": format!("Skill from {}", summary.display_name()),
                        })]
                    } else {
                        vec![]
                    },
                    hooks: vec![],
                    apps: vec![],
                    app_templates: vec![],
                    mcp_servers,
                    scheduled_tasks: None,
                    share_url: None,
                },
            });
        }
    }
    None
}

/// Safe fixture response for `mcpServer/tool/call` (never executes real tools).
pub fn fixture_mcp_tool_call(params: &McpServerToolCallParams) -> McpServerToolCallResponse {
    let servers = fixture_demo_mcp_servers();
    let server_ok = servers.data.iter().any(|s| s.name == params.server);
    if !server_ok {
        return McpServerToolCallResponse {
            content: vec![json!({
                "type": "text",
                "text": format!("fixture: unknown MCP server '{}'", params.server),
            })],
            structured_content: None,
            is_error: Some(true),
            meta: Some(json!({ "fixture": true })),
        };
    }
    let tool_known = servers
        .data
        .iter()
        .find(|s| s.name == params.server)
        .map(|s| s.tools.contains_key(&params.tool))
        .unwrap_or(false);
    if !tool_known {
        return McpServerToolCallResponse {
            content: vec![json!({
                "type": "text",
                "text": format!(
                    "fixture: unknown tool '{}' on server '{}'",
                    params.tool, params.server
                ),
            })],
            structured_content: None,
            is_error: Some(true),
            meta: Some(json!({ "fixture": true })),
        };
    }
    McpServerToolCallResponse {
        content: vec![json!({
            "type": "text",
            "text": format!(
                "[fixture] mcpServer/tool/call server={} tool={} threadId={} args={}",
                params.server,
                params.tool,
                params.thread_id,
                params
                    .arguments
                    .as_ref()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "null".into())
            ),
        })],
        structured_content: Some(json!({
            "fixture": true,
            "server": params.server,
            "tool": params.tool,
        })),
        is_error: Some(false),
        meta: Some(json!({ "fixture": true, "safe": true })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_mcp_list_has_two_to_three_servers() {
        let resp = fixture_demo_mcp_servers();
        assert!(
            (2..=3).contains(&resp.data.len()),
            "expected 2–3 MCP servers, got {}",
            resp.data.len()
        );
        assert!(resp.data.iter().any(|s| s.name == "fixture-filesystem"));
        assert!(resp.data.iter().any(|s| s.tool_count() > 0));
        let raw = serde_json::to_value(&resp).expect("serialize mcp list");
        assert!(raw.get("data").is_some());
        let round: ListMcpServerStatusResponse =
            serde_json::from_value(raw).expect("deserialize mcp list");
        assert_eq!(round.data.len(), resp.data.len());
    }

    #[test]
    fn fixture_plugin_list_has_marketplace_density() {
        let resp = fixture_demo_plugins();
        let n = resp.plugin_count();
        assert!(
            (40..=90).contains(&n),
            "expected 40–90 marketplace plugins (bar overflow density), got {n}"
        );
        // Brand-visible installed (exclude internal tools) should cover strip density.
        let brand_installed = resp
            .all_plugins()
            .iter()
            .filter(|p| p.installed && p.category() != "tools")
            .count();
        assert!(
            brand_installed >= 10,
            "expected dense brand installed strip (≥10), got {brand_installed}"
        );
        assert!(
            resp.installed_count() >= 10,
            "expected dense installed (≥10), got {}",
            resp.installed_count()
        );
        let cats: Vec<_> = resp.all_plugins().iter().map(|p| p.category()).collect();
        assert!(cats.iter().any(|c| *c == "featured"));
        assert!(cats.iter().any(|c| *c == "productivity"));
        assert!(cats.iter().any(|c| *c == "creativity"));
        // Overflow head names match bar copy.
        let featured: Vec<_> = resp
            .all_plugins()
            .iter()
            .filter(|p| p.category() == "featured")
            .map(|p| p.display_name())
            .collect();
        assert!(featured.len() > 8, "featured needs See-N-more overflow");
        assert_eq!(featured.get(6).copied(), Some("GitHub"));
        assert_eq!(featured.get(7).copied(), Some("SharePoint"));
        let productivity: Vec<_> = resp
            .all_plugins()
            .iter()
            .filter(|p| p.category() == "productivity")
            .map(|p| p.display_name())
            .collect();
        assert!(productivity.len() > 8);
        assert_eq!(productivity.get(6).copied(), Some("Documents"));
        assert_eq!(productivity.get(7).copied(), Some("PDF"));
        // Bar-named catalog entries used for density parity.
        let names: Vec<_> = resp
            .all_plugins()
            .iter()
            .map(|p| p.display_name())
            .collect();
        assert!(names.iter().any(|n| *n == "Outlook Email"));
        assert!(names.iter().any(|n| *n == "Google Calendar"));
        assert!(names.iter().any(|n| *n == "Asana"));
        assert!(names.iter().any(|n| *n == "Descript"));
        assert!(names.iter().any(|n| *n == "Adobe (formerly Photoshop)"));
        let installed = fixture_demo_plugins_installed();
        assert_eq!(installed.plugin_count(), resp.installed_count());
        assert!(installed.all_plugins().iter().all(|p| p.installed));
        assert!(!resp.featured_plugin_ids.is_empty());
    }

    #[test]
    fn fixture_plugin_read_and_tool_call_safe() {
        let read = fixture_demo_plugin_read("fixture-review").expect("plugin read");
        assert_eq!(read.plugin.summary.name, "fixture-review");
        assert!(read.plugin.summary.installed);

        let call = fixture_mcp_tool_call(&McpServerToolCallParams::new(
            "fixture-thread",
            "fixture-filesystem",
            "read_file",
        ));
        assert_eq!(call.is_error, Some(false));
        assert!(!call.content.is_empty());

        let bad = fixture_mcp_tool_call(&McpServerToolCallParams::new(
            "fixture-thread",
            "no-such-server",
            "x",
        ));
        assert_eq!(bad.is_error, Some(true));
    }

    #[test]
    fn plugin_summary_wire_camel_case() {
        let p = &fixture_demo_plugins().marketplaces[0].plugins[0];
        let v = serde_json::to_value(p).unwrap();
        assert!(v.get("installPolicy").is_some());
        assert!(v.get("authPolicy").is_some());
        assert_eq!(v.get("installed").and_then(|x| x.as_bool()), Some(true));
    }
}
