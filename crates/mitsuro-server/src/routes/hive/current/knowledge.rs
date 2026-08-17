use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Utc};

use mitsuro_core::storage::{get_current_snapshot, ReportStore, SessionInfo};

use crate::error::AppError;

use super::diagnostics::parse_timestamp;
use super::HiveKnowledgeHealthSummary;

pub(super) fn summarize_knowledge_health(
    db_path: &Path,
    report_store: &ReportStore,
    sessions: &[SessionInfo],
    user_id: Option<&str>,
) -> Result<HiveKnowledgeHealthSummary, AppError> {
    let mut scopes = BTreeMap::<Option<String>, Vec<&SessionInfo>>::new();
    for session in sessions {
        scopes
            .entry(session.project_dir.clone())
            .or_default()
            .push(session);
    }

    let mut missing_snapshot_count = 0usize;
    let mut stale_snapshot_count = 0usize;
    let mut latest_snapshot_at: Option<DateTime<Utc>> = None;

    for (scope, scoped_sessions) in &scopes {
        let latest_session_at = scoped_sessions
            .iter()
            .map(|session| session.updated_at)
            .max();
        let latest_report_at = report_store
            .list_reports_for_user(scope.as_deref(), user_id)?
            .iter()
            .filter_map(|report| parse_utc_timestamp(&report.created_at))
            .max();
        let latest_signal_at = latest_session_at.into_iter().chain(latest_report_at).max();

        // Migration 39 moved generated snapshots out of `agent_memories` into
        // the dedicated `knowledge_snapshots` store. Health must read the same
        // store the refresh path writes, otherwise every scope reports a
        // missing snapshot forever.
        match get_current_snapshot(db_path, scope.as_deref(), user_id)? {
            Some(snapshot) => {
                let snapshot_at = parse_utc_timestamp(&snapshot.updated_at);
                latest_snapshot_at = latest_snapshot_at.into_iter().chain(snapshot_at).max();
                // The snapshot store keeps second-precision timestamps, so
                // compare at second precision: a snapshot refreshed within the
                // same second as the newest signal is fresh, not stale.
                if latest_signal_at
                    .zip(snapshot_at)
                    .is_some_and(|(signal_at, snapshot_at)| {
                        signal_at.timestamp() > snapshot_at.timestamp()
                    })
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

    Ok(HiveKnowledgeHealthSummary {
        scope_count,
        healthy_scope_count,
        missing_snapshot_count,
        stale_snapshot_count,
        latest_snapshot_at: latest_snapshot_at.map(|at| at.to_rfc3339()),
    })
}

/// Snapshot and report rows keep SQLite `datetime('now')` timestamps
/// (`YYYY-MM-DD HH:MM:SS`, UTC) while session state uses RFC 3339. Accept both
/// so freshness comparisons never silently degrade to "never stale".
fn parse_utc_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    parse_timestamp(raw).or_else(|| {
        chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
            .ok()
            .map(|naive| naive.and_utc())
    })
}

#[cfg(test)]
mod tests {
    use super::parse_utc_timestamp;

    #[test]
    fn parses_sqlite_and_rfc3339_timestamps_as_utc() {
        let sqlite = parse_utc_timestamp("2026-08-16 06:42:34").expect("sqlite format");
        let rfc3339 = parse_utc_timestamp("2026-08-16T06:42:34Z").expect("rfc3339 format");
        assert_eq!(sqlite, rfc3339);
        assert!(parse_utc_timestamp("not a timestamp").is_none());
    }
}
