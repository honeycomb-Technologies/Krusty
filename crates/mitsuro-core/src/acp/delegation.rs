//! ACP projection of the canonical durable delegation lifecycle.
//!
//! ACP has no native delegation/group event type. Exact lifecycle names are
//! therefore carried in stable standard tool calls, while the SQLite event
//! cursor remains the replay authority.

use std::collections::HashMap;

use agent_client_protocol::{
    SessionUpdate, ToolCall, ToolCallId, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};

use crate::storage::{
    DelegationEventRecord, DelegationEventType, DelegationGroupRecord, DelegationGroupState,
    DelegationTaskRecord, DelegationTaskState,
};

use super::tools::{text_to_tool_content, tool_name_to_kind};

#[derive(Default)]
pub(crate) struct AcpDelegationProjection {
    initialized: bool,
    cursor: i64,
    groups: HashMap<String, DelegationGroupState>,
    tasks: HashMap<String, DelegationTaskState>,
}

impl AcpDelegationProjection {
    pub(crate) fn cursor(&self) -> i64 {
        self.cursor
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub(crate) fn contains_group(&self, group_id: &str) -> bool {
        self.groups.contains_key(group_id)
    }

    pub(crate) fn hydrate(
        &mut self,
        groups: &[DelegationGroupRecord],
        cursor: i64,
    ) -> Vec<SessionUpdate> {
        let mut updates = Vec::new();
        for group in groups {
            self.project_group(group, group.state, &mut updates);
            for task in &group.tasks {
                self.project_task(task, task.state, &mut updates);
            }
        }
        self.initialized = true;
        self.cursor = self.cursor.max(cursor);
        updates
    }

    pub(crate) fn apply_event(
        &mut self,
        event: &DelegationEventRecord,
        group: Option<&DelegationGroupRecord>,
    ) -> Vec<SessionUpdate> {
        if event.event_id <= self.cursor {
            return Vec::new();
        }
        let mut updates = Vec::new();
        match &event.event_type {
            DelegationEventType::GroupCreated => {
                if let Some(group) = group {
                    self.project_group(group, DelegationGroupState::Created, &mut updates);
                    for task in &group.tasks {
                        self.project_task(task, DelegationTaskState::Created, &mut updates);
                    }
                }
            }
            DelegationEventType::GroupQueued => {
                if let Some(group) = group {
                    self.project_group(group, DelegationGroupState::Queued, &mut updates);
                    for task in &group.tasks {
                        self.project_task(task, DelegationTaskState::Queued, &mut updates);
                    }
                }
            }
            DelegationEventType::GroupStateChanged => {
                if let (Some(group), Some(state)) = (
                    group,
                    event
                        .payload
                        .get("to")
                        .cloned()
                        .and_then(|value| serde_json::from_value(value).ok()),
                ) {
                    self.project_group(group, state, &mut updates);
                }
            }
            DelegationEventType::TaskClaimed => {
                self.project_event_task(group, event, DelegationTaskState::Leased, &mut updates);
            }
            DelegationEventType::TaskRunning => {
                self.project_event_task(group, event, DelegationTaskState::Running, &mut updates);
            }
            DelegationEventType::TaskStateChanged => {
                if let Some(state) = event
                    .payload
                    .get("state")
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                {
                    self.project_event_task(group, event, state, &mut updates);
                }
            }
            DelegationEventType::ParentContinuationQueued
            | DelegationEventType::ParentContinuationPromoted => {
                if let Some(group) = group {
                    updates.push(group_event_update(
                        group,
                        self.groups
                            .get(&group.delegation_group_id)
                            .copied()
                            .unwrap_or(group.state),
                        event.event_type.as_str(),
                        &event.payload,
                    ));
                }
            }
            DelegationEventType::Other(kind) => {
                if let Some(group) = group {
                    updates.push(group_event_update(
                        group,
                        self.groups
                            .get(&group.delegation_group_id)
                            .copied()
                            .unwrap_or(group.state),
                        kind,
                        &event.payload,
                    ));
                }
            }
        }
        self.cursor = event.event_id;
        updates
    }

    fn project_event_task(
        &mut self,
        group: Option<&DelegationGroupRecord>,
        event: &DelegationEventRecord,
        state: DelegationTaskState,
        updates: &mut Vec<SessionUpdate>,
    ) {
        let Some(task_id) = event.delegation_task_id.as_deref() else {
            return;
        };
        let Some(task) = group.and_then(|group| {
            group
                .tasks
                .iter()
                .find(|task| task.specification.delegation_task_id == task_id)
        }) else {
            return;
        };
        self.project_task(task, state, updates);
    }

    fn project_group(
        &mut self,
        group: &DelegationGroupRecord,
        state: DelegationGroupState,
        updates: &mut Vec<SessionUpdate>,
    ) {
        let id = group.delegation_group_id.clone();
        match self.groups.insert(id, state) {
            Some(previous) if previous == state => {}
            Some(_) => updates.push(group_state_update(group, state)),
            None => updates.push(group_state_call(group, state)),
        }
    }

    fn project_task(
        &mut self,
        task: &DelegationTaskRecord,
        state: DelegationTaskState,
        updates: &mut Vec<SessionUpdate>,
    ) {
        let id = task.specification.delegation_task_id.clone();
        match self.tasks.insert(id, state) {
            Some(previous) if previous == state => {}
            Some(_) => updates.push(task_state_update(task, state)),
            None => updates.push(task_state_call(task, state)),
        }
    }
}

fn group_tool_id(group_id: &str) -> ToolCallId {
    ToolCallId::from(format!("delegation-group:{group_id}"))
}

fn task_tool_id(task_id: &str) -> ToolCallId {
    ToolCallId::from(format!("delegation-task:{task_id}"))
}

fn acp_group_status(state: DelegationGroupState) -> ToolCallStatus {
    match state {
        DelegationGroupState::Complete | DelegationGroupState::Degraded => {
            ToolCallStatus::Completed
        }
        DelegationGroupState::Failed | DelegationGroupState::Cancelled => ToolCallStatus::Failed,
        _ => ToolCallStatus::InProgress,
    }
}

fn acp_task_status(state: DelegationTaskState) -> ToolCallStatus {
    match state {
        DelegationTaskState::Complete | DelegationTaskState::Degraded => ToolCallStatus::Completed,
        DelegationTaskState::Failed | DelegationTaskState::Cancelled => ToolCallStatus::Failed,
        _ => ToolCallStatus::InProgress,
    }
}

fn group_state_text(group: &DelegationGroupRecord, state: DelegationGroupState) -> String {
    format!(
        "Delegation group {}: {} ({} tasks)",
        group.delegation_group_id,
        group_state_label(state),
        group.tasks.len()
    )
}

fn task_state_text(task: &DelegationTaskRecord, state: DelegationTaskState) -> String {
    format!(
        "Delegated task {}: {} (attempt {})",
        task.specification.task_key,
        task_state_label(state),
        task.attempt_count
    )
}

fn group_state_call(group: &DelegationGroupRecord, state: DelegationGroupState) -> SessionUpdate {
    SessionUpdate::ToolCall(
        ToolCall::new(
            group_tool_id(&group.delegation_group_id),
            format!("Delegation · {}", group_state_label(state)),
        )
        .kind(tool_name_to_kind("agent"))
        .status(acp_group_status(state))
        .content(vec![text_to_tool_content(&group_state_text(group, state))]),
    )
}

fn group_state_update(group: &DelegationGroupRecord, state: DelegationGroupState) -> SessionUpdate {
    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        group_tool_id(&group.delegation_group_id),
        ToolCallUpdateFields::new()
            .title(format!("Delegation · {}", group_state_label(state)))
            .status(acp_group_status(state))
            .content(vec![text_to_tool_content(&group_state_text(group, state))]),
    ))
}

