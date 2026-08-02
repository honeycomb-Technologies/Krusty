use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

pub(super) const DEFAULT_HOOK_TIMEOUT_SECONDS: u64 = 30;

fn default_hook_timeout_seconds() -> u64 {
    DEFAULT_HOOK_TIMEOUT_SECONDS
}

/// Type of user hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UserHookType {
    /// Runs before tool execution, can block.
    PreToolUse,
    /// Runs after tool execution.
    PostToolUse,
    /// Fires on notification events (non-blocking).
    Notification,
    /// Fires when user submits a prompt.
    UserPromptSubmit,
}

impl UserHookType {
    /// All hook types for UI display.
    pub fn all() -> &'static [UserHookType] {
        &[
            UserHookType::PreToolUse,
            UserHookType::PostToolUse,
            UserHookType::Notification,
            UserHookType::UserPromptSubmit,
        ]
    }

    /// Human-readable name.
    pub fn display_name(&self) -> &'static str {
        match self {
            UserHookType::PreToolUse => "PreToolUse",
            UserHookType::PostToolUse => "PostToolUse",
            UserHookType::Notification => "Notification",
            UserHookType::UserPromptSubmit => "UserPromptSubmit",
        }
    }

    /// Description for UI.
    pub fn description(&self) -> &'static str {
        match self {
            UserHookType::PreToolUse => "Before tool execution",
            UserHookType::PostToolUse => "After tool execution",
            UserHookType::Notification => "When notifications are sent",
            UserHookType::UserPromptSubmit => "When the user submits a prompt",
        }
    }

    /// Parse from string representation.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "PreToolUse" => Some(UserHookType::PreToolUse),
            "PostToolUse" => Some(UserHookType::PostToolUse),
            "Notification" => Some(UserHookType::Notification),
            "UserPromptSubmit" => Some(UserHookType::UserPromptSubmit),
            _ => None,
        }
    }
}

impl std::fmt::Display for UserHookType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Runtime provenance for a hook.
///
/// Package hooks are deliberately ephemeral: they are reconstructed from an
/// enabled plugin's immutable snapshot and are never written to `user_hooks`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UserHookSource {
    /// A hook explicitly created by the user and persisted in SQLite.
    #[default]
    User,
    /// A read-only hook contributed by an installed plugin package.
    Package {
        plugin_id: String,
        /// Internal immutable config path. Do not expose host paths through
        /// serialized hook responses.
        #[serde(skip)]
        config_path: PathBuf,
    },
}

impl UserHookSource {
    /// Whether this hook came from a plugin package.
    pub fn is_package(&self) -> bool {
        matches!(self, Self::Package { .. })
    }

    /// Package identifier for package hooks.
    pub fn plugin_id(&self) -> Option<&str> {
        match self {
            Self::User => None,
            Self::Package { plugin_id, .. } => Some(plugin_id),
        }
    }
}

/// One validated package hook configuration input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageHookConfig {
    pub plugin_id: String,
    pub config_path: PathBuf,
    pub package_root: PathBuf,
}

impl PackageHookConfig {
    pub fn new(
        plugin_id: impl Into<String>,
        config_path: impl Into<PathBuf>,
        package_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            plugin_id: plugin_id.into(),
            config_path: config_path.into(),
            package_root: package_root.into(),
        }
    }
}

/// Summary returned after atomically replacing all package hooks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PackageHookLoadReport {
    pub config_count: usize,
    pub hook_count: usize,
}

/// A user-defined hook.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserHook {
    /// Unique identifier.
    pub id: String,
    /// Type of hook.
    pub hook_type: UserHookType,
    /// Regex pattern to match tool names.
    pub tool_pattern: String,
    /// Shell command to execute.
    pub command: String,
    /// Whether this hook is enabled.
    pub enabled: bool,
    /// Maximum command runtime. Package formats may override the 30-second default.
    #[serde(default = "default_hook_timeout_seconds")]
    pub timeout_seconds: u64,
    /// When the hook was created.
    pub created_at: String,
    /// Whether this is a persisted user hook or an ephemeral package hook.
    #[serde(default)]
    pub source: UserHookSource,
    /// Owner of a persisted tenant hook. `None` identifies a local/global hook.
    /// Package hooks are global contributions and never carry an owner.
    #[serde(skip)]
    pub(super) owner_user_id: Option<String>,
    /// Working directory used for execution. Package hooks run from their
    /// immutable package root so relative script references are deterministic.
    #[serde(skip)]
    pub(super) working_dir: Option<PathBuf>,
    /// Compiled regex (not serialized).
    #[serde(skip)]
    pub(super) compiled_pattern: Option<Regex>,
}

