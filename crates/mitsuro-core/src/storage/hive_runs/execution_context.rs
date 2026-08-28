use std::path::Path;

use anyhow::{ensure, Result};
use serde::{Deserialize, Deserializer, Serialize};

use crate::storage::{WorkerConversationLane, WorkspaceMode};

pub const HIVE_RUN_EXECUTION_CONTEXT_VERSION: u8 = 1;
const MAX_WORKER_BINDING_ID_BYTES: usize = 256;
const MAX_WORKSPACE_PATH_BYTES: usize = 16 * 1024;

/// Frozen, least-privilege execution context for a Worker-bound Hive run.
///
/// This is intentionally separate from `config_json`: the latter still
/// carries model/runtime options, while this value is the authoritative
/// capability and Worker/lane binding. Legacy non-Worker runs may have no
/// execution context; every new Worker-bound run must have one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HiveRunExecutionContextV1 {
    pub schema_version: u8,
    pub mode: HiveRunExecutionModeV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HiveRunExecutionModeV1 {
    WorkerConversationNeutral {
        worker_id: String,
        worker_revision: u64,
        lane: WorkerConversationLane,
    },
    WorkerWorkspaceAttached {
        worker_id: String,
        worker_revision: u64,
        lane: WorkerConversationLane,
        workspace_mode: WorkspaceMode,
        working_dir: String,
        project_dir: Option<String>,
    },
    /// Exact attached-workspace authority for one bounded durable Workflow
    /// attempt.  Goal/plan/step identities come from trusted storage and are
    /// frozen here so execution never accepts model-selected identifiers.
    WorkerGoal {
        worker_id: String,
        worker_revision: u64,
        lane: WorkerConversationLane,
        workspace_mode: WorkspaceMode,
        working_dir: String,
        project_dir: String,
        goal_id: String,
        goal_revision: u64,
        workflow_aggregate_revision: u64,
        attempt_id: String,
        plan_revision_id: String,
        plan_revision_number: u64,
        step_id: String,
        step_revision: u64,
        tool_allowlist: Vec<String>,
    },
    /// Frozen, provider-free owner acceptance boundary for one exact
    /// `Progressed` Worker Goal outcome. This mode is deliberately distinct
    /// from executable Worker Goal authority and has an empty tool ceiling.
    WorkerGoalAcceptance {
        worker_id: String,
        worker_revision: u64,
        lane: WorkerConversationLane,
        workspace_mode: WorkspaceMode,
        working_dir: String,
        project_dir: String,
        source_run_id: String,
        goal_id: String,
        goal_revision: u64,
        workflow_aggregate_revision: u64,
        source_attempt_id: String,
        plan_revision_id: String,
        plan_revision_number: u64,
        step_id: String,
        step_revision: u64,
        acceptance_contract_sha256: String,
        source_outcome_sha256: String,
        tool_allowlist: Vec<String>,
    },
}

impl HiveRunExecutionContextV1 {
    pub fn worker_conversation_neutral(
        worker_id: impl Into<String>,
        worker_revision: u64,
        lane: WorkerConversationLane,
    ) -> Result<Self> {
        let context = Self {
            schema_version: HIVE_RUN_EXECUTION_CONTEXT_VERSION,
            mode: HiveRunExecutionModeV1::WorkerConversationNeutral {
                worker_id: worker_id.into(),
                worker_revision,
                lane,
            },
        };
        context.validate()?;
        Ok(context)
    }

