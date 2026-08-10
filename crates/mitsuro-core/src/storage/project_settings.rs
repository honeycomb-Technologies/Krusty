//! Per-project settings loaded from `.mitsuro/settings.json`.
//!
//! Provides override values that layer on top of global preferences
//! and session runtime options when working inside a project directory.

use serde::{Deserialize, Serialize};
use std::path::Path;
use thiserror::Error;

use crate::agent::state::RunBudget;
use crate::ai::models::ProjectModelRef;

pub const DEFAULT_HIVE_TICK_INTERVAL_SECS: u64 = 30;
pub const MIN_HIVE_TICK_INTERVAL_SECS: u64 = 5;
pub const MAX_HIVE_TICK_INTERVAL_SECS: u64 = 86_400;
pub const DEFAULT_HIVE_MAX_TICKS: usize = 1_000;
pub const MAX_HIVE_MAX_TICKS: usize = 10_000;
pub const DEFAULT_HIVE_MAX_TURNS_PER_TICK: usize = 32;
pub const MAX_HIVE_MAX_TURNS_PER_TICK: usize = 128;

/// How strongly the primary model should prefer delegated execution.
///
/// This controls model guidance only. The core remains the authority for
/// permissions, tool scope, budgets, concurrency, and recursion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationMode {
    ExplicitOnly,
    #[default]
    Balanced,
    Proactive,
    Orchestrator,
}

impl DelegationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitOnly => "explicit_only",
            Self::Balanced => "balanced",
            Self::Proactive => "proactive",
            Self::Orchestrator => "orchestrator",
        }
    }

    pub fn prompt_contract(self) -> String {
        let guidance = match self {
            Self::ExplicitOnly => {
                "Use the agent tool only when the user explicitly requests delegation or asks to continue an existing delegated run. Otherwise work directly."
            }
            Self::Balanced => {
                "Delegate substantial independent work when parallelism, a fresh focused context, or background execution clearly improves the outcome. Work directly for simple, tightly coupled, or sequential tasks."
            }
            Self::Proactive => {
                "Actively identify substantial independent work that benefits from parallel agents, fresh context, or background execution. Do not delegate trivial, tightly coupled, or coordination-heavy work."
            }
            Self::Orchestrator => {
                "For substantial decomposable objectives, coordinate through focused agents early. Keep tightly coupled decisions in the parent, avoid duplicate work, and do not delegate trivial actions."
            }
        };

        format!(
            "[DELEGATION MODE: {}]\n{} The parent must coordinate, inspect evidence, and verify delegated results. For one decomposable operation, prefer one agent spawn with a structured tasks graph over several separate spawn calls: give every task a stable id, bounded instructions, minimum capabilities, scope, declared write_intent, and real depends_on edges. Independent ready tasks may run concurrently. Tasks that must consume another task's edits, or that intentionally touch the same mutable files, must be dependency-ordered instead of described as parallel. When setting max_turns, include a final handoff turn after expected inspect, edit, and verification phases; multi-step writers normally need at least 4 turns unless the inherited ceiling is lower. Do not create multiple agents merely to multiply activity.",
            self.as_str().to_ascii_uppercase(),
            guidance
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HiveSettingsError {
    #[error("Hive {field} must be between {minimum} and {maximum}, got {actual}")]
    OutOfRange {
        field: &'static str,
        minimum: u64,
        maximum: u64,
        actual: u64,
    },
}

/// Resolved Hive runtime settings after project overrides are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HiveSettings {
    pub tick_interval_secs: u64,
    pub max_ticks: usize,
    /// Hard parent-loop budget applied independently to every autonomous tick.
    pub max_turns_per_tick: usize,
}

impl Default for HiveSettings {
    fn default() -> Self {
        Self {
            tick_interval_secs: DEFAULT_HIVE_TICK_INTERVAL_SECS,
            max_ticks: DEFAULT_HIVE_MAX_TICKS,
            max_turns_per_tick: DEFAULT_HIVE_MAX_TURNS_PER_TICK,
        }
    }
}

/// Optional project-level overrides for Hive cadence behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProjectHiveSettings {
    /// Seconds between autonomous wake ticks while Hive is active.
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

impl ProjectHiveSettings {
    fn is_empty(&self) -> bool {
        self.tick_interval_secs.is_none()
            && self.max_ticks.is_none()
            && self.max_turns_per_tick.is_none()
    }
}

/// Per-project settings loaded from `.mitsuro/settings.json`.
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

    /// Primary-agent delegation preference. Defaults to balanced.
    pub delegation_mode: Option<DelegationMode>,

    /// Optional parent-run resource limits for this project.
    /// Omit `max_turns` (or this object) for unlimited interactive runs.
    #[serde(alias = "run_budget")]
    pub run_limits: Option<RunBudget>,

    /// Custom conventions for builder agents.
    pub conventions: Option<Vec<String>>,

    /// Disable specific tools by name.
    pub disabled_tools: Option<Vec<String>>,

    /// Optional Hive-specific cadence settings.
    #[serde(alias = "mako")]
    pub hive: Option<ProjectHiveSettings>,

    /// Trust/enablement policy for executable agent extensions.
    #[serde(alias = "agentExtensions")]
    pub agent_extensions: Option<ProjectAgentExtensionSettings>,
}

