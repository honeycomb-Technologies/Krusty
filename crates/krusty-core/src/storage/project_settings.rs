//! Per-project settings loaded from `.krusty/settings.json`.
//!
//! Provides override values that layer on top of global preferences
//! and session runtime options when working inside a project directory.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Resolved Mako cadence settings after project overrides are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MakoSettings {
    pub tick_interval_secs: u64,
    pub max_ticks: usize,
}

impl Default for MakoSettings {
    fn default() -> Self {
        Self {
            tick_interval_secs: 30,
            max_ticks: 1000,
        }
    }
}

/// Optional project-level overrides for Mako cadence behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProjectMakoSettings {
    /// Seconds between autonomous wake ticks while Mako is active.
    pub tick_interval_secs: Option<u64>,

    /// Maximum autonomous wake ticks to execute before stopping.
    pub max_ticks: Option<usize>,
}

impl ProjectMakoSettings {
    fn is_empty(&self) -> bool {
        self.tick_interval_secs.is_none() && self.max_ticks.is_none()
    }
}

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

    /// Optional Mako-specific cadence settings.
    pub mako: Option<ProjectMakoSettings>,
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
            && self.mako.as_ref().is_none_or(ProjectMakoSettings::is_empty)
    }

    /// Resolve Mako cadence settings with defaults and basic zero-value rejection.
    pub fn mako_settings(&self) -> MakoSettings {
        let mut resolved = MakoSettings::default();

        if let Some(mako) = &self.mako {
            if let Some(tick_interval_secs) = mako.tick_interval_secs.filter(|secs| *secs > 0) {
                resolved.tick_interval_secs = tick_interval_secs;
            }
            if let Some(max_ticks) = mako.max_ticks.filter(|max_ticks| *max_ticks > 0) {
                resolved.max_ticks = max_ticks;
            }
        }

        resolved
    }

    /// Load resolved Mako cadence settings directly from the active project directory.
    pub fn load_mako_settings(project_dir: Option<&Path>) -> MakoSettings {
        project_dir
            .map(Self::load)
            .unwrap_or_default()
            .mako_settings()
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
            r#"{ "model": "claude-opus-4-6-20250320", "subagent_max_turns": 50, "mako": { "tick_interval_secs": 45 } }"#,
        )
        .unwrap();

        let settings = ProjectSettings::load(temp.path());
        assert_eq!(settings.model.as_deref(), Some("claude-opus-4-6-20250320"));
        assert_eq!(settings.subagent_max_turns, Some(50));
        assert_eq!(
            settings
                .mako
                .as_ref()
                .and_then(|mako| mako.tick_interval_secs),
            Some(45)
        );
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
                "disabled_tools": ["bash"],
                "mako": {
                    "tick_interval_secs": 20,
                    "max_ticks": 200
                }
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
        assert_eq!(settings.mako_settings().tick_interval_secs, 20);
        assert_eq!(settings.mako_settings().max_ticks, 200);
    }

    #[test]
    fn mako_settings_return_defaults_when_missing() {
        let settings = ProjectSettings::default();

        assert_eq!(settings.mako_settings(), MakoSettings::default());
    }

    #[test]
    fn mako_settings_ignore_zero_values() {
        let settings = ProjectSettings {
            mako: Some(ProjectMakoSettings {
                tick_interval_secs: Some(0),
                max_ticks: Some(0),
            }),
            ..Default::default()
        };

        assert_eq!(settings.mako_settings(), MakoSettings::default());
    }
}
