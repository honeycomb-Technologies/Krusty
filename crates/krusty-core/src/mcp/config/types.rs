use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

const fn default_true() -> bool {
    true
}

const fn default_startup_timeout_ms() -> u64 {
    15_000
}

const fn default_tool_timeout_ms() -> u64 {
    60_000
}

/// MCP configuration merged from package, user-global, and project files.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfigRaw>,
    /// Local servers declared by the user in `~/.krusty/mcp.json` are trusted
    /// to auto-connect. Project/package declarations are always disconnected
    /// until an explicit user action.
    #[serde(skip)]
    pub(super) auto_connect_local_servers: HashSet<String>,
    #[serde(skip)]
    pub(super) project_servers: HashSet<String>,
    #[serde(skip)]
    pub(super) global_servers: HashSet<String>,
    /// Authority inherited from the exact enabled package descriptor that
    /// contributed each server. User-owned global/project overrides remove
    /// this entry during merge.
    #[serde(skip)]
    pub(super) package_server_authorities: HashMap<String, McpConnectionAuthority>,
}

/// Common behavior and governance controls for one MCP server.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerOptionsRaw {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub auto_connect: Option<bool>,
    #[serde(default = "default_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
    #[serde(default = "default_tool_timeout_ms")]
    pub tool_timeout_ms: u64,
    #[serde(default)]
    pub tools: McpToolRules,
}

/// Interactive OAuth 2.1 configuration for a remote MCP server.
///
/// Krusty discovers authorization-server metadata from the MCP resource URL,
/// uses PKCE for every browser flow, and dynamically registers a public client
/// unless `clientId` or `clientMetadataUrl` is supplied.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpOAuthConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_name: Option<String>,
    #[serde(default)]
    pub client_metadata_url: Option<String>,
}

impl Default for McpOAuthConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scopes: Vec::new(),
            client_id: None,
            client_name: None,
            client_metadata_url: None,
        }
    }
}

impl McpOAuthConfig {
    pub fn client_name(&self) -> &str {
        self.client_name.as_deref().unwrap_or("Mitsuro")
    }
}

impl Default for McpServerOptionsRaw {
    fn default() -> Self {
        Self {
            enabled: true,
            required: false,
            auto_connect: None,
            startup_timeout_ms: default_startup_timeout_ms(),
            tool_timeout_ms: default_tool_timeout_ms(),
            tools: McpToolRules::default(),
        }
    }
}

/// Raw server configuration from JSON.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged, rename_all = "camelCase")]
pub enum McpServerConfigRaw {
    /// Local server (spawns a process and uses stdio transport).
    Local {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
        #[serde(default)]
        cwd: Option<PathBuf>,
        #[serde(flatten)]
        options: McpServerOptionsRaw,
    },
    /// Remote server using Streamable HTTP.
    Remote {
        #[serde(rename = "type")]
        server_type: String,
        url: String,
        #[serde(default)]
        transport: Option<String>,
        #[serde(default, rename = "authorizationToken", alias = "authorization_token")]
        authorization_token: Option<String>,
        #[serde(default, rename = "bearerTokenEnvVar", alias = "bearer_token_env_var")]
        bearer_token_env_var: Option<String>,
        /// OAuth is used when enabled and no explicit bearer token is present.
        #[serde(default)]
        oauth: Option<McpOAuthConfig>,
        #[serde(default)]
        headers: HashMap<String, String>,
        /// Header name to environment-variable name mappings. Secret values
        /// stay outside the config file.
        #[serde(default, rename = "envHeaders", alias = "env_headers")]
        env_headers: HashMap<String, String>,
        #[serde(flatten)]
        options: McpServerOptionsRaw,
    },
}

/// Source of the effective server declaration after precedence is applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum McpConfigSource {
    Package,
    Global,
    Project,
}

/// Host authority available to an MCP declaration.
///
/// Process and network are deliberately independent: a plugin grant that
/// permits outbound network access must never authorize spawning a stdio
/// child, and a process-only grant must never authorize remote HTTP.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionAuthority {
    pub process: bool,
    pub network: bool,
}

impl McpConnectionAuthority {
    pub const NONE: Self = Self {
        process: false,
        network: false,
    };

    pub const FULL: Self = Self {
        process: true,
        network: true,
    };

    pub const fn new(process: bool, network: bool) -> Self {
        Self { process, network }
    }

    pub const fn is_empty(self) -> bool {
        !self.process && !self.network
    }

    pub const fn allows_process(self) -> bool {
        self.process
    }

    pub const fn allows_network(self) -> bool {
        self.network
    }
}

/// One package-provided MCP fragment plus the exact permission grant under
/// which it was contributed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPackageConfig {
    pub path: PathBuf,
    pub authority: McpConnectionAuthority,
}

impl McpPackageConfig {
    pub fn new(path: PathBuf, authority: McpConnectionAuthority) -> Self {
        Self { path, authority }
    }
}

/// Resolved server configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpServerConfig {
    Local {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        cwd: Option<PathBuf>,
        options: McpServerOptions,
    },
    Remote {
        url: String,
        transport: Option<String>,
        authorization_token: Option<String>,
        oauth: Option<McpOAuthConfig>,
        headers: HashMap<String, String>,
        options: McpServerOptions,
    },
}

/// Resolved common behavior and governance controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerOptions {
    pub enabled: bool,
    pub required: bool,
    pub auto_connect: bool,
    pub startup_timeout_ms: u64,
    pub tool_timeout_ms: u64,
    pub tools: McpToolRules,
    pub source: McpConfigSource,
    pub authority: McpConnectionAuthority,
}

