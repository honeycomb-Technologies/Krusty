use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use super::disk::write_report_to_disk;
use super::model::{CreateReportInput, Report};
use crate::storage::database::Database;

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

        self.db.conn().execute(
            "INSERT INTO reports (id, title, session_id, project_dir, content, summary, tags, sources)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                input.title,
                input.session_id,
                input.project_dir,
                input.content,
                input.summary,
                tags_json,
                sources_json
            ],
        )?;

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

        Ok(id)
    }

    pub fn get_report(&self, id: &str) -> Result<Option<Report>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
             FROM reports WHERE id = ?1",
        )?;

        stmt.query_row(params![id], row_to_report)
            .optional()
            .context("fetching report")
    }

    pub fn get_report_for_user(&self, id: &str, user_id: Option<&str>) -> Result<Option<Report>> {
        if let Some(user_id) = user_id {
            let mut stmt = self.db.conn().prepare(
                "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                        reports.content, reports.summary, reports.tags, reports.sources,
                        reports.created_at
                 FROM reports
                 INNER JOIN sessions ON sessions.id = reports.session_id
                 WHERE reports.id = ?1 AND sessions.user_id = ?2",
            )?;

            stmt.query_row(params![id, user_id], row_to_report)
                .optional()
                .context("fetching report for user")
        } else {
            self.get_report(id)
        }
    }

    pub fn list_reports(&self, project_dir: Option<&str>) -> Result<Vec<Report>> {
        let (sql, bound) = if let Some(pd) = project_dir {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
                 FROM reports WHERE project_dir = ?1
                 ORDER BY created_at DESC".to_string(),
                vec![pd.to_string()],
            )
        } else {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
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
        let (sql, bound) = match (project_dir, user_id) {
            (Some(project_dir), Some(user_id)) => (
                "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                        reports.content, reports.summary, reports.tags, reports.sources,
                        reports.created_at
                 FROM reports
                 INNER JOIN sessions ON sessions.id = reports.session_id
                 WHERE reports.project_dir = ?1 AND sessions.user_id = ?2
                 ORDER BY reports.created_at DESC"
                    .to_string(),
                vec![project_dir.to_string(), user_id.to_string()],
            ),
            (Some(project_dir), None) => (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
                 FROM reports WHERE project_dir = ?1
                 ORDER BY created_at DESC"
                    .to_string(),
                vec![project_dir.to_string()],
            ),
            (None, Some(user_id)) => (
                "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                        reports.content, reports.summary, reports.tags, reports.sources,
                        reports.created_at
                 FROM reports
                 INNER JOIN sessions ON sessions.id = reports.session_id
                 WHERE sessions.user_id = ?1
                 ORDER BY reports.created_at DESC"
                    .to_string(),
                vec![user_id.to_string()],
            ),
            (None, None) => (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
                 FROM reports ORDER BY created_at DESC"
                    .to_string(),
                vec![],
            ),
        };

        let mut stmt = self.db.conn().prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = bound
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), row_to_report)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("listing reports for user")
    }

    /// List reports whose source session belongs to the exact owner.
    ///
    /// This deliberately differs from the legacy `None` semantics of
    /// [`Self::list_reports_for_user`]: local Hive (`None`) sees only reports
    /// from local sessions, never every tenant's reports.
    pub fn list_reports_for_exact_owner(
        &self,
        project_dir: Option<&str>,
        user_id: Option<&str>,
    ) -> Result<Vec<Report>> {
        let mut sql = "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                    reports.content, reports.summary, reports.tags, reports.sources,
                    reports.created_at
             FROM reports
             INNER JOIN sessions ON sessions.id = reports.session_id
             WHERE sessions.user_id IS ?1"
            .to_string();
        let mut bound = vec![user_id.map(ToOwned::to_owned)];
        if let Some(project_dir) = project_dir {
            bound.push(Some(project_dir.to_string()));
            sql.push_str(" AND reports.project_dir = ?2");
        }
        sql.push_str(" ORDER BY reports.created_at DESC");

        let mut stmt = self.db.conn().prepare(&sql)?;
        let params = bound
            .iter()
            .map(|value| value as &dyn rusqlite::types::ToSql)
            .collect::<Vec<_>>();
        let rows = stmt.query_map(params.as_slice(), row_to_report)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("listing reports for exact owner")
    }

    pub fn search_reports(&self, query: &str, project_dir: Option<&str>) -> Result<Vec<Report>> {
        let pattern = report_search_pattern(query);

        let (sql, bound) = if let Some(pd) = project_dir {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
                 FROM reports
                 WHERE (title LIKE ?1 OR summary LIKE ?1 OR tags LIKE ?1 OR sources LIKE ?1)
                   AND project_dir = ?2
                 ORDER BY created_at DESC",
                vec![pattern, pd.to_string()],
            )
        } else {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
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

        let (sql, bound) = match (project_dir, user_id) {
            (Some(project_dir), Some(user_id)) => (
                "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                        reports.content, reports.summary, reports.tags, reports.sources,
                        reports.created_at
                 FROM reports
                 INNER JOIN sessions ON sessions.id = reports.session_id
                 WHERE (reports.title LIKE ?1 OR reports.summary LIKE ?1 OR reports.tags LIKE ?1 OR reports.sources LIKE ?1)
                   AND reports.project_dir = ?2
                   AND sessions.user_id = ?3
                 ORDER BY reports.created_at DESC"
                    .to_string(),
                vec![pattern, project_dir.to_string(), user_id.to_string()],
            ),
            (Some(project_dir), None) => (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
                 FROM reports
                 WHERE (title LIKE ?1 OR summary LIKE ?1 OR tags LIKE ?1 OR sources LIKE ?1)
                   AND project_dir = ?2
                 ORDER BY created_at DESC"
                    .to_string(),
                vec![pattern, project_dir.to_string()],
            ),
            (None, Some(user_id)) => (
                "SELECT reports.id, reports.title, reports.session_id, reports.project_dir,
                        reports.content, reports.summary, reports.tags, reports.sources,
                        reports.created_at
                 FROM reports
                 INNER JOIN sessions ON sessions.id = reports.session_id
                 WHERE (reports.title LIKE ?1 OR reports.summary LIKE ?1 OR reports.tags LIKE ?1 OR reports.sources LIKE ?1)
                   AND sessions.user_id = ?2
                 ORDER BY reports.created_at DESC"
                    .to_string(),
                vec![pattern, user_id.to_string()],
            ),
            (None, None) => (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
                 FROM reports
                 WHERE (title LIKE ?1 OR summary LIKE ?1 OR tags LIKE ?1 OR sources LIKE ?1)
                 ORDER BY created_at DESC"
                    .to_string(),
                vec![pattern],
            ),
        };

        let mut stmt = self.db.conn().prepare(&sql)?;
        let params: Vec<&dyn rusqlite::types::ToSql> = bound
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let rows = stmt.query_map(params.as_slice(), row_to_report)?;
        rows.collect::<Result<Vec<_>, _>>()
            .context("searching reports for user")
    }

    pub fn delete_report(&self, id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM reports WHERE id = ?1", params![id])?;
        Ok(())
    }
}

pub(super) fn row_to_report(row: &rusqlite::Row<'_>) -> rusqlite::Result<Report> {
    let tags_json: String = row.get(6)?;
    let sources_json: String = row.get(7)?;

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
    })
}

fn report_search_pattern(query: &str) -> String {
    format!("%{}%", query.trim())
}
