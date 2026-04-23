use std::collections::BTreeMap;

use krusty_core::storage::{MemoryStore, ReportStore, SessionInfo, CURRENT_SNAPSHOT_TITLE};

use crate::error::AppError;

use super::diagnostics::{later_timestamp, parse_timestamp};
use super::MakoKnowledgeHealthSummary;

pub(super) fn summarize_knowledge_health(
    memory_store: &MemoryStore,
    report_store: &ReportStore,
    sessions: &[SessionInfo],
    user_id: Option<&str>,
) -> Result<MakoKnowledgeHealthSummary, AppError> {
    let mut scopes = BTreeMap::<Option<String>, Vec<&SessionInfo>>::new();
    for session in sessions {
        scopes
            .entry(session.project_dir.clone())
            .or_default()
            .push(session);
    }

    let mut missing_snapshot_count = 0usize;
    let mut stale_snapshot_count = 0usize;
    let mut latest_snapshot_at: Option<String> = None;

    for (scope, scoped_sessions) in &scopes {
        let latest_session_at = scoped_sessions
            .iter()
            .map(|session| session.updated_at.to_rfc3339())
            .max();
        let latest_report_at = report_store
            .list_reports_for_user(scope.as_deref(), user_id)?
            .first()
            .map(|report| report.created_at.clone());
        let latest_signal_at = later_timestamp(latest_session_at, latest_report_at);
        let snapshot = memory_store.find_by_title_in_exact_scope(
            CURRENT_SNAPSHOT_TITLE,
            scope.as_deref(),
            user_id,
        );

        match snapshot {
            Some(snapshot) => {
                latest_snapshot_at =
                    later_timestamp(latest_snapshot_at, Some(snapshot.updated_at.clone()));
                if latest_signal_at
                    .as_deref()
                    .zip(parse_timestamp(&snapshot.updated_at))
                    .and_then(|(signal, snapshot_at)| {
                        parse_timestamp(signal).map(|signal_at| signal_at > snapshot_at)
                    })
                    .unwrap_or(false)
                {
                    stale_snapshot_count += 1;
                }
            }
            None => missing_snapshot_count += 1,
        }
    }

    let scope_count = scopes.len();
    let healthy_scope_count = scope_count
        .saturating_sub(missing_snapshot_count)
        .saturating_sub(stale_snapshot_count);

    Ok(MakoKnowledgeHealthSummary {
        scope_count,
        healthy_scope_count,
        missing_snapshot_count,
        stale_snapshot_count,
        latest_snapshot_at,
    })
}