impl UserHook {
    /// Create a new user hook.
    pub fn new(hook_type: UserHookType, tool_pattern: String, command: String) -> Self {
        let compiled = Regex::new(&tool_pattern).ok();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            hook_type,
            tool_pattern,
            command,
            enabled: true,
            timeout_seconds: DEFAULT_HOOK_TIMEOUT_SECONDS,
            created_at: chrono::Utc::now().to_rfc3339(),
            source: UserHookSource::User,
            owner_user_id: None,
            working_dir: None,
            compiled_pattern: compiled,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn new_package(
        id: String,
        hook_type: UserHookType,
        tool_pattern: String,
        command: String,
        enabled: bool,
        timeout_seconds: u64,
        plugin_id: String,
        config_path: PathBuf,
        package_root: PathBuf,
    ) -> Self {
        let compiled_pattern = Regex::new(&tool_pattern).ok();
        Self {
            id,
            hook_type,
            tool_pattern,
            command,
            enabled,
            timeout_seconds,
            created_at: chrono::Utc::now().to_rfc3339(),
            source: UserHookSource::Package {
                plugin_id,
                config_path,
            },
            owner_user_id: None,
            working_dir: Some(package_root),
            compiled_pattern,
        }
    }

    /// Whether this hook is an ephemeral, read-only package contribution.
    pub fn is_package_hook(&self) -> bool {
        self.source.is_package()
    }

    /// Persisted tenant owner, if any. Global and package hooks return `None`.
    pub fn owner_user_id(&self) -> Option<&str> {
        self.owner_user_id.as_deref()
    }

    /// Whether this hook is visible and executable for the given request user.
    /// Global persisted hooks and package hooks apply to every context, while a
    /// tenant-owned hook applies only to its exact owner.
    pub fn applies_to_user(&self, user_id: Option<&str>) -> bool {
        self.is_package_hook()
            || self.owner_user_id.is_none()
            || self.owner_user_id.as_deref() == user_id
    }

    /// Working directory used to execute this hook.
    pub fn working_dir(&self) -> Option<&Path> {
        self.working_dir.as_deref()
    }

    /// Check if this hook matches a tool name.
    pub fn matches(&mut self, tool_name: &str) -> bool {
        if !self.enabled {
            return false;
        }

        if self.compiled_pattern.is_none() {
            self.compiled_pattern = Regex::new(&self.tool_pattern).ok();
        }

        self.compiled_pattern
            .as_ref()
            .map(|re| re.is_match(tool_name))
            .unwrap_or(false)
    }

    /// Compile the pattern (call after loading from DB).
    pub fn compile_pattern(&mut self) {
        self.compiled_pattern = Regex::new(&self.tool_pattern).ok();
    }

    /// Check if the pattern is valid regex.
    pub fn is_pattern_valid(&self) -> bool {
        Regex::new(&self.tool_pattern).is_ok()
    }
}

/// Result of executing a user hook.
#[derive(Debug)]
pub enum UserHookResult {
    /// Continue with tool execution.
    Continue,
    /// Block tool execution with reason (exit code 2).
    Block { reason: String },
    /// Warning shown to user, but continue (other non-zero exit).
    Warn { message: String },
}

#[cfg(test)]
mod tests {
    use super::{UserHook, UserHookSource, UserHookType, DEFAULT_HOOK_TIMEOUT_SECONDS};

    fn create_test_hook(hook_type: UserHookType, pattern: &str, command: &str) -> UserHook {
        UserHook::new(hook_type, pattern.to_string(), command.to_string())
    }

