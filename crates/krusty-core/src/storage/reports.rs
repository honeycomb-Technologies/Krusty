//! Persistent research reports
//!
//! Stores reports produced by Chat (with research toggle) and Mako sessions.
//! Each report is persisted in SQLite and also written to disk as a Markdown
//! file under `.krusty/reports/` within the active workspace when one exists.

use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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

pub struct CreateReportInput<'a> {
    pub title: &'a str,
    pub session_id: &'a str,
    pub project_dir: Option<&'a str>,
    pub report_root: Option<&'a Path>,
    pub content: &'a str,
    pub summary: &'a str,
    pub tags: &'a [String],
    pub sources: &'a [String],
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

pub fn promote_report_content(report: &Report) -> String {
    let summary = report.summary.trim();
    if !summary.is_empty() {
        return summary.to_string();
    }

    let mut collapsed = report
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.len() > 600 {
        collapsed.truncate(600);
        collapsed.push_str("...");
    }
    collapsed
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
    let slug = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if slug.is_empty() {
        "report".to_string()
    } else {
        slug
    }
}

#[derive(Serialize)]
struct ReportFrontmatter<'a> {
    title: &'a str,
    created: &'a str,
    session_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_dir: Option<&'a str>,
    #[serde(skip_serializing_if = "slice_is_empty")]
    tags: &'a [String],
    #[serde(skip_serializing_if = "slice_is_empty")]
    sources: &'a [String],
}

fn slice_is_empty(values: &[String]) -> bool {
    values.is_empty()
}

fn write_report_to_disk(report: &Report, report_root: Option<&Path>) -> Result<PathBuf> {
    let reports_dir = report_root
        .map(paths::project_reports_dir)
        .unwrap_or_else(|| paths::config_dir().join("reports"));
    std::fs::create_dir_all(&reports_dir).context("creating reports directory")?;

    let path = next_report_path(report, &reports_dir);
    let markdown = render_report_markdown(report)?;

    std::fs::write(&path, markdown).context("writing report file")?;
    Ok(path)
}

fn report_date_prefix(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| chrono::Utc::now().format("%Y-%m-%d").to_string())
}

