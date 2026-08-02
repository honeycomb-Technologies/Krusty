//! Persistence for compaction checkpoints and segments.

use anyhow::Result;
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::storage::database::Database;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSegmentRecord {
    pub id: String,
    pub session_id: String,
    pub checkpoint_id: String,
    pub message_id_start: i64,
    pub message_id_end: i64,
    pub segment_markdown: String,
    pub token_estimate: usize,
    pub created_at: String,
}

pub struct CompactionStore<'a> {
    db: &'a Database,
}

impl<'a> CompactionStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    pub fn count_checkpoints(&self, session_id: &str) -> Result<u32> {
        let count: i64 = self.db.conn().query_row(
            "SELECT COUNT(*) FROM compaction_checkpoints WHERE session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(count.max(0) as u32)
    }

    pub fn search_segments(
        &self,
        session_id: &str,
        query: Option<&str>,
        limit: usize,
    ) -> Result<Vec<CompactionSegmentRecord>> {
        let has_query = query.is_some_and(|value| !value.trim().is_empty());
        let sql = if has_query {
            "SELECT id, session_id, checkpoint_id, message_id_start, message_id_end,
                    segment_markdown, token_estimate, created_at
             FROM compaction_segments
             WHERE session_id = ?1 AND segment_markdown LIKE ?2
             ORDER BY created_at DESC LIMIT ?3"
        } else {
            "SELECT id, session_id, checkpoint_id, message_id_start, message_id_end,
                    segment_markdown, token_estimate, created_at
             FROM compaction_segments
             WHERE session_id = ?1
             ORDER BY created_at DESC LIMIT ?2"
        };

        let mut stmt = self.db.conn().prepare(sql)?;
        let rows = if let Some(query) = query.filter(|value| !value.trim().is_empty()) {
            let pattern = format!("%{query}%");
            stmt.query_map(params![session_id, pattern, limit as i64], row_to_segment)?
        } else {
            stmt.query_map(params![session_id, limit as i64], row_to_segment)?
        };

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn row_to_segment(row: &rusqlite::Row<'_>) -> rusqlite::Result<CompactionSegmentRecord> {
    Ok(CompactionSegmentRecord {
        id: row.get(0)?,
        session_id: row.get(1)?,
        checkpoint_id: row.get(2)?,
        message_id_start: row.get(3)?,
        message_id_end: row.get(4)?,
        segment_markdown: row.get(5)?,
        token_estimate: row.get::<_, i64>(6)? as usize,
        created_at: row.get(7)?,
    })
}