/// Tool visibility and approval-classification rules.
///
/// Deny rules always win. When `allow` is non-empty, tools must match at least
/// one allow pattern. Patterns use shell-style glob syntax.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolRules {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub approval: HashMap<String, McpToolApproval>,
}

/// Explicit approval classification for a remote tool.
///
/// This metadata is carried through discovery and execution. Krusty's central
/// tool policy remains the enforcement authority and conservatively treats
/// unknown remote tools as write-capable.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum McpToolApproval {
    #[default]
    Inherit,
    #[serde(alias = "ask", alias = "always")]
    Prompt,
    #[serde(alias = "never")]
    Allow,
}

/// Connector-ready remote server descriptor retained for future provider
/// integrations. Current MCP execution does not bypass `McpManager`.
#[derive(Debug, Clone, Serialize)]
pub struct RemoteMcpServer {
    #[serde(rename = "type")]
    pub server_type: String,
    pub url: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization_token: Option<String>,
}

impl McpServerConfig {
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }

    pub fn options(&self) -> &McpServerOptions {
        match self {
            Self::Local { options, .. } | Self::Remote { options, .. } => options,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.options().enabled
    }

    pub fn is_required(&self) -> bool {
        self.options().required
    }

    pub fn should_auto_connect(&self) -> bool {
        self.is_enabled() && self.options().auto_connect
    }

    pub fn startup_timeout_ms(&self) -> u64 {
        self.options().startup_timeout_ms
    }

    pub fn tool_timeout_ms(&self) -> u64 {
        self.options().tool_timeout_ms
    }

    pub fn source(&self) -> McpConfigSource {
        self.options().source
    }

    pub fn declared_authority(&self) -> McpConnectionAuthority {
        self.options().authority
    }

    pub fn required_authority(&self) -> McpConnectionAuthority {
        match self {
            Self::Local { .. } => McpConnectionAuthority::new(true, false),
            Self::Remote { .. } => McpConnectionAuthority::new(false, true),
        }
    }

    pub fn is_authorized_by(&self, authority: McpConnectionAuthority) -> bool {
        match self {
            Self::Local { .. } => authority.allows_process(),
            Self::Remote { .. } => authority.allows_network(),
        }
    }

    pub fn transport_type(&self) -> &'static str {
        match self {
            Self::Local { .. } => "stdio",
            Self::Remote { .. } => "streamable-http",
        }
    }

    pub fn oauth(&self) -> Option<&McpOAuthConfig> {
        match self {
            Self::Remote { oauth, .. } => oauth.as_ref().filter(|oauth| oauth.enabled),
            Self::Local { .. } => None,
        }
    }

    pub fn allows_tool(&self, tool_name: &str) -> bool {
        let rules = &self.options().tools;
        if rules
            .deny
            .iter()
            .any(|pattern| glob_matches(pattern, tool_name))
        {
            return false;
        }
        rules.allow.is_empty()
            || rules
                .allow
                .iter()
                .any(|pattern| glob_matches(pattern, tool_name))
    }

    pub fn tool_approval(&self, tool_name: &str) -> McpToolApproval {
        let mut matches: Vec<_> = self
            .options()
            .tools
            .approval
            .iter()
            .filter(|(pattern, _)| glob_matches(pattern, tool_name))
            .collect();
        // Prefer the most specific rule, then lexical order for determinism.
        matches.sort_by(|(left, _), (right, _)| {
            pattern_specificity(right)
                .cmp(&pattern_specificity(left))
                .then_with(|| left.cmp(right))
        });
        matches
            .first()
            .map(|(_, approval)| **approval)
            .unwrap_or_default()
    }
}

fn pattern_specificity(pattern: &str) -> usize {
    pattern
        .chars()
        .filter(|character| !matches!(character, '*' | '?' | '[' | ']'))
        .count()
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    globset::Glob::new(pattern)
        .ok()
        .map(|glob| glob.compile_matcher().is_match(value))
        .unwrap_or_else(|| pattern == value)
}

#[cfg(test)]
mod rule_tests {
    use super::*;

    fn options(rules: McpToolRules) -> McpServerOptions {
        McpServerOptions {
            enabled: true,
            required: false,
            auto_connect: true,
            startup_timeout_ms: 1,
            tool_timeout_ms: 1,
            tools: rules,
            source: McpConfigSource::Global,
            authority: McpConnectionAuthority::FULL,
        }
    }

    #[test]
    fn deny_wins_over_allow() {
        let config = McpServerConfig::Remote {
            url: String::new(),
            transport: None,
            authorization_token: None,
            oauth: None,
            headers: HashMap::new(),
            options: options(McpToolRules {
                allow: vec!["repo_*".into()],
                deny: vec!["repo_delete".into()],
                approval: HashMap::new(),
            }),
        };

        assert!(config.allows_tool("repo_read"));
        assert!(!config.allows_tool("repo_delete"));
        assert!(!config.allows_tool("web_search"));
    }

    #[test]
    fn most_specific_approval_rule_wins() {
        let config = McpServerConfig::Remote {
            url: String::new(),
            transport: None,
            authorization_token: None,
            oauth: None,
            headers: HashMap::new(),
            options: options(McpToolRules {
                allow: Vec::new(),
                deny: Vec::new(),
                approval: HashMap::from([
                    ("repo_*".into(), McpToolApproval::Allow),
                    ("repo_delete".into(), McpToolApproval::Prompt),
                ]),
            }),
        };

        assert_eq!(config.tool_approval("repo_delete"), McpToolApproval::Prompt);
        assert_eq!(config.tool_approval("repo_read"), McpToolApproval::Allow);
    }
}