fn next_report_path(report: &Report, reports_dir: &Path) -> PathBuf {
    let date = report_date_prefix(&report.created_at);
    let slug = slugify(&report.title);
    let base = reports_dir.join(format!("{date}-{slug}.md"));
    if !base.exists() {
        return base;
    }

    let short_id: String = report.id.chars().filter(|c| *c != '-').take(8).collect();
    let fallback = reports_dir.join(format!("{date}-{slug}-{short_id}.md"));
    if !fallback.exists() {
        return fallback;
    }

    for index in 2.. {
        let candidate = reports_dir.join(format!("{date}-{slug}-{short_id}-{index}.md"));
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("report path search should always find an available file name");
}

fn render_report_markdown(report: &Report) -> Result<String> {
    let frontmatter = ReportFrontmatter {
        title: &report.title,
        created: &report.created_at,
        session_id: &report.session_id,
        project_dir: report.project_dir.as_deref(),
        tags: &report.tags,
        sources: &report.sources,
    };
    let yaml = serde_yaml::to_string(&frontmatter).context("serializing report frontmatter")?;
    let yaml = yaml.strip_prefix("---\n").unwrap_or(yaml.as_str());
    let body = report.content.trim_start_matches('\n');
    Ok(format!("---\n{}---\n\n{}", yaml, body))
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

    fn create_store_with_users() -> (ReportStore, TempDir) {
        let (store, tmp) = create_store();
        let now = chrono::Utc::now().to_rfc3339();
        store
            .db
            .conn()
            .execute(
                "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
                params!["user-a", "user-a@example.com", "free"],
            )
            .expect("seed user a");
        store
            .db
            .conn()
            .execute(
                "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
                params!["user-b", "user-b@example.com", "free"],
            )
            .expect("seed user b");
        store
            .db
            .conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at, user_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["sess-a", "User A Session", now, now, "user-a"],
            )
            .expect("seed owned session a");
        store
            .db
            .conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at, user_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                params!["sess-b", "User B Session", now, now, "user-b"],
            )
            .expect("seed owned session b");
        (store, tmp)
    }

    #[test]
    fn create_and_get_report() {
        let (store, _tmp) = create_store();
        let id = store
            .create_report(CreateReportInput {
                title: "Auth Analysis",
                session_id: "sess-1",
                project_dir: Some("/home/user/project"),
                report_root: None,
                content: "# Auth\nDetailed analysis...",
                summary: "OAuth2 flow review",
                tags: &["auth".into(), "security".into()],
                sources: &["RFC 6749".into()],
            })
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
            .create_report(CreateReportInput {
                title: "Report A",
                session_id: "sess-1",
                project_dir: Some("/proj-a"),
                report_root: None,
                content: "content a",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();
        store
            .create_report(CreateReportInput {
                title: "Report B",
                session_id: "sess-1",
                project_dir: Some("/proj-b"),
                report_root: None,
                content: "content b",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();
        store
            .create_report(CreateReportInput {
                title: "Report C",
                session_id: "sess-1",
                project_dir: Some("/proj-a"),
                report_root: None,
                content: "content c",
                summary: "",
                tags: &[],
                sources: &[],
            })
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
            .create_report(CreateReportInput {
                title: "Database Migration Guide",
                session_id: "sess-1",
                project_dir: None,
                report_root: None,
                content: "content",
                summary: "",
                tags: &["database".into(), "migration".into()],
                sources: &[],
            })
            .unwrap();
        store
            .create_report(CreateReportInput {
                title: "API Design",
                session_id: "sess-1",
                project_dir: None,
                report_root: None,
                content: "content",
                summary: "",
                tags: &["api".into()],
                sources: &[],
            })
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
            .create_report(CreateReportInput {
                title: "Temporary",
                session_id: "sess-1",
                project_dir: None,
                report_root: None,
                content: "content",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();
        assert!(store.get_report(&id).unwrap().is_some());

        store.delete_report(&id).unwrap();
        assert!(store.get_report(&id).unwrap().is_none());
    }

    #[test]
    fn writes_reports_to_project_local_directory() {
        let (store, tmp) = create_store();
        let project_root = tmp.path().join("workspace");
        std::fs::create_dir_all(&project_root).unwrap();

        store
            .create_report(CreateReportInput {
                title: "Workspace Report",
                session_id: "sess-1",
                project_dir: Some(project_root.to_str().unwrap()),
                report_root: Some(&project_root),
                content: "content",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();

        let reports_dir = crate::paths::project_reports_dir(&project_root);
        let entries = std::fs::read_dir(reports_dir)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        let content = std::fs::read_to_string(entries[0].path()).unwrap();
        assert!(content.contains("title: Workspace Report"));
        assert!(content.contains("session_id: sess-1"));
        assert!(content.contains("project_dir:"));
        assert!(content.contains("\n---\n\ncontent"));
    }

    #[test]
    fn duplicate_titles_do_not_overwrite_report_files() {
        let (store, tmp) = create_store();
        let project_root = tmp.path().join("workspace");
        std::fs::create_dir_all(&project_root).unwrap();

        store
            .create_report(CreateReportInput {
                title: "Repeated Title",
                session_id: "sess-1",
                project_dir: Some(project_root.to_str().unwrap()),
                report_root: Some(&project_root),
                content: "first",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();
        store
            .create_report(CreateReportInput {
                title: "Repeated Title",
                session_id: "sess-1",
                project_dir: Some(project_root.to_str().unwrap()),
                report_root: Some(&project_root),
                content: "second",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();

        let reports_dir = crate::paths::project_reports_dir(&project_root);
        let mut names = std::fs::read_dir(reports_dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();

        assert_eq!(names.len(), 2);
        assert_ne!(names[0], names[1]);
    }

    #[test]
    fn slugify_works() {
        assert_eq!(slugify("Auth Analysis"), "auth-analysis");
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  spaces  "), "spaces");
        assert_eq!(slugify("a--b"), "a-b");
        assert_eq!(slugify("!!!"), "report");
    }

    #[test]
    fn list_reports_for_user_filters_via_session_owner() {
        let (store, _tmp) = create_store_with_users();
        store
            .create_report(CreateReportInput {
                title: "A Report",
                session_id: "sess-a",
                project_dir: Some("/proj-a"),
                report_root: None,
                content: "content a",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();
        store
            .create_report(CreateReportInput {
                title: "B Report",
                session_id: "sess-b",
                project_dir: Some("/proj-b"),
                report_root: None,
                content: "content b",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();

        let user_a_reports = store.list_reports_for_user(None, Some("user-a")).unwrap();
        assert_eq!(user_a_reports.len(), 1);
        assert_eq!(user_a_reports[0].title, "A Report");

        let project_scoped = store
            .list_reports_for_user(Some("/proj-a"), Some("user-a"))
            .unwrap();
        assert_eq!(project_scoped.len(), 1);
        assert_eq!(project_scoped[0].title, "A Report");
    }

    #[test]
    fn get_report_for_user_hides_foreign_reports() {
        let (store, _tmp) = create_store_with_users();
        let report_id = store
            .create_report(CreateReportInput {
                title: "A Report",
                session_id: "sess-a",
                project_dir: Some("/proj-a"),
                report_root: None,
                content: "content a",
                summary: "",
                tags: &[],
                sources: &[],
            })
            .unwrap();

        let owned = store
            .get_report_for_user(&report_id, Some("user-a"))
            .unwrap()
            .expect("owned report should load");
        assert_eq!(owned.title, "A Report");

        let hidden = store
            .get_report_for_user(&report_id, Some("user-b"))
            .unwrap();
        assert!(hidden.is_none());
    }
}
