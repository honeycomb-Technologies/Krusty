use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapability {
    Read,
    Write,
    Execute,
}

impl AgentCapability {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "execute" => Ok(Self::Execute),
            other => Err(format!(
                "Unknown agent capability '{other}'. Use read, write, or execute"
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentContextMode {
    #[default]
    Auto,
    Project,
    Brief,
    Full,
}

impl AgentContextMode {
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        match value.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "project" | "fresh" | "none" => Ok(Self::Project),
            "brief" | "recent" => Ok(Self::Brief),
            "full" | "all" => Ok(Self::Full),
            other => Err(format!(
                "Unknown agent context '{other}'. Use auto, project, brief, or full"
            )),
        }
    }
}

/// Runtime engine for a delegated child. Product surface is an agnostic child
/// directed by parent instructions; this enum only selects tool capability class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentExecutionProfile {
    /// Read-focused worker (optional execute when capability requests it).
    Explore,
    /// Write-capable worker.
    Build,
}

impl AgentExecutionProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explore => "explore",
            Self::Build => "build",
        }
    }
}

/// Provider-neutral task specification proposed by the primary model.
///
/// Children are agnostic workers. `profile` is an optional label (legacy
/// explore/plan/verify/build names map to capability defaults only). Behavior
/// comes from parent `objective` / instructions. Capabilities are requests
/// still clamped by the parent's immutable DelegationPolicy at execution time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSpec {
    pub task_name: String,
    pub objective: String,
    pub expected_output: Option<String>,
    pub delegation_reason: Option<String>,
    pub profile: String,
    pub capabilities: BTreeSet<AgentCapability>,
    pub context: AgentContextMode,
    pub max_turns: Option<usize>,
}

impl AgentSpec {
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        profile: Option<&str>,
        legacy_agent_type: Option<&str>,
        task_name: Option<&str>,
        objective: &str,
        expected_output: Option<&str>,
        delegation_reason: Option<&str>,
        capabilities: &[String],
        context: Option<&str>,
        requested_max_turns: Option<usize>,
        inherited_max_turns: Option<usize>,
    ) -> Result<Self, String> {
        let objective = objective.trim();
        if objective.is_empty() {
            return Err("Agent spawn requires a non-empty prompt/objective".to_string());
        }

        let has_legacy_identity = profile
            .or(legacy_agent_type)
            .is_some_and(is_legacy_unnamed_profile);
        let profile = match (profile.map(str::trim), legacy_agent_type.map(str::trim)) {
            (Some(profile), Some(legacy)) if !profile.eq_ignore_ascii_case(legacy) => {
                return Err(format!(
                    "Conflicting agent profile '{profile}' and legacy agent_type '{legacy}'"
                ));
            }
            (Some(profile), _) if !profile.is_empty() => profile.to_ascii_lowercase(),
            (_, Some(legacy)) if !legacy.is_empty() => legacy.to_ascii_lowercase(),
            _ => "child".to_string(),
        };
        if profile.len() > 64 {
            return Err("Agent profile must be 64 characters or fewer".to_string());
        }

        let mut requested_capabilities = capabilities
            .iter()
            .map(|value| AgentCapability::parse(value))
            .collect::<Result<BTreeSet<_>, _>>()?;
        let preset = preset_capabilities(&profile);
        if requested_capabilities.is_empty() {
            requested_capabilities = preset;
        } else if matches!(profile.as_str(), "explore" | "plan")
            && (requested_capabilities.contains(&AgentCapability::Write)
                || requested_capabilities.contains(&AgentCapability::Execute))
        {
            return Err(format!(
                "Profile '{profile}' is read-only and cannot request write or execute"
            ));
        }
        // Name is the parent-chosen identity for status/completion. Prefer
        // explicit name over legacy profile labels.
        let explicit_task_name = task_name
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToString::to_string);
        if explicit_task_name.is_none() && !has_legacy_identity {
            return Err(
                "Agent spawn requires a non-empty name. Legacy profile/agent_type calls may omit it during compatibility replay."
                    .to_string(),
            );
        }
        let task_name = explicit_task_name.unwrap_or_else(|| {
            if matches!(
                profile.as_str(),
                "child" | "general" | "explore" | "plan" | "verify" | "build" | "default"
            ) {
                "child".to_string()
            } else {
                profile.clone()
            }
        });
        if task_name.len() > 96 {
            return Err("Agent task_name must be 96 characters or fewer".to_string());
        }

        Ok(Self {
            task_name,
            objective: objective.to_string(),
            expected_output: clean_optional(expected_output),
            delegation_reason: clean_optional(delegation_reason),
            profile,
            capabilities: requested_capabilities,
            context: AgentContextMode::parse(context)?,
            max_turns: restrictive_turn_budget(requested_max_turns, inherited_max_turns),
        })
    }

    /// Capability class for tool policy only — not a product persona.
    pub fn execution_profile(&self) -> AgentExecutionProfile {
        if self.capabilities.contains(&AgentCapability::Write) {
            AgentExecutionProfile::Build
        } else {
            AgentExecutionProfile::Explore
        }
    }

    pub fn allows_execute(&self) -> bool {
        self.capabilities.contains(&AgentCapability::Execute)
    }

    pub fn parent_context_turns(&self) -> Option<usize> {
        match self.context {
            AgentContextMode::Project => None,
            AgentContextMode::Brief => Some(10),
            AgentContextMode::Full => Some(usize::MAX),
            AgentContextMode::Auto
                if self.execution_profile() == AgentExecutionProfile::Explore =>
            {
                Some(10)
            }
            AgentContextMode::Auto => None,
        }
    }

    pub fn rendered_objective(&self) -> String {
        let mut sections = vec![self.objective.clone()];
        if let Some(reason) = self.delegation_reason.as_deref() {
            sections.push(format!("Delegation reason: {reason}"));
        }
        if let Some(expected) = self.expected_output.as_deref() {
            sections.push(format!("Expected output: {expected}"));
        }
        sections.join("\n\n")
    }
}

