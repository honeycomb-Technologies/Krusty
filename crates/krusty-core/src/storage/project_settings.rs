//! Per-project settings loaded from `.krusty/settings.json`.
//!
//! Provides override values that layer on top of global preferences
//! and session runtime options when working inside a project directory.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

use crate::agent::state::RunBudget;
use crate::ai::models::ProjectModelRef;

pub const DEFAULT_MAKO_TICK_INTERVAL_SECS: u64 = 30;
pub const MIN_MAKO_TICK_INTERVAL_SECS: u64 = 5;
pub const MAX_MAKO_TICK_INTERVAL_SECS: u64 = 86_400;
pub const DEFAULT_MAKO_MAX_TICKS: usize = 1_000;
pub const MAX_MAKO_MAX_TICKS: usize = 10_000;
pub const DEFAULT_MAKO_MAX_TURNS_PER_TICK: usize = 32;
pub const MAX_MAKO_MAX_TURNS_PER_TICK: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MakoSettingsError {
    #[error("Mako {field} must be between {minimum} and {maximum}, got {actual}")]
    OutOfRange {
        field: &'static str,
        minimum: u64,
        maximum: u64,
        actual: u64,
    },
}

/// Resolved Mako runtime settings after project overrides are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MakoSettings {
    pub tick_interval_secs: u64,
    pub max_ticks: usize,
    /// Hard parent-loop budget applied independently to every autonomous tick.
    pub max_turns_per_tick: usize,
}

impl Default for MakoSettings {
    fn default() -> Self {
        Self {
            tick_interval_secs: DEFAULT_MAKO_TICK_INTERVAL_SECS,
            max_ticks: DEFAULT_MAKO_MAX_TICKS,
            max_turns_per_tick: DEFAULT_MAKO_MAX_TURNS_PER_TICK,
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

    /// Maximum parent-agent model turns allowed within each autonomous tick.
    /// Subagents retain their own inherited, independently enforced budgets.
    pub max_turns_per_tick: Option<usize>,
}

/// Repository-owned restrictions for executable agent extensions after a
/// separate user-owned project trust grant. Patterns use the standard glob
/// grammar (`review-*`, `acme.*`, or `*`). Deny wins over allow; this structure
/// can never grant execution authority by itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProjectAgentExtensionSettings {
    /// Disable all executable extensions for this project when false.
    pub enabled: Option<bool>,
    /// Optional allowlist. An empty list allows every extension not denied.
    pub allow: Vec<String>,
    /// Explicit denylist evaluated before allow.
    pub deny: Vec<String>,
}

impl ProjectAgentExtensionSettings {
    fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.allow.is_empty() && self.deny.is_empty()
    }

    pub fn allows(&self, extension_id: &str) -> bool {
        if self.enabled == Some(false) {
            return false;
        }
        if self
            .deny
            .iter()
            .any(|pattern| extension_pattern_matches(pattern, extension_id))
        {
            return false;
        }
        self.allow.is_empty()
            || self
                .allow
                .iter()
                .any(|pattern| extension_pattern_matches(pattern, extension_id))
    }
}

fn extension_pattern_matches(pattern: &str, extension_id: &str) -> bool {
    glob::Pattern::new(pattern)
        .map(|pattern| pattern.matches(extension_id))
        .unwrap_or_else(|_| pattern == extension_id)
}

impl ProjectMakoSettings {
    fn is_empty(&self) -> bool {
        self.tick_interval_secs.is_none()
            && self.max_ticks.is_none()
            && self.max_turns_per_tick.is_none()
    }
}

