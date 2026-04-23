use regex::Regex;
use serde::{Deserialize, Serialize};

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
    /// When the hook was created.
    pub created_at: String,
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
            created_at: chrono::Utc::now().to_rfc3339(),
            compiled_pattern: compiled,
        }
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
    use super::{UserHook, UserHookType};

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
            created_at: chrono::Utc::now().to_rfc3339(),
            compiled_pattern: None,
        };

        assert!(hook.matches("Write"));
        assert!(hook.compiled_pattern.is_some());
        assert!(hook.matches("Write"));
        assert!(!hook.matches("Read"));
    }
}