    #[test]
    fn test_user_hook_type_display() {
        assert_eq!(UserHookType::PreToolUse.display_name(), "PreToolUse");
        assert_eq!(UserHookType::PostToolUse.display_name(), "PostToolUse");
        assert_eq!(UserHookType::Notification.display_name(), "Notification");
        assert_eq!(
            UserHookType::UserPromptSubmit.display_name(),
            "UserPromptSubmit"
        );
    }

    #[test]
    fn test_user_hook_type_parse() {
        assert_eq!(
            UserHookType::parse("PreToolUse"),
            Some(UserHookType::PreToolUse)
        );
        assert_eq!(
            UserHookType::parse("PostToolUse"),
            Some(UserHookType::PostToolUse)
        );
        assert_eq!(UserHookType::parse("Invalid"), None);
        assert_eq!(UserHookType::parse(""), None);
    }

    #[test]
    fn test_user_hook_matches_exact_tool() {
        let mut hook = create_test_hook(UserHookType::PreToolUse, "Write", "echo 'test'");

        assert!(hook.matches("Write"));
        assert!(!hook.matches("Read"));
        assert!(hook.matches("WriteFile"));
    }

    #[test]
    fn test_user_hook_matches_pattern() {
        let mut hook = create_test_hook(UserHookType::PreToolUse, "Write|Edit", "echo 'test'");

        assert!(hook.matches("Write"));
        assert!(hook.matches("Edit"));
        assert!(!hook.matches("Read"));
    }

    #[test]
    fn test_user_hook_matches_wildcard() {
        let mut hook = create_test_hook(UserHookType::PreToolUse, ".*", "echo 'test'");

        assert!(hook.matches("Write"));
        assert!(hook.matches("Read"));
        assert!(hook.matches("Bash"));
        assert!(hook.matches("AnyTool"));
    }

    #[test]
    fn test_user_hook_disabled_does_not_match() {
        let mut hook = create_test_hook(UserHookType::PreToolUse, "Write", "echo 'test'");
        hook.enabled = false;

        assert!(!hook.matches("Write"));
        assert!(!hook.matches("Read"));
    }

    #[test]
    fn test_user_hook_invalid_regex_pattern() {
        let mut hook = create_test_hook(UserHookType::PreToolUse, "[invalid", "echo 'test'");

        assert!(!hook.matches("Write"));
        assert!(!hook.matches("[invalid"));
        assert!(!hook.is_pattern_valid());
    }

    #[test]
    fn test_user_hook_valid_regex_pattern() {
        let hook = create_test_hook(UserHookType::PreToolUse, "Write.*", "echo 'test'");
        assert!(hook.is_pattern_valid());
    }

    #[test]
    fn test_user_hook_pattern_case_sensitive() {
        let mut hook = create_test_hook(UserHookType::PreToolUse, "write", "echo 'test'");

        assert!(!hook.matches("Write"));
        assert!(hook.matches("write"));
    }

    #[test]
    fn test_user_hook_complex_pattern() {
        let mut hook = create_test_hook(
            UserHookType::PreToolUse,
            r"^File(Read|Write|Edit)$",
            "echo 'test'",
        );

        assert!(hook.matches("FileRead"));
        assert!(hook.matches("FileWrite"));
        assert!(hook.matches("FileEdit"));
        assert!(!hook.matches("FileReadMore"));
        assert!(!hook.matches("MyFileRead"));
    }

    #[test]
    fn test_user_hook_lazy_compile() {
        let mut hook = UserHook {
            id: "test".to_string(),
            hook_type: UserHookType::PreToolUse,
            tool_pattern: "Write".to_string(),
            command: "echo 'test'".to_string(),
            enabled: true,
            timeout_seconds: DEFAULT_HOOK_TIMEOUT_SECONDS,
            created_at: chrono::Utc::now().to_rfc3339(),
            source: UserHookSource::User,
            owner_user_id: None,
            working_dir: None,
            compiled_pattern: None,
        };

        assert!(hook.matches("Write"));
        assert!(hook.compiled_pattern.is_some());
        assert!(hook.matches("Write"));
        assert!(!hook.matches("Read"));
    }
}