/// Per-project settings loaded from `.krusty/settings.json`.
///
/// All fields are optional — only specified values override the defaults.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectSettings {
    /// Override model for this project. Legacy strings remain supported, while
    /// an exact key avoids cross-provider/auth/transport ambiguity.
    pub model: Option<ProjectModelRef>,

    /// Override permission mode: `"supervised"` or `"autonomous"`.
    pub permission_mode: Option<String>,

    /// Additional system prompt text appended to context injection.
    pub system_prompt_append: Option<String>,

    /// Max turns for subagents in this project.
    pub subagent_max_turns: Option<usize>,

    /// Optional parent-run resource limits for this project.
    /// Omit `max_turns` (or this object) for unlimited interactive runs.
    #[serde(alias = "run_budget")]
    pub run_limits: Option<RunBudget>,

    /// Custom conventions for builder agents.
    pub conventions: Option<Vec<String>>,

    /// Disable specific tools by name.
    pub disabled_tools: Option<Vec<String>>,

    /// Optional Mako-specific cadence settings.
    pub mako: Option<ProjectMakoSettings>,

    /// Trust/enablement policy for executable agent extensions.
    #[serde(alias = "agentExtensions")]
    pub agent_extensions: Option<ProjectAgentExtensionSettings>,
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
            && self.run_limits.is_none()
            && self.conventions.is_none()
            && self.disabled_tools.is_none()
            && self.mako.as_ref().is_none_or(ProjectMakoSettings::is_empty)
            && self
                .agent_extensions
                .as_ref()
                .is_none_or(ProjectAgentExtensionSettings::is_empty)
    }

    /// Resolve Mako settings for read-only/status surfaces.
    ///
    /// Invalid explicit overrides never escape as unbounded work. They are
    /// surfaced in logs and replaced wholesale with the finite safe defaults.
    pub fn mako_settings(&self) -> MakoSettings {
        match self.mako_settings_checked() {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(error = %error, "Rejecting invalid project Mako settings");
                MakoSettings::default()
            }
        }
    }

    /// Resolve Mako settings for execution. Explicit invalid or oversized
    /// values fail closed instead of being ignored, clamped, or made unbounded.
    pub fn mako_settings_checked(&self) -> Result<MakoSettings, MakoSettingsError> {
        let Some(mako) = &self.mako else {
            return Ok(MakoSettings::default());
        };

        let tick_interval_secs = checked_u64(
            "tick_interval_secs",
            mako.tick_interval_secs
                .unwrap_or(DEFAULT_MAKO_TICK_INTERVAL_SECS),
            MIN_MAKO_TICK_INTERVAL_SECS,
            MAX_MAKO_TICK_INTERVAL_SECS,
        )?;
        let max_ticks = checked_usize(
            "max_ticks",
            mako.max_ticks.unwrap_or(DEFAULT_MAKO_MAX_TICKS),
            1,
            MAX_MAKO_MAX_TICKS,
        )?;
        let max_turns_per_tick = checked_usize(
            "max_turns_per_tick",
            mako.max_turns_per_tick
                .unwrap_or(DEFAULT_MAKO_MAX_TURNS_PER_TICK),
            1,
            MAX_MAKO_MAX_TURNS_PER_TICK,
        )?;

        Ok(MakoSettings {
            tick_interval_secs,
            max_ticks,
            max_turns_per_tick,
        })
    }

    /// Load resolved Mako cadence settings directly from the active project directory.
    pub fn load_mako_settings(project_dir: Option<&Path>) -> MakoSettings {
        project_dir
            .map(Self::load)
            .unwrap_or_default()
            .mako_settings()
    }

    pub fn allows_agent_extension(&self, extension_id: &str) -> bool {
        self.agent_extensions
            .as_ref()
            .is_none_or(|settings| settings.allows(extension_id))
    }

    /// Load Mako settings for an execution boundary. Unlike the status helper,
    /// this preserves validation failure so the caller can refuse the run.
    pub fn load_mako_settings_checked(
        project_dir: Option<&Path>,
    ) -> Result<MakoSettings, MakoSettingsError> {
        project_dir
            .map(Self::load)
            .unwrap_or_default()
            .mako_settings_checked()
    }
}

