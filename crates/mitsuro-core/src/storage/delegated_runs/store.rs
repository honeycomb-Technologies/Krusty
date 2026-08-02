use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use std::collections::BTreeSet;

use crate::agent::subagent::AgentCapability;
use crate::agent::DelegatedRunStage;
use crate::storage::database::Database;

use super::codec::{delegated_stage_str, row_to_delegated_run};
use super::model::{
    normalize_scope_key, DelegatedRunRecord, DelegatedRunRole, DelegatedRunScope,
    DelegatedRunSnapshot, DelegatedRunStartInput,
};

pub struct DelegatedRunStore {
    pub(super) db: Database,
}

impl DelegatedRunStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn create_run(&self, input: &DelegatedRunStartInput) -> Result<()> {
        self.create_run_with_child_contract(input, None, &BTreeSet::new())
    }

    pub fn create_run_with_child_contract(
        &self,
        input: &DelegatedRunStartInput,
        child_name: Option<&str>,
        capabilities: &BTreeSet<AgentCapability>,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let scope_key = normalize_scope_key(&input.target_scope);
        let scope_json = serde_json::to_string(&input.target_scope)?;
        let capabilities_json = serde_json::to_string(capabilities)?;

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
                child_name,
                capabilities_json,
                created_at,
                updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
                child_name,
                capabilities_json,
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
        // An explicit parent interrupt is authoritative. A child may observe
        // cancellation slightly later and attempt normal failure finalization;
        // never let that race erase the durable cancelled state.
        if stage != DelegatedRunStage::Cancelled
            && self
                .get_run(delegated_run_id)?
                .is_some_and(|record| record.stage == DelegatedRunStage::Cancelled)
        {
            return Ok(());
        }

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
                ,child_name
                ,capabilities_json
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
                ,child_name
                ,capabilities_json
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
                ,child_name
                ,capabilities_json
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
