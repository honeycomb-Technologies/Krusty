use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::params;

use super::model::{build_activity_reasons, FileActivity, RankedFile};
use crate::storage::database::Database;

/// File activity tracker for a session
pub struct FileActivityTracker<'a> {
    db: &'a Database,
    session_id: String,
}

impl<'a> FileActivityTracker<'a> {
    /// Create a new tracker for a session
    pub fn new(db: &'a Database, session_id: String) -> Self {
        Self { db, session_id }
    }

    /// Get all file activities for the session
    pub fn get_all_activities(&self) -> Result<Vec<FileActivity>> {
        let mut stmt = self.db.conn().prepare(
            "SELECT file_path, read_count, write_count, edit_count, last_accessed, user_referenced
             FROM file_activity WHERE session_id = ?1",
        )?;

        let activities = stmt.query_map([&self.session_id], |row| {
            let last_accessed: String = row.get(4)?;
            Ok(FileActivity {
                file_path: row.get(0)?,
                read_count: row.get::<_, i64>(1)? as usize,
                write_count: row.get::<_, i64>(2)? as usize,
                edit_count: row.get::<_, i64>(3)? as usize,
                last_accessed: parse_last_accessed(&last_accessed),
                user_referenced: row.get::<_, i64>(5)? != 0,
            })
        })?;

        activities
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Get ranked files sorted by importance (SQL-level ranking with paging)
    pub fn get_ranked_files(&self, limit: usize) -> Result<Vec<RankedFile>> {
        self.get_ranked_files_sql(limit)
    }

    /// Get ranked files using SQL-level ranking and sorting
    pub fn get_ranked_files_sql(&self, limit: usize) -> Result<Vec<RankedFile>> {
        let sql = r#"
            SELECT
                file_path,
                read_count,
                write_count,
                edit_count,
                last_accessed,
                user_referenced,
                (
                    (write_count * 3 + edit_count * 2 + read_count + CASE WHEN user_referenced = 1 THEN 5 ELSE 0 END)
                    *
                    (0.5 + 0.5 / (1.0 + CAST(strftime('%s', 'now') - strftime('%s', last_accessed) AS REAL) / 86400.0))
                ) as importance_score
            FROM file_activity
            WHERE session_id = ?1
            ORDER BY importance_score DESC
            LIMIT ?2
        "#;

        let mut stmt = self.db.conn().prepare(sql)?;

        let files = stmt.query_map(params![&self.session_id, limit as i64], |row| {
            let file_path: String = row.get(0)?;
            let read_count: i64 = row.get(1)?;
            let write_count: i64 = row.get(2)?;
            let edit_count: i64 = row.get(3)?;
            let user_referenced = row.get::<_, i64>(5)? != 0;
            let score: f64 = row.get(6)?;

            Ok(RankedFile {
                path: file_path,
                score,
                reasons: build_activity_reasons(
                    read_count,
                    write_count,
                    edit_count,
                    user_referenced,
                ),
            })
        })?;

        files.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Get top N files as (path, score) pairs for preview
    pub fn get_top_files_preview(&self, n: usize) -> Vec<(String, f64)> {
        self.get_ranked_files(n)
            .unwrap_or_default()
            .into_iter()
            .map(|f| (f.path, f.score))
            .collect()
    }
}

fn parse_last_accessed(last_accessed: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(last_accessed)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now())
}
