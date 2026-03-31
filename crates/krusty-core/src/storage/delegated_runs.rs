//! Persisted delegated run records for first-class subagent lifecycle tracking.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::database::Database;
use crate::agent::{DelegatedRunStage, DelegatedToolKind};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DelegatedRunRole {
    Explore,
    Build,
    Planner,
    Verifier,
}

impl DelegatedRunRole {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Explore => "explore",
            Self::Build => "build",
            Self::Planner => "planner",
            Self::Verifier => "verifier",
        }
    }

    fn from_str(value: &str) -> Option<Self> {
        match value {
            "explore" => Some(Self::Explore),
            "build" => Some(Self::Build),
            "planner" => Some(Self::Planner),
            "verifier" => Some(Self::Verifier),
            _ => None,
        }
    }
}

impl From<DelegatedToolKind> for DelegatedRunRole {
    fn from(value: DelegatedToolKind) -> Self {
        match value {
            DelegatedToolKind::Explore => Self::Explore,
            DelegatedToolKind::Build => Self::Build,
            DelegatedToolKind::Plan => Self::Planner,
            DelegatedToolKind::Verify => Self::Verifier,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedRunScope {
    pub label: String,
    pub path: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedRunAgentSnapshot {
    pub task_id: String,
    pub agent_name: String,
    pub status: String,
    pub tool_count: usize,
    pub tokens: usize,
    pub current_action: Option<String>,
    pub completion_summary: Option<String>,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub completed_plan_task: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DelegatedRunSnapshot {
    pub stage: DelegatedRunStage,
    #[serde(default)]
    pub agents: Vec<DelegatedRunAgentSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DelegatedRunRecord {
    pub delegated_run_id: String,
    pub parent_session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub role: DelegatedRunRole,
    pub stage: DelegatedRunStage,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub resumable: bool,
    pub resumed_from_run_id: Option<String>,
    pub target_scope_key: String,
    pub target_scope: Vec<DelegatedRunScope>,
    pub snapshot: Option<DelegatedRunSnapshot>,
    pub artifact: Option<Value>,
    pub human_review: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct DelegatedRunStartInput {
    pub delegated_run_id: String,
    pub parent_session_id: String,
    pub parent_tool_call_id: Option<String>,
    pub role: DelegatedRunRole,
    pub stage: DelegatedRunStage,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub resumable: bool,
    pub resumed_from_run_id: Option<String>,
    pub target_scope: Vec<DelegatedRunScope>,
}

pub struct DelegatedRunStore {
    db: Database,
}

impl DelegatedRunStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn create_run(&self, input: &DelegatedRunStartInput) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let scope_key = normalize_scope_key(&input.target_scope);
        let scope_json = serde_json::to_string(&input.target_scope)?;

        self.db.conn().execute(
            "INSERT OR REPLACE INTO delegated_runs (
                delegated_run_id,
                parent_session_id,
                parent_tool_call_id,
                role,
                stage,
                provider,
                model,
                resumable,
                resumed_from_run_id,
                target_scope_key,
                target_scope_json,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                input.delegated_run_id,
                input.parent_session_id,
                input.parent_tool_call_id,
                input.role.as_str(),
                delegated_stage_str(input.stage),
                input.provider,
                input.model,
                if input.resumable { 1 } else { 0 },
                input.resumed_from_run_id,
                scope_key,
                scope_json,
                now,
                now,
            ],
        )?;
        Ok(())
    }

    pub fn update_snapshot(
        &self,
        delegated_run_id: &str,
        stage: DelegatedRunStage,
        snapshot: &DelegatedRunSnapshot,
    ) -> Result<()> {
        let updated_at = Utc::now().to_rfc3339();
        let snapshot_json = serde_json::to_string(snapshot)?;
        self.db.conn().execute(
            "UPDATE delegated_runs
             SET stage = ?2,
                 snapshot_json = ?3,
                 updated_at = ?4
             WHERE delegated_run_id = ?1",
            params![
                delegated_run_id,
                delegated_stage_str(stage),
                snapshot_json,
                updated_at,
            ],
        )?;
        Ok(())
    }

    pub fn finalize_run(
        &self,
        delegated_run_id: &str,
        stage: DelegatedRunStage,
        artifact: &Value,
        human_review: Option<&str>,
        resumable: bool,
    ) -> Result<()> {
        let updated_at = Utc::now().to_rfc3339();
        let artifact_json = serde_json::to_string(artifact)?;
        let completed_at = if matches!(
            stage,
            DelegatedRunStage::Complete
                | DelegatedRunStage::Degraded
                | DelegatedRunStage::Failed
                | DelegatedRunStage::Cancelled
        ) {
            Some(updated_at.clone())
        } else {
            None
        };

        self.db.conn().execute(
            "UPDATE delegated_runs
             SET stage = ?2,
                 artifact_json = ?3,
                 human_review = ?4,
                 resumable = ?5,
                 updated_at = ?6,
                 completed_at = COALESCE(?7, completed_at)
             WHERE delegated_run_id = ?1",
            params![
                delegated_run_id,
                delegated_stage_str(stage),
                artifact_json,
                human_review,
                if resumable { 1 } else { 0 },
                updated_at,
                completed_at,
            ],
        )?;
        Ok(())
    }

    pub fn get_run(&self, delegated_run_id: &str) -> Result<Option<DelegatedRunRecord>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT
                delegated_run_id,
                parent_session_id,
                parent_tool_call_id,
                role,
                stage,
                provider,
                model,
                resumable,
                resumed_from_run_id,
                target_scope_key,
                target_scope_json,
                snapshot_json,
                artifact_json,
                human_review,
                created_at,
                updated_at,
                completed_at
             FROM delegated_runs
             WHERE delegated_run_id = ?1",
        )?;

        stmt.query_row(params![delegated_run_id], row_to_delegated_run)
            .optional()
            .map_err(Into::into)
    }

    pub fn list_runs_for_session(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<DelegatedRunRecord>> {
        let limit = limit.max(1) as i64;
        let mut stmt = self.db.conn().prepare(
            "SELECT
                delegated_run_id,
                parent_session_id,
                parent_tool_call_id,
                role,
                stage,
                provider,
                model,
                resumable,
                resumed_from_run_id,
                target_scope_key,
                target_scope_json,
                snapshot_json,
                artifact_json,
                human_review,
                created_at,
                updated_at,
                completed_at
             FROM delegated_runs
             WHERE parent_session_id = ?1
             ORDER BY updated_at DESC
             LIMIT ?2",
        )?;

        let rows = stmt.query_map(params![session_id, limit], row_to_delegated_run)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row?);
        }
        Ok(records)
    }

    pub fn find_related_run(
        &self,
        session_id: &str,
        role: DelegatedRunRole,
        target_scope: &[DelegatedRunScope],
    ) -> Result<Option<DelegatedRunRecord>> {
        let target_scope_key = normalize_scope_key(target_scope);
        let mut stmt = self.db.conn().prepare(
            "SELECT
                delegated_run_id,
                parent_session_id,
                parent_tool_call_id,
                role,
                stage,
                provider,
                model,
                resumable,
                resumed_from_run_id,
                target_scope_key,
                target_scope_json,
                snapshot_json,
                artifact_json,
                human_review,
                created_at,
                updated_at,
                completed_at
             FROM delegated_runs
             WHERE parent_session_id = ?1
               AND role = ?2
               AND target_scope_key = ?3
               AND stage NOT IN ('created', 'running')
             ORDER BY updated_at DESC
             LIMIT 1",
        )?;

        stmt.query_row(
            params![session_id, role.as_str(), target_scope_key],
            row_to_delegated_run,
        )
        .optional()
        .map_err(Into::into)
    }
}

pub fn normalize_scope_key(target_scope: &[DelegatedRunScope]) -> String {
    let mut entries = target_scope
        .iter()
        .map(|scope| {
            format!(
                "{}|{}|{}",
                scope.kind.trim(),
                scope.label.trim(),
                scope.path.trim()
            )
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries.join("\n")
}

fn delegated_stage_str(stage: DelegatedRunStage) -> &'static str {
    match stage {
        DelegatedRunStage::Created => "created",
        DelegatedRunStage::Running => "running",
        DelegatedRunStage::Synthesizing => "synthesizing",
        DelegatedRunStage::Complete => "complete",
        DelegatedRunStage::Degraded => "degraded",
        DelegatedRunStage::Failed => "failed",
        DelegatedRunStage::Cancelled => "cancelled",
    }
}

fn parse_stage(value: &str) -> rusqlite::Result<DelegatedRunStage> {
    match value {
        "created" => Ok(DelegatedRunStage::Created),
        "running" => Ok(DelegatedRunStage::Running),
        "synthesizing" => Ok(DelegatedRunStage::Synthesizing),
        "complete" => Ok(DelegatedRunStage::Complete),
        "degraded" => Ok(DelegatedRunStage::Degraded),
        "failed" => Ok(DelegatedRunStage::Failed),
        "cancelled" => Ok(DelegatedRunStage::Cancelled),
        other => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            format!("unknown delegated stage: {}", other).into(),
        )),
    }
}

