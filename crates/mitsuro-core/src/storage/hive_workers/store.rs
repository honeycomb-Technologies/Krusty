use std::io::{Error as IoError, ErrorKind};

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row};

use crate::ai::models::ModelKey;
use crate::storage::Database;
use crate::tools::registry::PermissionMode;

use super::super::hive_home::is_valid_crew_slug;
use super::super::hive_profiles::MAX_HIVE_PROFILE_DOCUMENT_BYTES;
use super::model::{
    display_name_from_slug, HiveWorker, HiveWorkerAutonomy, HiveWorkerDocument,
    HiveWorkerDocumentKind, HiveWorkerProfileUpdate, HiveWorkerStatus, NewHiveWorker,
};

pub(crate) const WORKER_COLUMNS: &str = "id, user_id, slug, display_name, avatar_color, model, model_key_json, model_catalog_revision, permission_mode, autonomy, heartbeat_interval_secs, status, dm_session_id, memory_namespace_id, created_at, updated_at";

/// Matches rows owned by exactly the given user (NULL = local), mirroring
/// the exact-owner semantics used across the rest of the hive stores.
const OWNER_PREDICATE: &str = "((?1 IS NULL AND user_id IS NULL) OR user_id = ?1)";

pub struct HiveWorkerStore {
    db: Database,
}