impl ProjectSettings {
    /// Load from `.mitsuro/settings.json` in the given directory.
    ///
    /// Returns `Default` if the file doesn't exist or can't be parsed,
    /// matching the graceful-degradation pattern used by MCP config loading.
    pub fn load(project_dir: &Path) -> Self {
        let path = project_dir.join(".mitsuro").join("settings.json");
        let legacy_path =
            crate::identity::legacy_project_state_dir(project_dir).join("settings.json");
        let read_path = if path.is_file() { &path } else { &legacy_path };
        match std::fs::read_to_string(read_path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse {}: {}", read_path.display(), e);
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
            && self.delegation_mode.is_none()
            && self.run_limits.is_none()
            && self.conventions.is_none()
            && self.disabled_tools.is_none()
            && self.hive.as_ref().is_none_or(ProjectHiveSettings::is_empty)
            && self
                .agent_extensions
                .as_ref()
                .is_none_or(ProjectAgentExtensionSettings::is_empty)
    }

    /// Resolve Hive settings for read-only/status surfaces.
    ///
    /// Invalid explicit overrides never escape as unbounded work. They are
    /// surfaced in logs and replaced wholesale with the finite safe defaults.
    pub fn hive_settings(&self) -> HiveSettings {
        match self.hive_settings_checked() {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(error = %error, "Rejecting invalid project Hive settings");
                HiveSettings::default()
            }
        }
    }

    /// Resolve Hive settings for execution. Explicit invalid or oversized
    /// values fail closed instead of being ignored, clamped, or made unbounded.
    pub fn hive_settings_checked(&self) -> Result<HiveSettings, HiveSettingsError> {
        let Some(hive) = &self.hive else {
            return Ok(HiveSettings::default());
        };

        let tick_interval_secs = checked_u64(
            "tick_interval_secs",
            hive.tick_interval_secs
                .unwrap_or(DEFAULT_HIVE_TICK_INTERVAL_SECS),
            MIN_HIVE_TICK_INTERVAL_SECS,
            MAX_HIVE_TICK_INTERVAL_SECS,
        )?;
        let max_ticks = checked_usize(
            "max_ticks",
            hive.max_ticks.unwrap_or(DEFAULT_HIVE_MAX_TICKS),
            1,
            MAX_HIVE_MAX_TICKS,
        )?;
        let max_turns_per_tick = checked_usize(
            "max_turns_per_tick",
            hive.max_turns_per_tick
                .unwrap_or(DEFAULT_HIVE_MAX_TURNS_PER_TICK),
            1,
            MAX_HIVE_MAX_TURNS_PER_TICK,
        )?;

        Ok(HiveSettings {
            tick_interval_secs,
            max_ticks,
            max_turns_per_tick,
        })
    }

    /// Load resolved Hive cadence settings directly from the active project directory.
    pub fn load_hive_settings(project_dir: Option<&Path>) -> HiveSettings {
        project_dir
            .map(Self::load)
            .unwrap_or_default()
            .hive_settings()
    }

    pub fn allows_agent_extension(&self, extension_id: &str) -> bool {
        self.agent_extensions
            .as_ref()
            .is_none_or(|settings| settings.allows(extension_id))
    }

    /// Load Hive settings for an execution boundary. Unlike the status helper,
    /// this preserves validation failure so the caller can refuse the run.
    pub fn load_hive_settings_checked(
        project_dir: Option<&Path>,
    ) -> Result<HiveSettings, HiveSettingsError> {
        project_dir
            .map(Self::load)
            .unwrap_or_default()
            .hive_settings_checked()
    }
}