    pub fn worker_workspace_attached(
        worker_id: impl Into<String>,
        worker_revision: u64,
        lane: WorkerConversationLane,
        workspace_mode: WorkspaceMode,
        working_dir: impl Into<String>,
        project_dir: Option<String>,
    ) -> Result<Self> {
        let context = Self {
            schema_version: HIVE_RUN_EXECUTION_CONTEXT_VERSION,
            mode: HiveRunExecutionModeV1::WorkerWorkspaceAttached {
                worker_id: worker_id.into(),
                worker_revision,
                lane,
                workspace_mode,
                working_dir: working_dir.into(),
                project_dir,
            },
        };
        context.validate()?;
        Ok(context)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn worker_goal(
        worker_id: impl Into<String>,
        worker_revision: u64,
        workspace_mode: WorkspaceMode,
        working_dir: impl Into<String>,
        project_dir: impl Into<String>,
        goal_id: impl Into<String>,
        goal_revision: u64,
        workflow_aggregate_revision: u64,
        attempt_id: impl Into<String>,
        plan_revision_id: impl Into<String>,
        plan_revision_number: u64,
        step_id: impl Into<String>,
        step_revision: u64,
        tool_allowlist: Vec<String>,
    ) -> Result<Self> {
        let context = Self {
            schema_version: HIVE_RUN_EXECUTION_CONTEXT_VERSION,
            mode: HiveRunExecutionModeV1::WorkerGoal {
                worker_id: worker_id.into(),
                worker_revision,
                lane: WorkerConversationLane::DirectMessage,
                workspace_mode,
                working_dir: working_dir.into(),
                project_dir: project_dir.into(),
                goal_id: goal_id.into(),
                goal_revision,
                workflow_aggregate_revision,
                attempt_id: attempt_id.into(),
                plan_revision_id: plan_revision_id.into(),
                plan_revision_number,
                step_id: step_id.into(),
                step_revision,
                tool_allowlist,
            },
        };
        context.validate()?;
        Ok(context)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn worker_goal_acceptance(
        worker_id: impl Into<String>,
        worker_revision: u64,
        workspace_mode: WorkspaceMode,
        working_dir: impl Into<String>,
        project_dir: impl Into<String>,
        source_run_id: impl Into<String>,
        goal_id: impl Into<String>,
        goal_revision: u64,
        workflow_aggregate_revision: u64,
        source_attempt_id: impl Into<String>,
        plan_revision_id: impl Into<String>,
        plan_revision_number: u64,
        step_id: impl Into<String>,
        step_revision: u64,
        acceptance_contract_sha256: impl Into<String>,
        source_outcome_sha256: impl Into<String>,
    ) -> Result<Self> {
        let context = Self {
            schema_version: HIVE_RUN_EXECUTION_CONTEXT_VERSION,
            mode: HiveRunExecutionModeV1::WorkerGoalAcceptance {
                worker_id: worker_id.into(),
                worker_revision,
                lane: WorkerConversationLane::DirectMessage,
                workspace_mode,
                working_dir: working_dir.into(),
                project_dir: project_dir.into(),
                source_run_id: source_run_id.into(),
                goal_id: goal_id.into(),
                goal_revision,
                workflow_aggregate_revision,
                source_attempt_id: source_attempt_id.into(),
                plan_revision_id: plan_revision_id.into(),
                plan_revision_number,
                step_id: step_id.into(),
                step_revision,
                acceptance_contract_sha256: acceptance_contract_sha256.into(),
                source_outcome_sha256: source_outcome_sha256.into(),
                tool_allowlist: Vec::new(),
            },
        };
        context.validate()?;
        Ok(context)
    }

    pub fn worker_id(&self) -> &str {
        match &self.mode {
            HiveRunExecutionModeV1::WorkerConversationNeutral { worker_id, .. }
            | HiveRunExecutionModeV1::WorkerWorkspaceAttached { worker_id, .. }
            | HiveRunExecutionModeV1::WorkerGoal { worker_id, .. }
            | HiveRunExecutionModeV1::WorkerGoalAcceptance { worker_id, .. } => worker_id,
        }
    }

    pub fn worker_revision(&self) -> u64 {
        match &self.mode {
            HiveRunExecutionModeV1::WorkerConversationNeutral {
                worker_revision, ..
            }
            | HiveRunExecutionModeV1::WorkerWorkspaceAttached {
                worker_revision, ..
            }
            | HiveRunExecutionModeV1::WorkerGoal {
                worker_revision, ..
            }
            | HiveRunExecutionModeV1::WorkerGoalAcceptance {
                worker_revision, ..
            } => *worker_revision,
        }
    }

    pub fn lane(&self) -> &WorkerConversationLane {
        match &self.mode {
            HiveRunExecutionModeV1::WorkerConversationNeutral { lane, .. }
            | HiveRunExecutionModeV1::WorkerWorkspaceAttached { lane, .. }
            | HiveRunExecutionModeV1::WorkerGoal { lane, .. }
            | HiveRunExecutionModeV1::WorkerGoalAcceptance { lane, .. } => lane,
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == HIVE_RUN_EXECUTION_CONTEXT_VERSION,
            "unsupported Hive run execution context version"
        );
        validate_worker_binding(self.worker_id(), self.worker_revision(), self.lane())?;
        if let HiveRunExecutionModeV1::WorkerWorkspaceAttached {
            workspace_mode,
            working_dir,
            project_dir,
            ..
        } = &self.mode
        {
            ensure!(
                matches!(
                    workspace_mode,
                    WorkspaceMode::Selected | WorkspaceMode::Created
                ),
                "workspace-attached Worker run has a neutral workspace mode"
            );
            validate_absolute_path(working_dir, "working directory")?;
            if let Some(project_dir) = project_dir {
                validate_absolute_path(project_dir, "project directory")?;
            }
        }
        if let HiveRunExecutionModeV1::WorkerGoal {
            lane,
            workspace_mode,
            working_dir,
            project_dir,
            goal_id,
            goal_revision,
            workflow_aggregate_revision,
            attempt_id,
            plan_revision_id,
            plan_revision_number,
            step_id,
            step_revision,
            tool_allowlist,
            ..
        } = &self.mode
        {
            ensure!(
                matches!(lane, WorkerConversationLane::DirectMessage),
                "Worker Goal must use the private direct-message lane"
            );
            ensure!(
                matches!(
                    workspace_mode,
                    WorkspaceMode::Selected | WorkspaceMode::Created
                ),
                "Worker Goal has no attached workspace"
            );
            validate_absolute_path(working_dir, "working directory")?;
            validate_absolute_path(project_dir, "project directory")?;
            ensure!(
                working_dir == project_dir,
                "Worker Goal working and project directories differ"
            );
            for (value, label) in [
                (goal_id.as_str(), "Goal id"),
                (attempt_id.as_str(), "attempt id"),
                (plan_revision_id.as_str(), "plan revision id"),
                (step_id.as_str(), "step id"),
            ] {
                validate_bounded_id(value, label)?;
            }
            ensure!(
                *goal_revision > 0
                    && *workflow_aggregate_revision == *goal_revision
                    && *plan_revision_number > 0
                    && *step_revision > 0,
                "Worker Goal revision binding is invalid"
            );
            const CEILING: [&str; 8] = [
                "apply_patch",
                "bash",
                "edit",
                "glob",
                "grep",
                "list",
                "multiedit",
                "read",
            ];
            ensure!(
                !tool_allowlist.is_empty() && tool_allowlist.len() <= CEILING.len(),
                "Worker Goal tool allowlist is empty or oversized"
            );
            let mut unique = std::collections::HashSet::with_capacity(tool_allowlist.len());
            for tool in tool_allowlist {
                ensure!(
                    CEILING.contains(&tool.as_str()) && unique.insert(tool.as_str()),
                    "Worker Goal tool allowlist exceeds the capability ceiling"
                );
            }
        }
        if let HiveRunExecutionModeV1::WorkerGoalAcceptance {
            lane,
            workspace_mode,
            working_dir,
            project_dir,
            source_run_id,
            goal_id,
            goal_revision,
            workflow_aggregate_revision,
            source_attempt_id,
            plan_revision_id,
            plan_revision_number,
            step_id,
            step_revision,
            acceptance_contract_sha256,
            source_outcome_sha256,
            tool_allowlist,
            ..
        } = &self.mode
        {
            ensure!(
                matches!(lane, WorkerConversationLane::DirectMessage),
                "Worker Goal acceptance must use the private direct-message lane"
            );
            ensure!(
                matches!(
                    workspace_mode,
                    WorkspaceMode::Selected | WorkspaceMode::Created
                ),
                "Worker Goal acceptance has no attached workspace"
            );
            validate_absolute_path(working_dir, "working directory")?;
            validate_absolute_path(project_dir, "project directory")?;
            ensure!(
                working_dir == project_dir,
                "Worker Goal acceptance working and project directories differ"
            );
            for (value, label) in [
                (source_run_id.as_str(), "source run id"),
                (goal_id.as_str(), "Goal id"),
                (source_attempt_id.as_str(), "source attempt id"),
                (plan_revision_id.as_str(), "plan revision id"),
                (step_id.as_str(), "step id"),
            ] {
                validate_bounded_id(value, label)?;
            }
            ensure!(
                *goal_revision > 0
                    && *workflow_aggregate_revision == *goal_revision
                    && *plan_revision_number > 0
                    && *step_revision > 0,
                "Worker Goal acceptance revision binding is invalid"
            );
            validate_sha256(acceptance_contract_sha256, "acceptance contract digest")?;
            validate_sha256(source_outcome_sha256, "source outcome digest")?;
            ensure!(
                tool_allowlist.is_empty(),
                "Worker Goal acceptance cannot carry executable tools"
            );
        }
        Ok(())
    }
}

impl WorkerConversationLane {
    /// Stable governor/concurrency lane key. Worker identity is stored in its
    /// own column, so `dm` is unambiguous within one Worker.
    pub fn canonical_lane_key(&self) -> Result<String> {
        match self {
            Self::DirectMessage => Ok("dm".to_string()),
            Self::Group { group_id } => {
                validate_bounded_id(group_id, "group id")?;
                Ok(format!("group:{group_id}"))
            }
        }
    }
}

fn validate_worker_binding(
    worker_id: &str,
    worker_revision: u64,
    lane: &WorkerConversationLane,
) -> Result<()> {
    validate_bounded_id(worker_id, "Worker id")?;
    ensure!(worker_revision >= 1, "Worker revision must be at least one");
    let _ = lane.canonical_lane_key()?;
    Ok(())
}

fn validate_bounded_id(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    ensure!(!value.is_empty(), "{label} is empty");
    ensure!(
        value.len() <= MAX_WORKER_BINDING_ID_BYTES,
        "{label} exceeds the byte limit"
    );
    ensure!(
        !value.chars().any(char::is_control),
        "{label} contains control characters"
    );
    Ok(())
}

fn validate_absolute_path(value: &str, label: &str) -> Result<()> {
    let value = value.trim();
    ensure!(!value.is_empty(), "{label} is empty");
    ensure!(
        value.len() <= MAX_WORKSPACE_PATH_BYTES,
        "{label} exceeds the byte limit"
    );
    ensure!(Path::new(value).is_absolute(), "{label} is not absolute");
    Ok(())
}

fn validate_sha256(value: &str, label: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
        "{label} is not lowercase SHA-256"
    );
    Ok(())
}

// Dedicated raw types make unknown-field rejection structural even though
// the governor's shared WorkerConversationLane predates this contract.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawExecutionContextV1 {
    schema_version: u8,
    mode: RawExecutionModeV1,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
// Variant names intentionally mirror the frozen `worker_*` wire discriminants
// and their validated public execution-mode counterparts.
#[allow(clippy::enum_variant_names)]
enum RawExecutionModeV1 {
    WorkerConversationNeutral {
        worker_id: String,
        worker_revision: u64,
        lane: RawConversationLane,
    },
    WorkerWorkspaceAttached {
        worker_id: String,
        worker_revision: u64,
        lane: RawConversationLane,
        workspace_mode: WorkspaceMode,
        working_dir: String,
        project_dir: Option<String>,
    },
    WorkerGoal {
        worker_id: String,
        worker_revision: u64,
        lane: RawConversationLane,
        workspace_mode: WorkspaceMode,
        working_dir: String,
        project_dir: String,
        goal_id: String,
        goal_revision: u64,
        workflow_aggregate_revision: u64,
        attempt_id: String,
        plan_revision_id: String,
        plan_revision_number: u64,
        step_id: String,
        step_revision: u64,
        tool_allowlist: Vec<String>,
    },
    WorkerGoalAcceptance {
        worker_id: String,
        worker_revision: u64,
        lane: RawConversationLane,
        workspace_mode: WorkspaceMode,
        working_dir: String,
        project_dir: String,
        source_run_id: String,
        goal_id: String,
        goal_revision: u64,
        workflow_aggregate_revision: u64,
        source_attempt_id: String,
        plan_revision_id: String,
        plan_revision_number: u64,
        step_id: String,
        step_revision: u64,
        acceptance_contract_sha256: String,
        source_outcome_sha256: String,
        tool_allowlist: Vec<String>,
    },
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawConversationLane {
    DirectMessage {},
    Group { group_id: String },
}

impl From<RawConversationLane> for WorkerConversationLane {
    fn from(value: RawConversationLane) -> Self {
        match value {
            RawConversationLane::DirectMessage {} => Self::DirectMessage,
            RawConversationLane::Group { group_id } => Self::Group { group_id },
        }
    }
}

impl<'de> Deserialize<'de> for HiveRunExecutionContextV1 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawExecutionContextV1::deserialize(deserializer)?;
        let mode = match raw.mode {
            RawExecutionModeV1::WorkerConversationNeutral {
                worker_id,
                worker_revision,
                lane,
            } => HiveRunExecutionModeV1::WorkerConversationNeutral {
                worker_id,
                worker_revision,
                lane: lane.into(),
            },
            RawExecutionModeV1::WorkerWorkspaceAttached {
                worker_id,
                worker_revision,
                lane,
                workspace_mode,
                working_dir,
                project_dir,
            } => HiveRunExecutionModeV1::WorkerWorkspaceAttached {
                worker_id,
                worker_revision,
                lane: lane.into(),
                workspace_mode,
                working_dir,
                project_dir,
            },
            RawExecutionModeV1::WorkerGoal {
                worker_id,
                worker_revision,
                lane,
                workspace_mode,
                working_dir,
                project_dir,
                goal_id,
                goal_revision,
                workflow_aggregate_revision,
                attempt_id,
                plan_revision_id,
                plan_revision_number,
                step_id,
                step_revision,
                tool_allowlist,
            } => HiveRunExecutionModeV1::WorkerGoal {
                worker_id,
                worker_revision,
                lane: lane.into(),
                workspace_mode,
                working_dir,
                project_dir,
                goal_id,
                goal_revision,
                workflow_aggregate_revision,
                attempt_id,
                plan_revision_id,
                plan_revision_number,
                step_id,
                step_revision,
                tool_allowlist,
            },
            RawExecutionModeV1::WorkerGoalAcceptance {
                worker_id,
                worker_revision,
                lane,
                workspace_mode,
                working_dir,
                project_dir,
                source_run_id,
                goal_id,
                goal_revision,
                workflow_aggregate_revision,
                source_attempt_id,
                plan_revision_id,
                plan_revision_number,
                step_id,
                step_revision,
                acceptance_contract_sha256,
                source_outcome_sha256,
                tool_allowlist,
            } => HiveRunExecutionModeV1::WorkerGoalAcceptance {
                worker_id,
                worker_revision,
                lane: lane.into(),
                workspace_mode,
                working_dir,
                project_dir,
                source_run_id,
                goal_id,
                goal_revision,
                workflow_aggregate_revision,
                source_attempt_id,
                plan_revision_id,
                plan_revision_number,
                step_id,
                step_revision,
                acceptance_contract_sha256,
                source_outcome_sha256,
                tool_allowlist,
            },
        };
        let context = Self {
            schema_version: raw.schema_version,
            mode,
        };
        context.validate().map_err(serde::de::Error::custom)?;
        Ok(context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_context_is_strict_and_has_a_canonical_lane() {
        let context = HiveRunExecutionContextV1::worker_conversation_neutral(
            "worker-1",
            3,
            WorkerConversationLane::DirectMessage,
        )
        .unwrap();
        assert_eq!(context.lane().canonical_lane_key().unwrap(), "dm");
        let encoded = serde_json::to_string(&context).unwrap();
        assert_eq!(
            serde_json::from_str::<HiveRunExecutionContextV1>(&encoded).unwrap(),
            context
        );

        let unknown = r#"{"schema_version":1,"mode":{"kind":"worker_conversation_neutral","worker_id":"worker-1","worker_revision":3,"lane":{"kind":"direct_message","forged":true}}}"#;
        assert!(serde_json::from_str::<HiveRunExecutionContextV1>(unknown).is_err());
    }

    #[test]
    fn attached_context_requires_a_real_absolute_workspace() {
        assert!(HiveRunExecutionContextV1::worker_workspace_attached(
            "worker-1",
            1,
            WorkerConversationLane::Group {
                group_id: "group-1".into(),
            },
            WorkspaceMode::Selected,
            "/work/repo",
            Some("/work/repo".into()),
        )
        .is_ok());
        assert!(HiveRunExecutionContextV1::worker_workspace_attached(
            "worker-1",
            1,
            WorkerConversationLane::DirectMessage,
            WorkspaceMode::Neutral,
            "/work/repo",
            None,
        )
        .is_err());
        assert!(HiveRunExecutionContextV1::worker_workspace_attached(
            "worker-1",
            1,
            WorkerConversationLane::DirectMessage,
            WorkspaceMode::Created,
            "relative",
            None,
        )
        .is_err());
    }

    #[test]
    fn acceptance_context_is_provider_and_tool_free_with_exact_digests() {
        let context = HiveRunExecutionContextV1::worker_goal_acceptance(
            "worker-1",
            3,
            WorkspaceMode::Selected,
            "/work/repo",
            "/work/repo",
            "source-run",
            "goal-1",
            8,
            8,
            "attempt-1",
            "plan-1",
            2,
            "step-1",
            4,
            "a".repeat(64),
            "b".repeat(64),
        )
        .unwrap();
        let encoded = serde_json::to_string(&context).unwrap();
        assert_eq!(
            serde_json::from_str::<HiveRunExecutionContextV1>(&encoded).unwrap(),
            context
        );
        assert_eq!(context.lane().canonical_lane_key().unwrap(), "dm");

        let mut forged: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        forged["mode"]["tool_allowlist"] = serde_json::json!(["read"]);
        assert!(serde_json::from_value::<HiveRunExecutionContextV1>(forged).is_err());
    }
}
