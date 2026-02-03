//! File activity tracking for pinch
//!
//! Tracks read/write/edit operations on files during a session
//! to determine which files are most important for context preservation.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::params;

use super::database::Database;

/// File access activity for importance scoring
#[derive(Debug, Clone)]
pub struct FileActivity {
    pub file_path: String,
    pub read_count: usize,
    pub write_count: usize,
    pub edit_count: usize,
    pub last_accessed: DateTime<Utc>,
    pub user_referenced: bool,
}

impl FileActivity {
    /// Calculate importance score (higher = more important)
    ///
    /// Weights:
    /// - Writes: 3 points each (created/overwritten files are critical)
    /// - Edits: 2 points each (modified files are important)
    /// - Reads: 1 point each (viewed files provide context)
    /// - User reference: 5 point bonus (explicitly mentioned by user)
    /// - Recency: multiplier based on how recently accessed
    pub fn importance_score(&self, now: DateTime<Utc>) -> f64 {
        let activity_score = (self.write_count * 3 + self.edit_count * 2 + self.read_count) as f64;
        let user_bonus = if self.user_referenced { 5.0 } else { 0.0 };

        // Recency bonus: files accessed recently get higher scores
        // Decays over 24 hours
        let hours_ago = (now - self.last_accessed).num_hours().max(0) as f64;
        let recency_multiplier = 1.0 / (1.0 + hours_ago / 24.0);

        (activity_score + user_bonus) * (0.5 + 0.5 * recency_multiplier)
    }
}

/// A file ranked by importance with reasons
#[derive(Debug, Clone)]
pub struct RankedFile {
    pub path: String,
    pub score: f64,
    pub reasons: Vec<String>,
}

impl RankedFile {
    /// Create from file activity
    pub fn from_activity(activity: &FileActivity, now: DateTime<Utc>) -> Self {
        let mut reasons = Vec::new();

        if activity.write_count > 0 {
            reasons.push(format!("written {} time(s)", activity.write_count));
        }
        if activity.edit_count > 0 {
            reasons.push(format!("edited {} time(s)", activity.edit_count));
        }
        if activity.read_count > 0 {
            reasons.push(format!("read {} time(s)", activity.read_count));
        }
        if activity.user_referenced {
            reasons.push("referenced by user".to_string());
        }

        Self {
            path: activity.file_path.clone(),
            score: activity.importance_score(now),
            reasons,
        }
    }
}

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
                last_accessed: DateTime::parse_from_rfc3339(&last_accessed)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
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
    ///
    /// This is more efficient than get_all_activities() for large datasets because:
    /// - Ranking is done in SQL (no in-memory sorting)
    /// - Uses LIMIT to avoid loading all rows
    /// - Only loads the top N files
    pub fn get_ranked_files_sql(&self, limit: usize) -> Result<Vec<RankedFile>> {
        // Calculate importance score in SQL
        // Formula: (writes*3 + edits*2 + reads + user_bonus*5) * (0.5 + 0.5 * recency_mult)
        // where recency_mult = 1.0 / (1.0 + hours_ago / 24.0)
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
            let score: f64 = row.get(6)?;

            let mut reasons = Vec::new();
            if write_count > 0 {
                reasons.push(format!("written {} time(s)", write_count));
            }
            if edit_count > 0 {
                reasons.push(format!("edited {} time(s)", edit_count));
            }
            if read_count > 0 {
                reasons.push(format!("read {} time(s)", read_count));
            }
            if row.get::<_, i64>(5)? != 0 {
                reasons.push("referenced by user".to_string());
            }

            Ok(RankedFile {
                path: file_path,
                score,
                reasons,
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
