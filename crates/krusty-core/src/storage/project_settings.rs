//! Per-project settings loaded from `.krusty/settings.json`.
//!
//! Provides override values that layer on top of global preferences
//! and session runtime options when working inside a project directory.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Per-project settings loaded from `.krusty/settings.json`.
///
/// All fields are optional — only specified values override the defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    /// Override model for this project (e.g. "claude-opus-4-6-20250320").
    pub model: Option<String>,

    /// Override permission mode: `"supervised"` or `"autonomous"`.
    pub permission_mode: Option<String>,

    /// Additional system prompt text appended to context injection.
    pub system_prompt_append: Option<String>,

    /// Max turns for subagents in this project.
    pub subagent_max_turns: Option<usize>,

    /// Custom conventions for builder agents.
    pub conventions: Option<Vec<String>>,

    /// Disable specific tools by name.
    pub disabled_tools: Option<Vec<String>>,
}

impl ProjectSettings {
    /// Load from `.krusty/settings.json` in the given directory.
    ///
    /// Returns `Default` if the file doesn't exist or can't be parsed,
    /// matching the graceful-degradation pattern used by MCP config loading.
    pub fn load(project_dir: &Path) -> Self {
        let path = project_dir.join(".krusty").join("settings.json");
        match std::fs::read_to_string(&path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse {}: {}", path.display(), e);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Check if any settings are configured (non-default).
    pub fn is_empty(&self) -> bool {
        self.model.is_none()
            && self.permission_mode.is_none()
            && self.system_prompt_append.is_none()
            && self.subagent_max_turns.is_none()
            && self.conventions.is_none()
            && self.disabled_tools.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn loads_from_krusty_settings_json() {
        let temp = TempDir::new().unwrap();
        let krusty_dir = temp.path().join(".krusty");
        fs::create_dir_all(&krusty_dir).unwrap();
        fs::write(
            krusty_dir.join("settings.json"),
            r#"{ "model": "claude-opus-4-6-20250320", "subagent_max_turns": 50 }"#,
        )
        .unwrap();

        let settings = ProjectSettings::load(temp.path());
        assert_eq!(settings.model.as_deref(), Some("claude-opus-4-6-20250320"));
        assert_eq!(settings.subagent_max_turns, Some(50));
        assert!(settings.permission_mode.is_none());
        assert!(!settings.is_empty());
    }

    #[test]
    fn returns_default_when_file_missing() {
        let temp = TempDir::new().unwrap();
        let settings = ProjectSettings::load(temp.path());
        assert!(settings.is_empty());
    }

    #[test]
    fn returns_default_on_invalid_json() {
        let temp = TempDir::new().unwrap();
        let krusty_dir = temp.path().join(".krusty");
        fs::create_dir_all(&krusty_dir).unwrap();
        fs::write(krusty_dir.join("settings.json"), "not valid json").unwrap();

        let settings = ProjectSettings::load(temp.path());
        assert!(settings.is_empty());
    }

    #[test]
    fn ignores_unknown_fields_gracefully() {
        let temp = TempDir::new().unwrap();
        let krusty_dir = temp.path().join(".krusty");
        fs::create_dir_all(&krusty_dir).unwrap();
        fs::write(
            krusty_dir.join("settings.json"),
            r#"{ "model": "gpt-5", "future_field": true }"#,
        )
        .unwrap();

        let settings = ProjectSettings::load(temp.path());
        assert_eq!(settings.model.as_deref(), Some("gpt-5"));
    }

    #[test]
    fn parses_all_fields() {
        let temp = TempDir::new().unwrap();
        let krusty_dir = temp.path().join(".krusty");
        fs::create_dir_all(&krusty_dir).unwrap();
        fs::write(
            krusty_dir.join("settings.json"),
            r#"{
                "model": "claude-opus-4-6-20250320",
                "permission_mode": "autonomous",
                "system_prompt_append": "Always use Rust idioms.",
                "subagent_max_turns": 100,
                "conventions": ["no-unwrap", "error-chain"],
                "disabled_tools": ["bash"]
            }"#,
        )
        .unwrap();

        let settings = ProjectSettings::load(temp.path());
        assert_eq!(settings.model.as_deref(), Some("claude-opus-4-6-20250320"));
        assert_eq!(settings.permission_mode.as_deref(), Some("autonomous"));
        assert_eq!(
            settings.system_prompt_append.as_deref(),
            Some("Always use Rust idioms.")
        );
        assert_eq!(settings.subagent_max_turns, Some(100));
        assert_eq!(
            settings.conventions.as_deref(),
            Some(&["no-unwrap".to_string(), "error-chain".to_string()][..])
        );
        assert_eq!(
            settings.disabled_tools.as_deref(),
            Some(&["bash".to_string()][..])
        );
    }
}
