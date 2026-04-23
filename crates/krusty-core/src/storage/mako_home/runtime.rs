use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::Result;
use chrono::{DateTime, NaiveDateTime};

use crate::storage::{
    AutonomousTaskStore, DelegatedRunStore, MakoRuntimeState, MakoRuntimeStateStatus, SessionInfo,
    TaskStatus,
};

use super::{MakoCrewRuntimeStatus, MakoCrewRuntimeSummary, MakoHomeProfile};
use crate::agent::DelegatedRunStage;

pub fn summarize_crew_runtime(
    profile: &MakoHomeProfile,
    sessions: &[SessionInfo],
    runtime_states: &HashMap<String, MakoRuntimeState>,
    task_store: &AutonomousTaskStore,
    delegated_store: &DelegatedRunStore,
) -> Result<Vec<MakoCrewRuntimeSummary>> {
    let known_slugs = profile
        .crew
        .iter()
        .map(|member| normalize_agent_key(&member.slug))
        .collect::<HashSet<_>>();
    let mut summaries = BTreeMap::<String, MakoCrewRuntimeSummary>::new();

    for member in &profile.crew {
        summaries.insert(
            normalize_agent_key(&member.slug),
            MakoCrewRuntimeSummary {
                slug: member.slug.clone(),
                known_to_home: true,
                ..Default::default()
            },
        );
    }

    for session in sessions {
        if let Some(runtime_state) = runtime_states.get(session.id.as_str()) {
            if let Some(crew_slug) = runtime_state.crew_slug.as_deref() {
                let key = normalize_agent_key(crew_slug);
                if !key.is_empty() {
                    let summary = summaries.entry(key.clone()).or_insert_with(|| {
                        new_runtime_summary(crew_slug, known_slugs.contains(&key))
                    });
                    summary.recent_run_count += 1;
                    match runtime_state.status {
                        MakoRuntimeStateStatus::Running
                        | MakoRuntimeStateStatus::AwaitingInput
                        | MakoRuntimeStateStatus::Paused => {
                            summary.active_run_count += 1;
                        }
                        MakoRuntimeStateStatus::Error => {
                            summary.failed_run_count += 1;
                        }
                        _ => {}
                    }
                    record_latest_activity(
                        &mut summary.latest_activity_at,
                        runtime_state.updated_at.as_str(),
                    );
                }
            }
        }

        for task in task_store.list_tasks(&session.id)? {
            let Some(owner) = task.owner.as_deref() else {
                continue;
            };
            let key = normalize_agent_key(owner);
            if key.is_empty() {
                continue;
            }
            let summary = summaries
                .entry(key.clone())
                .or_insert_with(|| new_runtime_summary(owner, known_slugs.contains(&key)));

            match task.status {
                TaskStatus::Pending => summary.queued_task_count += 1,
                TaskStatus::InProgress => summary.active_task_count += 1,
                TaskStatus::Completed => summary.completed_task_count += 1,
                TaskStatus::Failed => summary.failed_task_count += 1,
            }

            let task_activity = task
                .completed_at
                .as_deref()
                .unwrap_or(task.updated_at.as_str());
            record_latest_activity(&mut summary.latest_activity_at, task_activity);
        }

        for run in delegated_store.list_runs_for_session(&session.id, 100)? {
            let Some(snapshot) = run.snapshot.as_ref() else {
                continue;
            };
            for agent in &snapshot.agents {
                let key = normalize_agent_key(&agent.agent_name);
                if key.is_empty() {
                    continue;
                }
                let summary = summaries.entry(key.clone()).or_insert_with(|| {
                    new_runtime_summary(&agent.agent_name, known_slugs.contains(&key))
                });

                summary.recent_run_count += 1;
                if matches!(
                    run.stage,
                    DelegatedRunStage::Created
                        | DelegatedRunStage::Running
                        | DelegatedRunStage::Synthesizing
                ) || matches!(agent.status.as_str(), "running" | "pending")
                {
                    summary.active_run_count += 1;
                }
                if matches!(
                    run.stage,
                    DelegatedRunStage::Failed | DelegatedRunStage::Degraded
                ) || agent.status.eq_ignore_ascii_case("failed")
                {
                    summary.failed_run_count += 1;
                }
                record_latest_activity(
                    &mut summary.latest_activity_at,
                    &run.updated_at.to_rfc3339(),
                );
            }
        }
    }

    let mut values = summaries.into_values().collect::<Vec<_>>();
    for summary in &mut values {
        summary.status = resolve_runtime_status(summary);
    }
    values.sort_by(
        |left, right| match (left.known_to_home, right.known_to_home) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => left.slug.cmp(&right.slug),
        },
    );
    Ok(values)
}

fn new_runtime_summary(slug: &str, known_to_home: bool) -> MakoCrewRuntimeSummary {
    MakoCrewRuntimeSummary {
        slug: slug.trim().to_string(),
        known_to_home,
        ..Default::default()
    }
}

fn resolve_runtime_status(summary: &MakoCrewRuntimeSummary) -> MakoCrewRuntimeStatus {
    if summary.active_run_count > 0 || summary.active_task_count > 0 {
        MakoCrewRuntimeStatus::Running
    } else if summary.failed_run_count > 0 || summary.failed_task_count > 0 {
        MakoCrewRuntimeStatus::Degraded
    } else if summary.queued_task_count > 0 {
        MakoCrewRuntimeStatus::Waiting
    } else {
        MakoCrewRuntimeStatus::Idle
    }
}

fn record_latest_activity(current: &mut Option<String>, candidate: &str) {
    if candidate.trim().is_empty() {
        return;
    }
    match current {
        Some(existing) => {
            if timestamp_sort_key(candidate) > timestamp_sort_key(existing) {
                *existing = candidate.to_string();
            }
        }
        None => *current = Some(candidate.to_string()),
    }
}

fn timestamp_sort_key(value: &str) -> i64 {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
        return parsed.timestamp();
    }
    if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
        return parsed.and_utc().timestamp();
    }
    i64::MIN
}

fn normalize_agent_key(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