fn task_state_call(task: &DelegationTaskRecord, state: DelegationTaskState) -> SessionUpdate {
    SessionUpdate::ToolCall(
        ToolCall::new(
            task_tool_id(&task.specification.delegation_task_id),
            format!(
                "{} · {}",
                task.specification.task_key,
                task_state_label(state)
            ),
        )
        .kind(tool_name_to_kind("agent"))
        .status(acp_task_status(state))
        .content(vec![text_to_tool_content(&task_state_text(task, state))]),
    )
}

fn task_state_update(task: &DelegationTaskRecord, state: DelegationTaskState) -> SessionUpdate {
    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        task_tool_id(&task.specification.delegation_task_id),
        ToolCallUpdateFields::new()
            .title(format!(
                "{} · {}",
                task.specification.task_key,
                task_state_label(state)
            ))
            .status(acp_task_status(state))
            .content(vec![text_to_tool_content(&task_state_text(task, state))]),
    ))
}

fn group_event_update(
    group: &DelegationGroupRecord,
    state: DelegationGroupState,
    kind: &str,
    payload: &serde_json::Value,
) -> SessionUpdate {
    let kind = bounded_event_kind(kind);
    let metadata = allowlisted_event_metadata(payload);
    SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
        group_tool_id(&group.delegation_group_id),
        ToolCallUpdateFields::new()
            .title(format!("Delegation · {kind}"))
            .status(acp_group_status(state))
            .content(vec![text_to_tool_content(&format!(
                "Delegation event {kind}{metadata}"
            ))]),
    ))
}

