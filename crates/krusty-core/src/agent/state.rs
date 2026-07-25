//! Agent state tracking
//!
//! Tracks turn count and timing for safety limits.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::constants;

/// Optional resource limits for one parent agent run.
///
/// A missing value is deliberately unlimited. Behavioral loop protection is
/// handled by the semantic progress ledger rather than by an arbitrary turn
/// ceiling.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RunBudget {
    /// Maximum provider turns in this run (`None` = unlimited).
    pub max_turns: Option<usize>,
}

impl RunBudget {
    pub const fn unlimited() -> Self {
        Self { max_turns: None }
    }

    pub const fn with_max_turns(max_turns: usize) -> Self {
        Self {
            max_turns: Some(max_turns),
        }
    }

    pub fn is_exhausted(self, completed_turns: usize) -> bool {
        self.max_turns
            .is_some_and(|max_turns| completed_turns >= max_turns)
    }
}

/// Provenance for the effective run budget recorded in runtime telemetry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunBudgetSource {
    /// Typed per-run override supplied by the caller.
    ExplicitRun,
    /// Historical trace value retained for backward-compatible replay. New
    /// runs resolve bounded callers through `ExplicitRun`.
    LegacyMaxIterations,
    /// Repository-owned `.krusty/settings.json` override.
    ProjectSettings,
    /// No configured ceiling; primary interactive execution is unlimited.
    UnlimitedDefault,
}

/// The canonical effective budget and where it came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunBudgetResolution {
    pub budget: RunBudget,
    pub source: RunBudgetSource,
}

impl RunBudgetResolution {
    /// Resolve exactly once at the core execution boundary.
    ///
    /// Per-run overrides are strongest, followed by repository policy and the
    /// unlimited interactive default.
    pub fn resolve(explicit_run: Option<RunBudget>, project: Option<RunBudget>) -> Self {
        if let Some(budget) = explicit_run {
            return Self {
                budget,
                source: RunBudgetSource::ExplicitRun,
            };
        }
        if let Some(budget) = project {
            return Self {
                budget,
                source: RunBudgetSource::ProjectSettings,
            };
        }

        Self {
            budget: RunBudget::unlimited(),
            source: RunBudgetSource::UnlimitedDefault,
        }
    }
}

/// Runtime state of the agent
#[derive(Debug, Default)]
pub struct AgentState {
    /// Current turn number (increments each time we send to AI)
    pub current_turn: usize,
    /// When the current turn started
    pub turn_start: Option<Instant>,
    /// Whether the agent was interrupted
    pub is_interrupted: bool,
}

impl AgentState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new turn
    pub fn start_turn(&mut self) {
        self.current_turn += 1;
        self.turn_start = Some(Instant::now());
        self.is_interrupted = false;
    }

    /// Get duration of current turn
    pub fn turn_duration(&self) -> Option<Duration> {
        self.turn_start.map(|start| start.elapsed())
    }

    /// Mark as interrupted
    pub fn interrupt(&mut self) {
        self.is_interrupted = true;
        self.turn_start = None;
    }

    /// Reset all per-session state.
    pub fn reset(&mut self) {
        self.current_turn = 0;
        self.turn_start = None;
        self.is_interrupted = false;
    }
}

/// Configuration for agent behavior
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Legacy alias for the primary-session turn budget.
    /// `None` means unlimited.
    pub max_turns: Option<usize>,
    /// Maximum turns for primary interactive sessions (None = unlimited).
    pub primary_max_turns: Option<usize>,
    /// Maximum turns for sub-agents (None = unlimited).
    pub subagent_max_turns: Option<usize>,
    /// Maximum turns for ACP sessions (None = unlimited).
    pub acp_max_turns: Option<usize>,
    /// Idle stream timeout in seconds.
    pub stream_idle_timeout_secs: u64,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: None,
            primary_max_turns: None,
            subagent_max_turns: None,
            acp_max_turns: None,
            stream_idle_timeout_secs: constants::http::STREAM_TIMEOUT.as_secs(),
        }
    }
}

impl AgentConfig {
    /// Resolve the primary-session turn budget, honoring the legacy alias.
    pub fn primary_max_turns(&self) -> Option<usize> {
        self.primary_max_turns.or(self.max_turns)
    }

    /// Typed primary-run override for the canonical core resolver.
    ///
    /// `None` means there is no caller override, allowing project settings or
    /// the unlimited default to resolve at the execution boundary.
    pub fn primary_run_budget_override(&self) -> Option<RunBudget> {
        self.primary_max_turns().map(RunBudget::with_max_turns)
    }

    /// Resolve the ACP turn budget, honoring the legacy alias when no explicit ACP budget exists.
    pub fn acp_max_turns(&self) -> Option<usize> {
        self.acp_max_turns.or(self.max_turns)
    }

    /// Resolve the configured idle stream timeout.
    pub fn stream_idle_timeout(&self) -> Duration {
        Duration::from_secs(self.stream_idle_timeout_secs.max(1))
    }

    /// Check if we've exceeded the primary-session turn budget.
    pub fn exceeded_max_turns(&self, current_turn: usize) -> bool {
        self.primary_max_turns()
            .is_some_and(|max| current_turn >= max)
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentConfig, AgentState, RunBudget, RunBudgetResolution, RunBudgetSource};

    #[test]
    fn state_reset_clears_turn_tracking() {
        let mut state = AgentState::new();
        state.start_turn();
        state.interrupt();
        state.reset();

        assert_eq!(state.current_turn, 0);
        assert!(state.turn_start.is_none());
        assert!(!state.is_interrupted);
    }

    #[test]
    fn default_config_leaves_primary_and_acp_unlimited() {
        let config = AgentConfig::default();

        assert_eq!(config.primary_max_turns(), None);
        assert_eq!(config.primary_run_budget_override(), None);
        assert_eq!(config.acp_max_turns(), None);
        assert_eq!(config.subagent_max_turns, None);
        assert!(config.stream_idle_timeout().as_secs() >= 600);
    }

    #[test]
    fn unlimited_primary_budget_allows_progress_beyond_fifty_turns() {
        let resolution = RunBudgetResolution::resolve(None, None);

        assert_eq!(resolution.source, RunBudgetSource::UnlimitedDefault);
        for completed_turns in 0..=100 {
            assert!(!resolution.budget.is_exhausted(completed_turns));
        }
    }

    #[test]
    fn run_budget_resolution_preserves_precedence_and_provenance() {
        let project = RunBudget::with_max_turns(80);
        let project_resolution = RunBudgetResolution::resolve(None, Some(project));
        assert_eq!(project_resolution.budget.max_turns, Some(80));
        assert_eq!(project_resolution.source, RunBudgetSource::ProjectSettings);

        let explicit = RunBudgetResolution::resolve(Some(RunBudget::unlimited()), Some(project));
        assert_eq!(explicit.budget.max_turns, None);
        assert_eq!(explicit.source, RunBudgetSource::ExplicitRun);
    }
}