fn is_legacy_unnamed_profile(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "explore" | "build" | "worker" | "plan" | "verify"
    )
}

fn preset_capabilities(profile: &str) -> BTreeSet<AgentCapability> {
    match profile {
        // Legacy build/worker implied the old builder surface, including
        // command execution. Explicit capability requests remain exact.
        "build" | "worker" => BTreeSet::from([
            AgentCapability::Read,
            AgentCapability::Write,
            AgentCapability::Execute,
        ]),
        "verify" => BTreeSet::from([AgentCapability::Read, AgentCapability::Execute]),
        // explore, plan, child, general, custom labels → read by default
        _ => BTreeSet::from([AgentCapability::Read]),
    }
}

fn clean_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn restrictive_turn_budget(requested: Option<usize>, inherited: Option<usize>) -> Option<usize> {
    match (requested, inherited) {
        (Some(requested), Some(inherited)) => Some(requested.min(inherited).max(1)),
        // A model-authored Agent call must not invent a hidden child ceiling
        // when the parent/session contract is unlimited. Requested values may
        // only narrow an inherited governance budget.
        (Some(_), None) => None,
        (None, inherited) => inherited.map(|value| value.max(1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_child_requires_parent_chosen_name() {
        let error =
            AgentSpec::resolve(None, None, None, "audit", None, None, &[], None, None, None)
                .unwrap_err();
        assert!(error.contains("requires a non-empty name"));
    }

    #[test]
    fn legacy_profile_may_supply_compatibility_name() {
        let spec = AgentSpec::resolve(
            Some("explore"),
            None,
            None,
            "audit",
            None,
            None,
            &[],
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(spec.profile, "explore");
        assert_eq!(spec.task_name, "child");
        assert_eq!(spec.execution_profile(), AgentExecutionProfile::Explore);
        assert_eq!(spec.parent_context_turns(), Some(10));
    }

    #[test]
    fn generic_or_custom_profile_does_not_bypass_required_name() {
        for profile in ["child", "default", "security-audit"] {
            let error = AgentSpec::resolve(
                Some(profile),
                None,
                None,
                "audit",
                None,
                None,
                &[],
                None,
                None,
                None,
            )
            .unwrap_err();
            assert!(
                error.contains("requires a non-empty name"),
                "{profile}: {error}"
            );
        }
    }

    #[test]
    fn parent_name_and_write_capability_define_child_not_profile_kind() {
        let spec = AgentSpec::resolve(
            Some("api-specialist"),
            None,
            Some("map ChatBar height"),
            "implement the padding fix",
            None,
            None,
            &["write".to_string()],
            Some("project"),
            Some(80),
            Some(30),
        )
        .unwrap();
        assert_eq!(spec.task_name, "map ChatBar height");
        assert_eq!(spec.execution_profile(), AgentExecutionProfile::Build);
        assert_eq!(spec.max_turns, Some(30));
        assert_eq!(spec.parent_context_turns(), None);
        assert!(spec
            .rendered_objective()
            .contains("implement the padding fix"));
        assert!(!spec.rendered_objective().contains("specialist profile"));
    }

    #[test]
    fn model_request_cannot_invent_a_turn_budget_for_an_unlimited_parent() {
        let spec = AgentSpec::resolve(
            None,
            None,
            Some("build proof"),
            "complete the proof",
            None,
            None,
            &["write".to_string()],
            None,
            Some(20),
            None,
        )
        .expect("resolved spec");

        assert_eq!(spec.max_turns, None);
    }

    #[test]
    fn legacy_plan_and_verify_labels_are_not_separate_engines() {
        let plan = AgentSpec::resolve(
            Some("plan"),
            None,
            Some("draft approach"),
            "produce a plan",
            None,
            None,
            &[],
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(plan.execution_profile(), AgentExecutionProfile::Explore);

        let verify = AgentSpec::resolve(
            Some("verify"),
            None,
            Some("check tests"),
            "run focused checks",
            None,
            None,
            &[],
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(verify.execution_profile(), AgentExecutionProfile::Explore);
        assert!(verify.allows_execute());
    }

    #[test]
    fn read_only_profiles_reject_capability_escalation() {
        let error = AgentSpec::resolve(
            Some("explore"),
            None,
            None,
            "change files",
            None,
            None,
            &["write".to_string()],
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("read-only"));
    }

    #[test]
    fn explicit_execute_only_capability_remains_exact() {
        let spec = AgentSpec::resolve(
            None,
            None,
            Some("focused command"),
            "run one focused validation",
            None,
            None,
            &["execute".to_string()],
            None,
            None,
            None,
        )
        .unwrap();

        assert_eq!(
            spec.capabilities,
            BTreeSet::from([AgentCapability::Execute])
        );
        assert!(spec.allows_execute());
        assert_eq!(spec.execution_profile(), AgentExecutionProfile::Explore);
    }

    #[test]
    fn conflicting_legacy_and_new_profiles_fail_closed() {
        let error = AgentSpec::resolve(
            Some("verify"),
            Some("build"),
            None,
            "test",
            None,
            None,
            &[],
            None,
            None,
            None,
        )
        .unwrap_err();
        assert!(error.contains("Conflicting"));
    }
}