fn checked_u64(
    field: &'static str,
    actual: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64, HiveSettingsError> {
    if (minimum..=maximum).contains(&actual) {
        Ok(actual)
    } else {
        Err(HiveSettingsError::OutOfRange {
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
) -> Result<usize, HiveSettingsError> {
    if (minimum..=maximum).contains(&actual) {
        Ok(actual)
    } else {
        Err(HiveSettingsError::OutOfRange {
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
    fn delegation_guidance_budgets_child_finalization() {
        let contract = DelegationMode::Orchestrator.prompt_contract();
        assert!(contract.contains("final handoff turn"));
        assert!(contract.contains("at least 4 turns"));
    }

    #[test]
    fn loads_from_mitsuro_settings_json() {
        let temp = TempDir::new().unwrap();
        let mitsuro_dir = temp.path().join(".mitsuro");
        fs::create_dir_all(&mitsuro_dir).unwrap();
        fs::write(
            mitsuro_dir.join("settings.json"),
            r#"{ "model": "claude-opus-4-6-20250320", "subagent_max_turns": 50, "hive": { "tick_interval_secs": 45 } }"#,
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
                .hive
                .as_ref()
                .and_then(|hive| hive.tick_interval_secs),
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
        let mitsuro_dir = temp.path().join(".mitsuro");
        fs::create_dir_all(&mitsuro_dir).unwrap();
        fs::write(mitsuro_dir.join("settings.json"), "not valid json").unwrap();

        let settings = ProjectSettings::load(temp.path());
        assert!(settings.is_empty());
    }

    #[test]
    fn ignores_unknown_fields_gracefully() {
        let temp = TempDir::new().unwrap();
        let mitsuro_dir = temp.path().join(".mitsuro");
        fs::create_dir_all(&mitsuro_dir).unwrap();
        fs::write(
            mitsuro_dir.join("settings.json"),
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
        let mitsuro_dir = temp.path().join(".mitsuro");
        fs::create_dir_all(&mitsuro_dir).unwrap();
        fs::write(
            mitsuro_dir.join("settings.json"),
            r#"{
                "model": "claude-opus-4-6-20250320",
                "permission_mode": "autonomous",
                "system_prompt_append": "Always use Rust idioms.",
                "subagent_max_turns": 100,
                "run_limits": { "max_turns": 75 },
                "conventions": ["no-unwrap", "error-chain"],
                "disabled_tools": ["bash"],
                "hive": {
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
        assert_eq!(settings.hive_settings().tick_interval_secs, 20);
        assert_eq!(settings.hive_settings().max_ticks, 200);
        assert_eq!(settings.hive_settings().max_turns_per_tick, 24);
    }

    #[test]
    fn hive_settings_return_defaults_when_missing() {
        let settings = ProjectSettings::default();

        assert_eq!(settings.hive_settings(), HiveSettings::default());
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
    fn hive_execution_settings_reject_zero_values() {
        for hive in [
            ProjectHiveSettings {
                tick_interval_secs: Some(0),
                ..Default::default()
            },
            ProjectHiveSettings {
                max_ticks: Some(0),
                ..Default::default()
            },
            ProjectHiveSettings {
                max_turns_per_tick: Some(0),
                ..Default::default()
            },
        ] {
            let settings = ProjectSettings {
                hive: Some(hive),
                ..Default::default()
            };
            assert!(settings.hive_settings_checked().is_err());
            assert_eq!(settings.hive_settings(), HiveSettings::default());
        }
    }

    #[test]
    fn hive_execution_settings_accept_hard_upper_bounds() {
        let settings = ProjectSettings {
            hive: Some(ProjectHiveSettings {
                tick_interval_secs: Some(MAX_HIVE_TICK_INTERVAL_SECS),
                max_ticks: Some(MAX_HIVE_MAX_TICKS),
                max_turns_per_tick: Some(MAX_HIVE_MAX_TURNS_PER_TICK),
            }),
            ..Default::default()
        };

        assert_eq!(
            settings.hive_settings_checked().unwrap(),
            HiveSettings {
                tick_interval_secs: MAX_HIVE_TICK_INTERVAL_SECS,
                max_ticks: MAX_HIVE_MAX_TICKS,
                max_turns_per_tick: MAX_HIVE_MAX_TURNS_PER_TICK,
            }
        );
    }

    #[test]
    fn hive_execution_settings_reject_values_above_hard_bounds() {
        for hive in [
            ProjectHiveSettings {
                tick_interval_secs: Some(MAX_HIVE_TICK_INTERVAL_SECS + 1),
                ..Default::default()
            },
            ProjectHiveSettings {
                max_ticks: Some(MAX_HIVE_MAX_TICKS + 1),
                ..Default::default()
            },
            ProjectHiveSettings {
                max_turns_per_tick: Some(MAX_HIVE_MAX_TURNS_PER_TICK + 1),
                ..Default::default()
            },
        ] {
            let settings = ProjectSettings {
                hive: Some(hive),
                ..Default::default()
            };
            assert!(settings.hive_settings_checked().is_err());
            assert_eq!(settings.hive_settings(), HiveSettings::default());
        }
    }

    #[test]
    fn hive_default_parent_turn_budget_is_finite_and_bounded() {
        let settings = HiveSettings::default();
        assert!(settings.max_turns_per_tick > 0);
        assert!(settings.max_turns_per_tick <= MAX_HIVE_MAX_TURNS_PER_TICK);
        assert!(settings.max_ticks <= MAX_HIVE_MAX_TICKS);
        assert!(settings.tick_interval_secs <= MAX_HIVE_TICK_INTERVAL_SECS);
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