fn bounded_event_kind(kind: &str) -> String {
    let value = kind
        .chars()
        .filter(|character| !character.is_control())
        .take(128)
        .collect::<String>();
    if value.is_empty() {
        "unknown".to_owned()
    } else {
        value
    }
}

fn allowlisted_event_metadata(payload: &serde_json::Value) -> String {
    for field in ["state", "to", "reason"] {
        if let Some(value) = payload.get(field).and_then(serde_json::Value::as_str) {
            let value = bounded_event_kind(value);
            return format!(" · {field}={value}");
        }
    }
    String::new()
}

pub(crate) fn group_state_label(state: DelegationGroupState) -> &'static str {
    match state {
        DelegationGroupState::Created => "created",
        DelegationGroupState::Queued => "queued",
        DelegationGroupState::Running => "running",
        DelegationGroupState::ReadyForParent => "ready_for_parent",
        DelegationGroupState::Synthesizing => "synthesizing",
        DelegationGroupState::Complete => "complete",
        DelegationGroupState::Degraded => "degraded",
        DelegationGroupState::Failed => "failed",
        DelegationGroupState::Cancelled => "cancelled",
    }
}

pub(crate) fn task_state_label(state: DelegationTaskState) -> &'static str {
    match state {
        DelegationTaskState::Created => "created",
        DelegationTaskState::Queued => "queued",
        DelegationTaskState::Leased => "leased",
        DelegationTaskState::Running => "running",
        DelegationTaskState::Retrying => "retrying",
        DelegationTaskState::Complete => "complete",
        DelegationTaskState::Degraded => "degraded",
        DelegationTaskState::Failed => "failed",
        DelegationTaskState::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;

    use crate::storage::{
        DelegatedRunRole, DelegationCompletionPolicy, DelegationExecutionMode,
        DelegationFailurePolicy, DelegationGovernance, DelegationGroupContract,
        DelegationParentContinuationState, DelegationTaskSpec, DelegationWriterMode,
    };
    use crate::tools::registry::{DelegationPolicy, PermissionMode};

    fn group() -> DelegationGroupRecord {
        let now = Utc::now();
        DelegationGroupRecord {
            delegation_group_id: "group-1".to_owned(),
            parent_session_id: "session-1".to_owned(),
            parent_tool_call_id: Some("call-1".to_owned()),
            contract: DelegationGroupContract {
                execution_mode: DelegationExecutionMode::Detached,
                completion_policy: DelegationCompletionPolicy::AllSettled,
                failure_policy: DelegationFailurePolicy::Continue,
                governance: DelegationGovernance {
                    permission_mode: PermissionMode::Supervised,
                    delegated_turn_budget: 4,
                    max_parallelism: 1,
                    execution_tool_allowlist: None,
                    delegation_policy: DelegationPolicy::for_subagent_build(
                        PermissionMode::Supervised,
                        Some(4),
                    ),
                },
            },
            state: DelegationGroupState::Queued,
            parent_continuation_state: DelegationParentContinuationState::NotRequested,
            parent_continuation_id: None,
            synthesis_owner_id: None,
            synthesis_lease_expires_at_ms: None,
            synthesis_attempt_count: 0,
            tasks: vec![DelegationTaskRecord {
                delegation_group_id: "group-1".to_owned(),
                ordinal: 0,
                specification: DelegationTaskSpec {
                    delegation_task_id: "task-1".to_owned(),
                    task_key: "builder".to_owned(),
                    objective: "build".to_owned(),
                    role: DelegatedRunRole::Build,
                    target_scope: Vec::new(),
                    max_attempts: 2,
                    writer_mode: DelegationWriterMode::Isolated,
                    attempt_workspace: None,
                    workspace_baseline: None,
                    executor_envelope: None,
                },
                state: DelegationTaskState::Queued,
                attempt_count: 0,
                result: None,
                error_summary: None,
                created_at: now,
                updated_at: now,
                completed_at: None,
            }],
            created_at: now,
            updated_at: now,
            completed_at: None,
        }
    }

    fn event(
        event_id: i64,
        event_type: DelegationEventType,
        task_id: Option<&str>,
        payload: serde_json::Value,
    ) -> DelegationEventRecord {
        DelegationEventRecord {
            event_id,
            parent_session_id: "session-1".to_owned(),
            delegation_group_id: "group-1".to_owned(),
            delegation_task_id: task_id.map(str::to_owned),
            event_type,
            payload,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn projection_replays_exact_task_group_and_unknown_lifecycle() {
        let group = group();
        let mut projection = AcpDelegationProjection::default();
        assert!(projection.hydrate(&[], 0).is_empty());

        assert_eq!(
            projection
                .apply_event(
                    &event(1, DelegationEventType::GroupCreated, None, json!({})),
                    Some(&group),
                )
                .len(),
            2
        );
        assert_eq!(
            projection
                .apply_event(
                    &event(2, DelegationEventType::GroupQueued, None, json!({})),
                    Some(&group),
                )
                .len(),
            2
        );
        for (id, event_type, state) in [
            (3, DelegationEventType::TaskClaimed, "leased"),
            (4, DelegationEventType::TaskRunning, "running"),
            (5, DelegationEventType::TaskStateChanged, "retrying"),
            (6, DelegationEventType::TaskStateChanged, "complete"),
        ] {
            let payload = if matches!(&event_type, DelegationEventType::TaskStateChanged) {
                json!({"state": state})
            } else {
                json!({})
            };
            assert_eq!(
                projection
                    .apply_event(
                        &event(id, event_type, Some("task-1"), payload),
                        Some(&group)
                    )
                    .len(),
                1,
                "missing task state {state}"
            );
        }
        for (id, state) in [
            (7, "ready_for_parent"),
            (8, "synthesizing"),
            (9, "complete"),
        ] {
            assert_eq!(
                projection
                    .apply_event(
                        &event(
                            id,
                            DelegationEventType::GroupStateChanged,
                            None,
                            json!({"to": state}),
                        ),
                        Some(&group),
                    )
                    .len(),
                1,
                "missing group state {state}"
            );
        }
        assert_eq!(
            projection
                .apply_event(
                    &event(
                        10,
                        DelegationEventType::Other("future_scheduler_event".to_owned()),
                        None,
                        json!({"epoch": 2}),
                    ),
                    Some(&group),
                )
                .len(),
            1
        );
        assert_eq!(projection.cursor(), 10);
    }

    #[test]
    fn event_type_round_trips_unknown_names() {
        let event_type: DelegationEventType =
            serde_json::from_str("\"future_scheduler_event\"").expect("unknown event");
        assert_eq!(event_type.as_str(), "future_scheduler_event");
        assert_eq!(
            serde_json::to_string(&event_type).expect("serialize event"),
            "\"future_scheduler_event\""
        );
        assert_eq!(
            allowlisted_event_metadata(&json!({"secret": "do not render"})),
            ""
        );
        assert_eq!(
            allowlisted_event_metadata(&json!({"state": "retrying"})),
            " · state=retrying"
        );
    }
}