fn checked_u64(
    field: &'static str,
    actual: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, MakoSettingsError> {
    if (minimum..=maximum).contains(&actual) {
        Ok(actual)
    } else {
        Err(MakoSettingsError::OutOfRange {
            field,
            minimum,
            maximum,
            actual,
        })
    }
}

fn checked_usize(
    field: &'static str,
    actual: usize,
    minimum: usize,
    maximum: usize,
) -> Result<usize, MakoSettingsError> {
    if (minimum..=maximum).contains(&actual) {
        Ok(actual)
    } else {
        Err(MakoSettingsError::OutOfRange {
            field,
            minimum: minimum as u64,
            maximum: maximum as u64,
            actual: u64::try_from(actual).unwrap_or(u64::MAX),
        })
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
        assert_eq!(
            settings.model.as_ref().map(ProjectModelRef::model_id),
            Some("claude-opus-4-6-20250320")
        );
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
        assert_eq!(
            settings.model.as_ref().map(ProjectModelRef::model_id),
            Some("gpt-5")
        );
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
                "run_limits": { "max_turns": 75 },
                "conventions": ["no-unwrap", "error-chain"],
                "disabled_tools": ["bash"],
                "mako": {
                    "tick_interval_secs": 20,
                    "max_ticks": 200,
                    "max_turns_per_tick": 24
                }
            }"#,
        )
        .unwrap();

        let settings = ProjectSettings::load(temp.path());
        assert_eq!(
            settings.model.as_ref().map(ProjectModelRef::model_id),
            Some("claude-opus-4-6-20250320")
        );
        assert_eq!(settings.permission_mode.as_deref(), Some("autonomous"));
        assert_eq!(
            settings.system_prompt_append.as_deref(),
            Some("Always use Rust idioms.")
        );
        assert_eq!(settings.subagent_max_turns, Some(100));
        assert_eq!(settings.run_limits, Some(RunBudget::with_max_turns(75)));
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
        assert_eq!(settings.mako_settings().max_turns_per_tick, 24);
    }

    #[test]
    fn mako_settings_return_defaults_when_missing() {
        let settings = ProjectSettings::default();

        assert_eq!(settings.mako_settings(), MakoSettings::default());
    }

    #[test]
    fn parses_exact_project_model_key_without_breaking_legacy_strings() {
        let legacy: ProjectSettings = serde_json::from_str(r#"{ "model": "grok-4.5" }"#).unwrap();
        assert_eq!(
            legacy.model,
            Some(ProjectModelRef::Legacy("grok-4.5".to_string()))
        );

        let exact: ProjectSettings = serde_json::from_str(
            r#"{
                "model": {
                    "provider": "grok",
                    "model_id": "grok-4.5",
                    "api_format": "open_ai_responses"
                }
            }"#,
        )
        .unwrap();
        let key = exact.model.as_ref().and_then(ProjectModelRef::exact_key);
        assert_eq!(
            key.map(|key| key.provider),
            Some(crate::ai::providers::ProviderId::Grok)
        );
        assert_eq!(key.map(|key| key.model_id.as_str()), Some("grok-4.5"));
    }

    #[test]
    fn mako_execution_settings_reject_zero_values() {
        for mako in [
            ProjectMakoSettings {
                tick_interval_secs: Some(0),
                ..Default::default()
            },
            ProjectMakoSettings {
                max_ticks: Some(0),
                ..Default::default()
            },
            ProjectMakoSettings {
                max_turns_per_tick: Some(0),
                ..Default::default()
            },
        ] {
            let settings = ProjectSettings {
                mako: Some(mako),
                ..Default::default()
            };
            assert!(settings.mako_settings_checked().is_err());
            assert_eq!(settings.mako_settings(), MakoSettings::default());
        }
    }

    #[test]
    fn mako_execution_settings_accept_hard_upper_bounds() {
        let settings = ProjectSettings {
            mako: Some(ProjectMakoSettings {
                tick_interval_secs: Some(MAX_MAKO_TICK_INTERVAL_SECS),
                max_ticks: Some(MAX_MAKO_MAX_TICKS),
                max_turns_per_tick: Some(MAX_MAKO_MAX_TURNS_PER_TICK),
            }),
            ..Default::default()
        };

        assert_eq!(
            settings.mako_settings_checked().unwrap(),
            MakoSettings {
                tick_interval_secs: MAX_MAKO_TICK_INTERVAL_SECS,
                max_ticks: MAX_MAKO_MAX_TICKS,
                max_turns_per_tick: MAX_MAKO_MAX_TURNS_PER_TICK,
            }
        );
    }

    #[test]
    fn mako_execution_settings_reject_values_above_hard_bounds() {
        for mako in [
            ProjectMakoSettings {
                tick_interval_secs: Some(MAX_MAKO_TICK_INTERVAL_SECS + 1),
                ..Default::default()
            },
            ProjectMakoSettings {
                max_ticks: Some(MAX_MAKO_MAX_TICKS + 1),
                ..Default::default()
            },
            ProjectMakoSettings {
                max_turns_per_tick: Some(MAX_MAKO_MAX_TURNS_PER_TICK + 1),
                ..Default::default()
            },
        ] {
            let settings = ProjectSettings {
                mako: Some(mako),
                ..Default::default()
            };
            assert!(settings.mako_settings_checked().is_err());
            assert_eq!(settings.mako_settings(), MakoSettings::default());
        }
    }

    #[test]
    fn mako_default_parent_turn_budget_is_finite_and_bounded() {
        let settings = MakoSettings::default();
        assert!(settings.max_turns_per_tick > 0);
        assert!(settings.max_turns_per_tick <= MAX_MAKO_MAX_TURNS_PER_TICK);
        assert!(settings.max_ticks <= MAX_MAKO_MAX_TICKS);
        assert!(settings.tick_interval_secs <= MAX_MAKO_TICK_INTERVAL_SECS);
    }

    #[test]
    fn agent_extension_policy_supports_allow_and_deny_globs() {
        let settings = ProjectAgentExtensionSettings {
            enabled: Some(true),
            allow: vec!["acme-*".to_string()],
            deny: vec!["acme-dangerous".to_string()],
        };

        assert!(settings.allows("acme-review"));
        assert!(!settings.allows("acme-dangerous"));
        assert!(!settings.allows("other"));
    }
}