impl HiveWorkerStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn create(&self, input: &NewHiveWorker) -> Result<HiveWorker> {
        let slug = input.slug.trim();
        anyhow::ensure!(is_valid_crew_slug(slug), "invalid Hive worker slug: {slug}");
        let memory_namespace_id = match input.memory_namespace_id.as_deref().map(str::trim) {
            Some("") | None => slug.to_string(),
            Some(namespace_id) => namespace_id.to_string(),
        };
        let display_name = match input.display_name.as_deref().map(str::trim) {
            Some("") | None => display_name_from_slug(slug),
            Some(display_name) => display_name.to_string(),
        };
        validate_model_identity(input.model.as_deref(), input.model_key.as_ref())?;
        if let Some(interval) = input.heartbeat_interval_secs {
            anyhow::ensure!(interval > 0, "heartbeat interval must be positive");
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let model_key_json = input
            .model_key
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("encoding Hive worker model key")?;
        self.db
            .conn()
            .execute(
                "INSERT INTO hive_workers (
                    id, user_id, slug, display_name, avatar_color, model,
                    model_key_json, model_catalog_revision, permission_mode,
                    autonomy, heartbeat_interval_secs, status, dm_session_id,
                    memory_namespace_id, created_at, updated_at
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'active',
                    ?12, ?13, ?14, ?14
                 )",
                params![
                    id,
                    input.user_id,
                    slug,
                    display_name,
                    input.avatar_color,
                    input.model,
                    model_key_json,
                    input.model_catalog_revision,
                    input.permission_mode.as_str(),
                    input.autonomy.as_str(),
                    input.heartbeat_interval_secs,
                    input.dm_session_id,
                    memory_namespace_id,
                    now,
                ],
            )
            .context("inserting Hive worker")?;
        self.get(&id)?
            .ok_or_else(|| anyhow::anyhow!("failed to load newly created Hive worker {id}"))
    }

    pub fn get(&self, id: &str) -> Result<Option<HiveWorker>> {
        let sql = format!("SELECT {WORKER_COLUMNS} FROM hive_workers WHERE id = ?1");
        self.db
            .conn()
            .query_row(&sql, [id], map_worker)
            .optional()
            .context("reading Hive worker")
    }

    /// Fetch the Worker whose private DM lane is this session. The schema
    /// keeps `dm_session_id` unique, so at most one Worker can own a session.
    pub fn get_by_dm_session(&self, session_id: &str) -> Result<Option<HiveWorker>> {
        let sql = format!("SELECT {WORKER_COLUMNS} FROM hive_workers WHERE dm_session_id = ?1");
        self.db
            .conn()
            .query_row(&sql, [session_id], map_worker)
            .optional()
            .context("reading Hive worker by DM session")
    }

    /// Fetch the non-archived Worker with this slug for exactly this owner.
    pub fn get_by_slug(&self, user_id: Option<&str>, slug: &str) -> Result<Option<HiveWorker>> {
        let sql = format!(
            "SELECT {WORKER_COLUMNS} FROM hive_workers
             WHERE {OWNER_PREDICATE} AND slug = ?2 AND status <> 'archived'"
        );
        self.db
            .conn()
            .query_row(&sql, params![user_id, slug], map_worker)
            .optional()
            .context("reading Hive worker by slug")
    }

    /// Fetch any Worker with this slug for exactly this owner, including
    /// archived rows, so callers can distinguish "never existed" from
    /// "archived".
    pub fn get_by_slug_any_status(
        &self,
        user_id: Option<&str>,
        slug: &str,
    ) -> Result<Option<HiveWorker>> {
        let sql = format!(
            "SELECT {WORKER_COLUMNS} FROM hive_workers
             WHERE {OWNER_PREDICATE} AND slug = ?2
             ORDER BY CASE status WHEN 'archived' THEN 1 ELSE 0 END, updated_at DESC
             LIMIT 1"
        );
        self.db
            .conn()
            .query_row(&sql, params![user_id, slug], map_worker)
            .optional()
            .context("reading Hive worker by slug including archived")
    }

    pub fn list_for_owner(
        &self,
        user_id: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<HiveWorker>> {
        let status_predicate = if include_archived {
            ""
        } else {
            " AND status <> 'archived'"
        };
        let sql = format!(
            "SELECT {WORKER_COLUMNS} FROM hive_workers
             WHERE {OWNER_PREDICATE}{status_predicate}
             ORDER BY slug ASC, created_at ASC"
        );
        let mut statement = self.db.conn().prepare(&sql)?;
        let workers = statement
            .query_map(params![user_id], map_worker)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive workers")?;
        Ok(workers)
    }

    /// Overwrite the profile-editable surface of one Worker.
    pub fn update_profile(
        &self,
        id: &str,
        update: &HiveWorkerProfileUpdate,
    ) -> Result<Option<HiveWorker>> {
        let display_name = update.display_name.trim();
        anyhow::ensure!(
            !display_name.is_empty(),
            "Hive worker display name must not be empty"
        );
        validate_model_identity(update.model.as_deref(), update.model_key.as_ref())?;
        let model_key_json = update
            .model_key
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .context("encoding Hive worker model key")?;
        let changed = self.db.conn().execute(
            "UPDATE hive_workers
             SET display_name = ?2, avatar_color = ?3, model = ?4,
                 model_key_json = ?5, model_catalog_revision = ?6,
                 permission_mode = ?7, updated_at = ?8
             WHERE id = ?1",
            params![
                id,
                display_name,
                update.avatar_color,
                update.model,
                model_key_json,
                update.model_catalog_revision,
                update.permission_mode.as_str(),
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        if changed == 0 {
            return Ok(None);
        }
        self.get(id)
    }

    pub fn set_status(&self, id: &str, status: HiveWorkerStatus) -> Result<bool> {
        let changed = self.db.conn().execute(
            "UPDATE hive_workers SET status = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, status.as_str(), chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(changed == 1)
    }

    pub fn set_autonomy(
        &self,
        id: &str,
        autonomy: HiveWorkerAutonomy,
        heartbeat_interval_secs: Option<u32>,
    ) -> Result<bool> {
        if let Some(interval) = heartbeat_interval_secs {
            anyhow::ensure!(interval > 0, "heartbeat interval must be positive");
        }
        let changed = self.db.conn().execute(
            "UPDATE hive_workers
             SET autonomy = ?2, heartbeat_interval_secs = ?3, updated_at = ?4
             WHERE id = ?1",
            params![
                id,
                autonomy.as_str(),
                heartbeat_interval_secs,
                chrono::Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(changed == 1)
    }

    /// Bind (or clear) the Worker's private DM session. The schema keeps the
    /// binding exclusive: one session can be the DM lane of at most one
    /// Worker.
    pub fn bind_dm_session(&self, id: &str, session_id: Option<&str>) -> Result<bool> {
        let changed = self
            .db
            .conn()
            .execute(
                "UPDATE hive_workers SET dm_session_id = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, session_id, chrono::Utc::now().to_rfc3339()],
            )
            .context("binding Hive worker DM session")?;
        Ok(changed == 1)
    }

    /// Dual-read transition helper: resolve a legacy crew slug to the Worker
    /// that inherited it. Prefers an exact slug match, then a Worker whose
    /// memory namespace still carries the crew slug (a renamed Worker keeps
    /// its crew-compatible namespace). Returns None when the slug has no
    /// Worker yet, letting callers keep the legacy crew path.
    pub fn resolve_worker_for_crew_slug(
        &self,
        user_id: Option<&str>,
        crew_slug: &str,
    ) -> Result<Option<HiveWorker>> {
        let sql = format!(
            "SELECT {WORKER_COLUMNS} FROM hive_workers
             WHERE {OWNER_PREDICATE} AND status <> 'archived'
               AND (slug = ?2 OR memory_namespace_id = ?2)
             ORDER BY (slug = ?2) DESC, created_at ASC
             LIMIT 1"
        );
        self.db
            .conn()
            .query_row(&sql, params![user_id, crew_slug], map_worker)
            .optional()
            .context("resolving Hive worker for crew slug")
    }

    pub fn upsert_document(
        &self,
        worker_id: &str,
        kind: HiveWorkerDocumentKind,
        content: &str,
    ) -> Result<()> {
        let content = content.trim();
        anyhow::ensure!(
            !content.is_empty(),
            "Hive worker document must not be empty"
        );
        anyhow::ensure!(
            content.len() <= MAX_HIVE_PROFILE_DOCUMENT_BYTES,
            "Hive worker document exceeds the {MAX_HIVE_PROFILE_DOCUMENT_BYTES}-byte limit"
        );
        self.db
            .conn()
            .execute(
                "INSERT INTO hive_worker_documents (worker_id, kind, content, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(worker_id, kind) DO UPDATE SET
                     content = excluded.content,
                     updated_at = excluded.updated_at",
                params![
                    worker_id,
                    kind.as_str(),
                    content,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )
            .context("writing Hive worker document")?;
        Ok(())
    }

    pub fn document(
        &self,
        worker_id: &str,
        kind: HiveWorkerDocumentKind,
    ) -> Result<Option<HiveWorkerDocument>> {
        self.db
            .conn()
            .query_row(
                "SELECT kind, content, updated_at FROM hive_worker_documents
                 WHERE worker_id = ?1 AND kind = ?2",
                params![worker_id, kind.as_str()],
                map_document,
            )
            .optional()
            .context("reading Hive worker document")
    }

    pub fn documents(&self, worker_id: &str) -> Result<Vec<HiveWorkerDocument>> {
        let mut statement = self.db.conn().prepare(
            "SELECT kind, content, updated_at FROM hive_worker_documents
             WHERE worker_id = ?1
             ORDER BY kind ASC",
        )?;
        let documents = statement
            .query_map([worker_id], map_document)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("listing Hive worker documents")?;
        Ok(documents)
    }

    #[cfg(test)]
    pub(super) fn conn(&self) -> &rusqlite::Connection {
        self.db.conn()
    }
}

pub fn load_worker_with_conn(conn: &rusqlite::Connection, id: &str) -> Result<Option<HiveWorker>> {
    let sql = format!("SELECT {WORKER_COLUMNS} FROM hive_workers WHERE id = ?1");
    conn.query_row(&sql, [id], map_worker)
        .optional()
        .context("reading Hive worker")
}

fn validate_model_identity(model: Option<&str>, model_key: Option<&ModelKey>) -> Result<()> {
    if let Some(key) = model_key {
        anyhow::ensure!(
            model == Some(key.model_id.as_str()),
            "Hive worker model does not match model key"
        );
    }
    Ok(())
}

pub(crate) fn map_worker(row: &Row<'_>) -> rusqlite::Result<HiveWorker> {
    let permission_mode_raw: String = row.get(8)?;
    let permission_mode = permission_mode_raw
        .parse::<PermissionMode>()
        .map_err(|error| conversion_error(8, error))?;
    let autonomy = parse_required(9, row.get::<_, String>(9)?, HiveWorkerAutonomy::parse)?;
    let status = parse_required(11, row.get::<_, String>(11)?, HiveWorkerStatus::parse)?;
    let model_key = row
        .get::<_, Option<String>>(6)?
        .map(|value| {
            serde_json::from_str::<ModelKey>(&value)
                .map_err(|error| conversion_error(6, format!("invalid model key JSON: {error}")))
        })
        .transpose()?;
    let heartbeat_interval_secs = row
        .get::<_, Option<i64>>(10)?
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| conversion_error(10, "heartbeat interval out of range"))
        })
        .transpose()?;
    Ok(HiveWorker {
        id: row.get(0)?,
        user_id: row.get(1)?,
        slug: row.get(2)?,
        display_name: row.get(3)?,
        avatar_color: row.get(4)?,
        model: row.get(5)?,
        model_key,
        model_catalog_revision: row.get(7)?,
        permission_mode,
        autonomy,
        heartbeat_interval_secs,
        status,
        dm_session_id: row.get(12)?,
        memory_namespace_id: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn map_document(row: &Row<'_>) -> rusqlite::Result<HiveWorkerDocument> {
    let kind = parse_required(0, row.get::<_, String>(0)?, HiveWorkerDocumentKind::parse)?;
    Ok(HiveWorkerDocument {
        kind,
        content: row.get(1)?,
        updated_at: row.get(2)?,
    })
}

fn parse_required<T>(
    index: usize,
    value: String,
    parse: impl FnOnce(&str) -> Option<T>,
) -> rusqlite::Result<T> {
    parse(&value).ok_or_else(|| conversion_error(index, format!("invalid enum value: {value}")))
}

fn conversion_error(index: usize, message: impl std::fmt::Display) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(IoError::new(ErrorKind::InvalidData, message.to_string())),
    )
}
