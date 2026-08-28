use std::str::FromStr;

use anyhow::{ensure, Context, Result};
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use uuid::Uuid;

use super::disk::write_report_to_disk;
use super::model::{CreateReportInput, Report, ReportScope};
use crate::storage::database::Database;
use crate::storage::{
    load_group_worker_lane_with_conn, resolve_worker_conversation_with_conn, MemoryAclScope,
    MemoryNamespace,
};

pub struct ReportStore {
    pub(super) db: Database,
}

impl ReportStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn create_report(&self, input: CreateReportInput<'_>) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let tags_json = serde_json::to_string(input.tags).context("serializing tags")?;
        let sources_json = serde_json::to_string(input.sources).context("serializing sources")?;
        input.scope.validate().map_err(anyhow::Error::msg)?;

        // Resolve and freeze the source-session authority under the same
        // writer lock as the insert. A caller cannot label a Worker lane as
        // owner-shared (or an ordinary session as Worker-private), and a DM
        // rebind cannot race between validation and persistence.
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)
            .context("acquiring report creation lock")?;
        let (owner_user_id, session_type) = tx
            .query_row(
                "SELECT user_id, session_type FROM sessions WHERE id = ?1",
                [input.session_id],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or_else(|| anyhow::anyhow!("report source session does not exist"))?;
        let authoritative_scope = authoritative_report_scope(
            &tx,
            input.session_id,
            &session_type,
            owner_user_id.as_deref(),
        )?;
        ensure!(
            input.scope == authoritative_scope,
            "report scope does not match the persisted source session"
        );

        tx.execute(
            "INSERT INTO reports (
                 id, title, session_id, project_dir, content, summary, tags, sources,
                 owner_user_id, memory_namespace, namespace_id, acl_scope, source_worker_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                id,
                input.title,
                input.session_id,
                input.project_dir,
                input.content,
                input.summary,
                tags_json,
                sources_json,
                owner_user_id,
                authoritative_scope.memory_namespace().as_str(),
                authoritative_scope.namespace_id(),
                authoritative_scope.acl_scope().as_str(),
                authoritative_scope.source_worker_id(),
            ],
        )?;
        tx.commit()?;

        // The project-local Markdown mirror has no per-Worker ACL. Keep it
        // only for owner-shared reports; Worker-private content remains in
        // the scoped SQLite store instead of leaking through a shared file.
        if authoritative_scope.acl_scope() == MemoryAclScope::Owner {
            match self.get_report(&id) {
                Ok(Some(report)) => {
                    if let Err(error) = write_report_to_disk(&report, input.report_root) {
                        tracing::warn!(report_id = %id, "Failed to write report to disk: {error}");
                    }
                }
                Ok(None) => {
                    tracing::warn!(report_id = %id, "Created report could not be reloaded for disk write");
                }
                Err(error) => {
                    tracing::warn!(report_id = %id, "Failed to reload report for disk write: {error}");
                }
            }
        }

        Ok(id)
    }

    pub(crate) fn get_report(&self, id: &str) -> Result<Option<Report>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at,
                    owner_user_id, memory_namespace, namespace_id, acl_scope, source_worker_id
             FROM reports WHERE id = ?1",
        )?;

        stmt.query_row(params![id], row_to_report)
            .optional()
            .context("fetching report")
    }

    pub fn get_report_for_user(&self, id: &str, user_id: Option<&str>) -> Result<Option<Report>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                    reports.content, reports.summary, reports.tags, reports.sources,
                    reports.created_at, reports.owner_user_id, reports.memory_namespace,
                    reports.namespace_id, reports.acl_scope, reports.source_worker_id
             FROM reports
             WHERE reports.id = ?1
               AND reports.owner_user_id IS ?2
               AND reports.acl_scope = 'owner'",
        )?;

        stmt.query_row(params![id, user_id], row_to_report)
            .optional()
            .context("fetching report for user")
    }

    /// Load one report visible at a Hive/standard tool boundary.
    ///
    /// Reports created outside Worker lanes are shared with the exact owner.
    /// A Worker may additionally read reports from its own DM and group lanes.
    /// Passing no `worker_id` (ordinary Chat/Code or primary Hive) excludes
    /// every Worker-authored report, even when the caller knows its id.
    pub fn get_report_for_memory_reader(
        &self,
        id: &str,
        user_id: Option<&str>,
        worker_id: Option<&str>,
    ) -> Result<Option<Report>> {
        let mut statement = self.db.conn().prepare(
            "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                    reports.content, reports.summary, reports.tags, reports.sources,
                    reports.created_at, reports.owner_user_id, reports.memory_namespace,
                    reports.namespace_id, reports.acl_scope, reports.source_worker_id
             FROM reports
             WHERE reports.id = ?1
               AND reports.owner_user_id IS ?2
               AND (
                   reports.acl_scope = 'owner'
                   OR (
                       ?3 IS NOT NULL
                       AND reports.acl_scope = 'worker'
                       AND reports.source_worker_id = ?3
                   )
               )",
        )?;
        statement
            .query_row(params![id, user_id, worker_id], row_to_report)
            .optional()
            .context("fetching report for memory reader")
    }

    #[cfg(test)]
    pub(crate) fn list_reports(&self, project_dir: Option<&str>) -> Result<Vec<Report>> {
        let (sql, bound) = if let Some(pd) = project_dir {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at,
                        owner_user_id, memory_namespace, namespace_id, acl_scope, source_worker_id
                 FROM reports WHERE project_dir = ?1
                 ORDER BY created_at DESC".to_string(),
                vec![pd.to_string()],
            )
        } else {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at,
                        owner_user_id, memory_namespace, namespace_id, acl_scope, source_worker_id
                 FROM reports ORDER BY created_at DESC".to_string(),
                vec![],
            )
        };

        let mut stmt = self.db.conn().prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = bound
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), row_to_report)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("listing reports")
    }

    pub fn list_reports_for_user(
        &self,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<Report>> {
        self.list_reports_for_exact_owner(project_dir, user_id)
    }

    /// List owner-shared reports frozen for the exact owner. `None` is the
    /// local owner, not a wildcard across tenants.
    pub fn list_reports_for_exact_owner(
        &self,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<Report>> {
        let mut statement = self.db.conn().prepare(
            "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                    reports.content, reports.summary, reports.tags, reports.sources,
                    reports.created_at, reports.owner_user_id, reports.memory_namespace,
                    reports.namespace_id, reports.acl_scope, reports.source_worker_id
             FROM reports
             WHERE reports.owner_user_id IS ?1
               AND reports.acl_scope = 'owner'
               AND (?2 IS NULL OR reports.project_dir = ?2)
             ORDER BY reports.created_at DESC",
        )?;
        let rows = statement.query_map(params![user_id, project_dir], row_to_report)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("listing reports for exact owner")
    }

    /// List reports visible at a Hive/standard prompt boundary without
    /// allowing one Worker's source session to bleed into another reader.
    ///
    /// Reports created outside Worker lanes remain owner-shared. A Worker may
    /// additionally see reports from its own DM and group lanes. Passing no
    /// `worker_id` (ordinary Chat/Code or primary Hive) excludes every
    /// Worker-authored report.
    pub fn list_reports_for_memory_reader(
        &self,
        project_dir: Option<&str>,
        user_id: Option<&str>,
        worker_id: Option<&str>,
    ) -> Result<Vec<Report>> {
        let mut statement = self.db.conn().prepare(
            "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                    reports.content, reports.summary, reports.tags, reports.sources,
                    reports.created_at, reports.owner_user_id, reports.memory_namespace,
                    reports.namespace_id, reports.acl_scope, reports.source_worker_id
             FROM reports
             WHERE reports.owner_user_id IS ?1
               AND (?2 IS NULL OR reports.project_dir = ?2)
               AND (
                   reports.acl_scope = 'owner'
                   OR (
                       ?3 IS NOT NULL
                       AND reports.acl_scope = 'worker'
                       AND reports.source_worker_id = ?3
                   )
               )
             ORDER BY reports.created_at DESC",
        )?;
        let rows = statement.query_map(params![user_id, project_dir, worker_id], row_to_report)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("listing reports for memory reader")
    }

    #[cfg(test)]
    pub(crate) fn search_reports(
        &self,
        query: &str,
        project_dir: Option<&str>,
    ) -> Result<Vec<Report>> {
        let pattern = report_search_pattern(query);

        let (sql, bound) = if let Some(pd) = project_dir {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at,
                        owner_user_id, memory_namespace, namespace_id, acl_scope, source_worker_id
                 FROM reports
                 WHERE (title LIKE ?1 OR summary LIKE ?1 OR tags LIKE ?1 OR sources LIKE ?1)
                   AND project_dir = ?2
                 ORDER BY created_at DESC",
                vec![pattern, pd.to_string()],
            )
        } else {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at,
                        owner_user_id, memory_namespace, namespace_id, acl_scope, source_worker_id
                 FROM reports
                 WHERE title LIKE ?1 OR summary LIKE ?1 OR tags LIKE ?1 OR sources LIKE ?1
                 ORDER BY created_at DESC",
                vec![pattern],
            )
        };

        let mut stmt = self.db.conn().prepare(sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = bound
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), row_to_report)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("searching reports")
    }

    pub fn search_reports_for_user(
        &self,
        query: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<Report>> {
        let pattern = report_search_pattern(query);
        let mut statement = self.db.conn().prepare(
            "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                    reports.content, reports.summary, reports.tags, reports.sources,
                    reports.created_at, reports.owner_user_id, reports.memory_namespace,
                    reports.namespace_id, reports.acl_scope, reports.source_worker_id
             FROM reports
             WHERE reports.owner_user_id IS ?1
               AND reports.acl_scope = 'owner'
               AND (?2 IS NULL OR reports.project_dir = ?2)
               AND (
                   reports.title LIKE ?3
                   OR reports.summary LIKE ?3
                   OR reports.tags LIKE ?3
                   OR reports.sources LIKE ?3
               )
             ORDER BY reports.created_at DESC",
        )?;
        let rows = statement.query_map(params![user_id, project_dir, pattern], row_to_report)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("searching reports for user")
    }

    /// Search reports visible to the exact standard/Hive reader.
    ///
    /// This is the search counterpart to
    /// [`Self::list_reports_for_memory_reader`]: shared owner reports remain
    /// visible, while private Worker reports require the matching Worker id.
    pub fn search_reports_for_memory_reader(
        &self,
        query: &str,
        project_dir: Option<&str>,
        user_id: Option<&str>,
        worker_id: Option<&str>,
    ) -> Result<Vec<Report>> {
        let pattern = report_search_pattern(query);
        let mut statement = self.db.conn().prepare(
            "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                    reports.content, reports.summary, reports.tags, reports.sources,
                    reports.created_at, reports.owner_user_id, reports.memory_namespace,
                    reports.namespace_id, reports.acl_scope, reports.source_worker_id
             FROM reports
             WHERE reports.owner_user_id IS ?1
               AND (?2 IS NULL OR reports.project_dir = ?2)
               AND (
                   reports.title LIKE ?4
                   OR reports.summary LIKE ?4
                   OR reports.tags LIKE ?4
                   OR reports.sources LIKE ?4
               )
               AND (
                   reports.acl_scope = 'owner'
                   OR (
                       ?3 IS NOT NULL
                       AND reports.acl_scope = 'worker'
                       AND reports.source_worker_id = ?3
                   )
               )
             ORDER BY reports.created_at DESC",
        )?;
        let rows = statement.query_map(
            params![user_id, project_dir, worker_id, pattern],
            row_to_report,
        )?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("searching reports for memory reader")
    }

    #[cfg(test)]
    pub(crate) fn delete_report(&self, id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM reports WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn authoritative_report_scope(
    conn: &rusqlite::Connection,
    session_id: &str,
    session_type: &str,
    owner_user_id: Option<&str>,
) -> Result<ReportScope> {
    let Some(binding) = resolve_worker_conversation_with_conn(conn, session_id)? else {
        return Ok(ReportScope::owner_shared());
    };
    ensure!(
        session_type == "hive",
        "Worker report source must be a Hive session"
    );
    ensure!(
        binding.worker.user_id.as_deref() == owner_user_id,
        "Worker report source does not match the session owner"
    );
    if let Some(group_id) = binding.group_id.as_deref() {
        let lane = load_group_worker_lane_with_conn(conn, group_id, &binding.worker.id)?
            .ok_or_else(|| anyhow::anyhow!("Worker report source has no valid group lane"))?;
        ensure!(
            lane.session_id == session_id,
            "Worker report source does not match its group lane"
        );
    }
    ReportScope::worker_private(binding.worker.id, binding.worker.memory_namespace_id)
        .map_err(anyhow::Error::msg)
}

pub(super) fn row_to_report(row: &rusqlite::Row<'_>) -> rusqlite::Result<Report> {
    let tags_json: String = row.get(6)?;
    let sources_json: String = row.get(7)?;
    let namespace_raw: String = row.get(10)?;
    let acl_raw: String = row.get(12)?;
    let memory_namespace = MemoryNamespace::from_str(&namespace_raw)
        .map_err(|error| report_conversion_error(10, error))?;
    let acl_scope =
        MemoryAclScope::from_str(&acl_raw).map_err(|error| report_conversion_error(12, error))?;
    let scope = ReportScope::from_storage(memory_namespace, row.get(11)?, acl_scope, row.get(13)?)
        .map_err(|error| report_conversion_error(12, error))?;

    Ok(Report {
        id: row.get(0)?,
        title: row.get(1)?,
        session_id: row.get(2)?,
        project_dir: row.get(3)?,
        content: row.get(4)?,
        summary: row.get(5)?,
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        sources: serde_json::from_str(&sources_json).unwrap_or_default(),
        created_at: row.get(8)?,
        owner_user_id: row.get(9)?,
        scope,
    })
}

fn report_conversion_error(index: usize, error: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, error)),
    )
}

fn report_search_pattern(query: &str) -> String {
    format!("%{}%", query.trim())
}