fn parse_datetime(value: String) -> rusqlite::Result<DateTime<Utc>> {
    value.parse::<DateTime<Utc>>().map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, err.into())
    })
}

fn row_to_delegated_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<DelegatedRunRecord> {
    let role = DelegatedRunRole::from_str(&row.get::<_, String>(3)?).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Text,
            "invalid delegated role".into(),
        )
    })?;
    let stage = parse_stage(&row.get::<_, String>(4)?)?;
    let target_scope_json: String = row.get(10)?;
    let target_scope =
        serde_json::from_str::<Vec<DelegatedRunScope>>(&target_scope_json).map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(10, rusqlite::types::Type::Text, err.into())
        })?;
    let snapshot = row
        .get::<_, Option<String>>(11)?
        .map(|value| serde_json::from_str::<DelegatedRunSnapshot>(&value))
        .transpose()
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, err.into())
        })?;
    let artifact = row
        .get::<_, Option<String>>(12)?
        .map(|value| serde_json::from_str::<Value>(&value))
        .transpose()
        .map_err(|err| {
            rusqlite::Error::FromSqlConversionFailure(12, rusqlite::types::Type::Text, err.into())
        })?;

    Ok(DelegatedRunRecord {
        delegated_run_id: row.get(0)?,
        parent_session_id: row.get(1)?,
        parent_tool_call_id: row.get(2)?,
        role,
        stage,
        provider: row.get(5)?,
        model: row.get(6)?,
        resumable: row.get::<_, i64>(7)? != 0,
        resumed_from_run_id: row.get(8)?,
        target_scope_key: row.get(9)?,
        target_scope,
        snapshot,
        artifact,
        human_review: row.get(13)?,
        created_at: parse_datetime(row.get(14)?)?,
        updated_at: parse_datetime(row.get(15)?)?,
        completed_at: row
            .get::<_, Option<String>>(16)?
            .map(parse_datetime)
            .transpose()?,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::storage::Database;

    fn create_store() -> (DelegatedRunStore, TempDir) {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("delegated-runs.db");
        let db = Database::new(&db_path).expect("db");
        let now = Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params!["session-1", "Delegated Run Test", now, now],
            )
            .expect("seed session");
        (DelegatedRunStore::new(db), temp_dir)
    }

    fn scope(label: &str, path: &str, kind: &str) -> DelegatedRunScope {
        DelegatedRunScope {
            label: label.to_string(),
            path: path.to_string(),
            kind: kind.to_string(),
        }
    }

    #[test]
    fn round_trip_persisted_delegated_run() {
        let (store, _tmp) = create_store();
        let delegated_run_id = "run-1".to_string();
        store
            .create_run(&DelegatedRunStartInput {
                delegated_run_id: delegated_run_id.clone(),
                parent_session_id: "session-1".to_string(),
                parent_tool_call_id: Some("tool-1".to_string()),
                role: DelegatedRunRole::Explore,
                stage: DelegatedRunStage::Created,
                provider: Some("minimax".to_string()),
                model: Some("MiniMax-M2.5".to_string()),
                resumable: true,
                resumed_from_run_id: None,
                target_scope: vec![scope(
                    "storage",
                    "crates/krusty-core/src/storage",
                    "directory",
                )],
            })
            .expect("create run");

        store
            .update_snapshot(
                &delegated_run_id,
                DelegatedRunStage::Running,
                &DelegatedRunSnapshot {
                    stage: DelegatedRunStage::Running,
                    agents: vec![DelegatedRunAgentSnapshot {
                        task_id: "dir-0".to_string(),
                        agent_name: "storage".to_string(),
                        status: "running".to_string(),
                        tool_count: 3,
                        tokens: 120,
                        current_action: Some("reading mod.rs".to_string()),
                        completion_summary: None,
                        lines_added: 0,
                        lines_removed: 0,
                        completed_plan_task: None,
                    }],
                },
            )
            .expect("update snapshot");

        store
            .finalize_run(
                &delegated_run_id,
                DelegatedRunStage::Complete,
                &serde_json::json!({
                    "human_review": "Storage owns session persistence and runtime traces."
                }),
                Some("Storage owns session persistence and runtime traces."),
                true,
            )
            .expect("finalize run");

        let record = store
            .get_run(&delegated_run_id)
            .expect("get run")
            .expect("record exists");
        assert_eq!(record.stage, DelegatedRunStage::Complete);
        assert_eq!(record.role, DelegatedRunRole::Explore);
        assert!(record.resumable);
        assert_eq!(record.target_scope.len(), 1);
        assert_eq!(
            record.snapshot.as_ref().unwrap().agents[0].agent_name,
            "storage"
        );
        assert_eq!(
            record.human_review.as_deref(),
            Some("Storage owns session persistence and runtime traces.")
        );
    }

    #[test]
    fn find_related_run_matches_scope_key() {
        let (store, _tmp) = create_store();
        let scope = vec![scope("agent", "crates/krusty-core/src/agent", "directory")];
        store
            .create_run(&DelegatedRunStartInput {
                delegated_run_id: "run-1".to_string(),
                parent_session_id: "session-1".to_string(),
                parent_tool_call_id: Some("tool-1".to_string()),
                role: DelegatedRunRole::Explore,
                stage: DelegatedRunStage::Complete,
                provider: None,
                model: None,
                resumable: true,
                resumed_from_run_id: None,
                target_scope: scope.clone(),
            })
            .expect("create run");

        let found = store
            .find_related_run("session-1", DelegatedRunRole::Explore, &scope)
            .expect("query related")
            .expect("related run");
        assert_eq!(found.delegated_run_id, "run-1");
    }
}
