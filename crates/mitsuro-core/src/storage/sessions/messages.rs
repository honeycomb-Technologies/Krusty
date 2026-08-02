use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use rusqlite::OptionalExtension;

use crate::storage::DelegatedRunScope;

use super::SessionManager;

impl SessionManager {
    /// Save a message to a session
    /// The content field stores JSON-serialized Vec<Content> for full fidelity
    pub fn save_message(&self, session_id: &str, role: &str, content_json: &str) -> Result<()> {
        super::super::messages::MessageStore::new(&self.db).save_message(
            session_id,
            role,
            content_json,
        )
    }

    pub fn queue_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
        content_json: &str,
    ) -> Result<()> {
        super::super::messages::MessageStore::new(&self.db).queue_pending_steering(
            session_id,
            pending_id,
            content_json,
        )
    }

    pub fn queue_pending_steering_once(
        &self,
        session_id: &str,
        pending_id: &str,
        content_json: &str,
    ) -> Result<bool> {
        super::super::messages::MessageStore::new(&self.db).queue_pending_steering_once(
            session_id,
            pending_id,
            content_json,
        )
    }

    pub fn has_pending_steering(&self, session_id: &str, pending_id: &str) -> Result<bool> {
        super::super::messages::MessageStore::new(&self.db)
            .has_pending_steering(session_id, pending_id)
    }

    pub fn load_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
    ) -> Result<Option<String>> {
        super::super::messages::MessageStore::new(&self.db)
            .load_pending_steering(session_id, pending_id)
    }

    pub fn promote_pending_steering(
        &self,
        session_id: &str,
        pending_id: &str,
    ) -> Result<Option<String>> {
        let store = super::super::messages::MessageStore::new(&self.db);
        if !pending_id.starts_with("child-wake-") {
            return store.promote_pending_steering(session_id, pending_id);
        }
        if store
            .load_pending_steering(session_id, pending_id)?
            .is_none()
        {
            return Ok(None);
        }

        let launch_workspace = self.authoritative_child_wake_workspace(session_id, pending_id)?;
        store.promote_pending_child_wake(session_id, pending_id, &launch_workspace)
    }

    pub fn promote_orphaned_pending_steering(&self, session_id: &str) -> Result<usize> {
        super::super::messages::MessageStore::new(&self.db)
            .promote_orphaned_pending_steering(session_id)
    }

    /// Resolve the immutable workspace carried by an existing durable child
    /// wake. Ordinary pending steering has no such authority contract and must
    /// never be allowed to manufacture one from a caller-supplied path.
    fn authoritative_child_wake_workspace(
        &self,
        session_id: &str,
        pending_id: &str,
    ) -> Result<PathBuf> {
        let delegated_run_id = pending_id
            .strip_prefix("child-wake-")
            .filter(|run_id| !run_id.is_empty())
            .context("child completion pending ID has no delegated run")?;
        let durable = self
            .db
            .conn()
            .query_row(
                "SELECT parent_session_id,
                        stage,
                        target_scope_json,
                        artifact_json,
                        human_review,
                        completed_at,
                        wake_parent
                   FROM delegated_runs
                  WHERE delegated_run_id = ?1",
                [delegated_run_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, i64>(6)? != 0,
                    ))
                },
            )
            .optional()?
            .context("child completion references an unknown delegated run")?;
        let (
            parent_session_id,
            stage,
            target_scope_json,
            artifact_json,
            human_review,
            completed_at,
            wake_parent,
        ) = durable;
        ensure!(
            parent_session_id == session_id,
            "child completion delegated run belongs to a different parent session"
        );
        let artifact = artifact_json
            .as_deref()
            .map(serde_json::from_str::<serde_json::Value>)
            .transpose()
            .context("decoding child completion durable artifact")?;
        let publishable = match stage.as_str() {
            // Existing pending rows with these stages are also the migration
            // compatibility proof for wakes written before `wake_parent` was
            // persisted. New rows require `wake_parent` before being queued.
            "complete" | "degraded" | "failed" => true,
            "cancelled" if wake_parent => {
                matches!(
                    artifact
                        .as_ref()
                        .and_then(|value| value.get("outcome_reason"))
                        .and_then(serde_json::Value::as_str),
                    Some("caller_aborted_before_terminal" | "background_host_lease_expired")
                )
            }
            _ => false,
        };
        ensure!(
            publishable,
            "child completion delegated run is not publishable"
        );
        ensure!(
            artifact.is_some() && human_review.is_some() && completed_at.is_some(),
            "child completion delegated run has no durable terminal artifact"
        );

        let scopes: Vec<DelegatedRunScope> = serde_json::from_str(&target_scope_json)
            .context("decoding child completion durable target scope")?;
        let workspace_scopes = scopes
            .iter()
            .filter(|scope| scope.kind == "workspace")
            .collect::<Vec<_>>();
        let [workspace_scope] = workspace_scopes.as_slice() else {
            anyhow::bail!("child completion delegated run has no unique launch workspace");
        };
        let launch_workspace = PathBuf::from(&workspace_scope.path)
            .canonicalize()
            .context("canonicalizing child completion durable launch workspace")?;
        ensure!(
            launch_workspace.is_dir(),
            "child completion durable launch workspace is not a directory"
        );
        Ok(launch_workspace)
    }

    /// Replace every persisted message for a session with a new ordered set.
    pub fn replace_session_messages(
        &self,
        session_id: &str,
        messages: &[(String, String)],
    ) -> Result<()> {
        super::super::messages::MessageStore::new(&self.db)
            .replace_session_messages(session_id, messages)
    }

    /// Update the most recent message of a given role in a session
    pub fn update_last_message(
        &self,
        session_id: &str,
        role: &str,
        content_json: &str,
    ) -> Result<()> {
        super::super::messages::MessageStore::new(&self.db).update_last_message(
            session_id,
            role,
            content_json,
        )
    }

    /// Load all messages for a session
    /// Returns (role, content_json) pairs where content_json can be deserialized to Vec<Content>
    pub fn load_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        super::super::messages::MessageStore::new(&self.db).load_session_messages(session_id)
    }

    /// Generate a title from the first message content
    /// using the same zero-token, Unicode-safe contract as every client.
    pub fn generate_title_from_content(content: &str) -> String {
        crate::ai::derive_title(content)
    }
}
