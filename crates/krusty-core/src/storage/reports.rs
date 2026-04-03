//! Persistent research reports
//!
//! Stores reports produced by Chat (with research toggle) and Mako sessions.
//! Each report is persisted in SQLite and also written to disk as a Markdown
//! file under `~/.krusty/reports/`.

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::database::Database;
use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub id: String,
    pub title: String,
    pub session_id: String,
    pub project_dir: Option<String>,
    pub content: String,
    pub summary: String,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub created_at: String,
}

pub struct ReportStore {
    db: Database,
}

impl ReportStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_report(
        &self,
        title: &str,
        session_id: &str,
        project_dir: Option<&str>,
        content: &str,
        summary: &str,
        tags: &[String],
        sources: &[String],
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let tags_json = serde_json::to_string(tags).context("serializing tags")?;
        let sources_json = serde_json::to_string(sources).context("serializing sources")?;

        self.db.conn().execute(
            "INSERT INTO reports (id, title, session_id, project_dir, content, summary, tags, sources)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![id, title, session_id, project_dir, content, summary, tags_json, sources_json],
        )?;

        if let Err(e) = write_report_to_disk(title, content) {
            tracing::warn!("Failed to write report to disk: {e}");
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

    pub fn search_reports(&self, query: &str, project_dir: Option<&str>) -> Result<Vec<Report>> {
        let pattern = format!("%{query}%");

        let (sql, bound) = if let Some(pd) = project_dir {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
                 FROM reports
                 WHERE (title LIKE ?1 OR tags LIKE ?1)
                   AND project_dir = ?2
                 ORDER BY created_at DESC",
                vec![pattern, pd.to_string()],
            )
        } else {
            (
                "SELECT id, title, session_id, project_dir, content, summary, tags, sources, created_at
                 FROM reports
                 WHERE title LIKE ?1 OR tags LIKE ?1
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

    pub fn delete_report(&self, id: &str) -> Result<()> {
        self.db
            .conn()
            .execute("DELETE FROM reports WHERE id = ?1", params![id])?;
        Ok(())
    }
}

fn row_to_report(row: &rusqlite::Row<'_>) -> rusqlite::Result<Report> {
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

fn slugify(title: &str) -> String {
    title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn write_report_to_disk(title: &str, content: &str) -> Result<()> {
    let reports_dir = paths::config_dir().join("reports");
    std::fs::create_dir_all(&reports_dir).context("creating reports directory")?;

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let slug = slugify(title);
    let filename = format!("{date}-{slug}.md");
    let path = reports_dir.join(filename);

    std::fs::write(&path, content).context("writing report file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use rusqlite::params;
    use tempfile::TempDir;

    use super::*;
    use crate::storage::Database;

    fn create_store() -> (ReportStore, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let db = Database::new(&tmp.path().join("reports.db")).expect("db");
        let now = chrono::Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params!["sess-1", "Report Test", now, now],
            )
            .expect("seed session");
        (ReportStore::new(db), tmp)
    }

    #[test]
    fn create_and_get_report() {
        let (store, _tmp) = create_store();
        let id = store
            .create_report(
                "Auth Analysis",
                "sess-1",
                Some("/home/user/project"),
                "# Auth\nDetailed analysis...",
                "OAuth2 flow review",
                &["auth".into(), "security".into()],
                &["RFC 6749".into()],
            )
            .unwrap();

        let report = store.get_report(&id).unwrap().unwrap();
        assert_eq!(report.title, "Auth Analysis");
        assert_eq!(report.summary, "OAuth2 flow review");
        assert_eq!(report.tags, vec!["auth", "security"]);
        assert_eq!(report.sources, vec!["RFC 6749"]);
        assert_eq!(report.project_dir.as_deref(), Some("/home/user/project"));
    }

    #[test]
    fn list_reports_by_project() {
        let (store, _tmp) = create_store();
        store
            .create_report(
                "Report A",
                "sess-1",
                Some("/proj-a"),
                "content a",
                "",
                &[],
                &[],
            )
            .unwrap();
        store
            .create_report(
                "Report B",
                "sess-1",
                Some("/proj-b"),
                "content b",
                "",
                &[],
                &[],
            )
            .unwrap();
        store
            .create_report(
                "Report C",
                "sess-1",
                Some("/proj-a"),
                "content c",
                "",
                &[],
                &[],
            )
            .unwrap();

        let proj_a = store.list_reports(Some("/proj-a")).unwrap();
        assert_eq!(proj_a.len(), 2);

        let all = store.list_reports(None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn search_reports_by_title_and_tags() {
        let (store, _tmp) = create_store();
        store
            .create_report(
                "Database Migration Guide",
                "sess-1",
                None,
                "content",
                "",
                &["database".into(), "migration".into()],
                &[],
            )
            .unwrap();
        store
            .create_report(
                "API Design",
                "sess-1",
                None,
                "content",
                "",
                &["api".into()],
                &[],
            )
            .unwrap();

        let results = store.search_reports("migration", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Database Migration Guide");

        let results = store.search_reports("api", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "API Design");
    }

    #[test]
    fn delete_report() {
        let (store, _tmp) = create_store();
        let id = store
            .create_report("Temporary", "sess-1", None, "content", "", &[], &[])
            .unwrap();
        assert!(store.get_report(&id).unwrap().is_some());

        store.delete_report(&id).unwrap();
        assert!(store.get_report(&id).unwrap().is_none());
    }

    #[test]
    fn slugify_works() {
        assert_eq!(slugify("Auth Analysis"), "auth-analysis");
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("a--b"), "a-b");
    }
}
