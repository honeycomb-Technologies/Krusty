use anyhow::{bail, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use uuid::Uuid;

use crate::storage::database::Database;

use super::{LearningCandidate, LearningCandidateInput, LearningCandidateStatus};

pub struct LearningCandidateStore<'a> {
    db: &'a Database,
}

impl<'a> LearningCandidateStore<'a> {
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Insert once per evidence message and canonical key. Reviewer retries
    /// return the existing record instead of multiplying proposals.
    pub fn insert(&self, input: &LearningCandidateInput) -> Result<LearningCandidate> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "INSERT INTO mako_learning_candidates (
                id, user_id, project_dir, canonical_key, kind, proposed_content,
                evidence_session_id, evidence_message_id, evidence_excerpt,
                explicit, confidence, sensitivity, status, reason, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(evidence_session_id, evidence_message_id, canonical_key) DO NOTHING",
            params![
                id,
                input.user_id,
                input.project_dir,
                input.canonical_key,
                input.kind.to_string(),
                input.proposed_content,
                input.evidence_session_id,
                input.evidence_message_id,
                input.evidence_excerpt,
                i64::from(input.explicit),
                input.confidence.clamp(0.0, 1.0),
                input.sensitivity.to_string(),
                input.status.to_string(),
                input.reason,
                now,
            ],
        )?;

        self.find_by_evidence(
            &input.evidence_session_id,
            input.evidence_message_id,
            &input.canonical_key,
        )?
        .ok_or_else(|| anyhow::anyhow!("learning candidate insert was not observable"))
    }

    pub fn list(
        &self,
        user_id: Option<&str>,
        status: Option<LearningCandidateStatus>,
        limit: usize,
    ) -> Result<Vec<LearningCandidate>> {
        let status = status.map(|value| value.to_string());
        let mut statement = self.db.conn().prepare(
            "SELECT id, user_id, project_dir, canonical_key, kind, proposed_content,
                    evidence_session_id, evidence_message_id, evidence_excerpt,
                    explicit, confidence, sensitivity, status, reason, created_at, reviewed_at
             FROM mako_learning_candidates
             WHERE ((?1 IS NULL AND user_id IS NULL) OR user_id = ?1)
               AND (?2 IS NULL OR status = ?2)
             ORDER BY created_at DESC
             LIMIT ?3",
        )?;
        let rows = statement.query_map(
            params![user_id, status, limit.clamp(1, 200) as i64],
            map_candidate,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn transition_owned(
        &self,
        id: &str,
        user_id: Option<&str>,
        target: LearningCandidateStatus,
    ) -> Result<Option<LearningCandidate>> {
        if !matches!(
            target,
            LearningCandidateStatus::Accepted
                | LearningCandidateStatus::Rejected
                | LearningCandidateStatus::Tombstoned
        ) {
            bail!("manual review cannot transition a candidate to {target}");
        }
        let now = Utc::now().to_rfc3339();
        self.db.conn().execute(
            "UPDATE mako_learning_candidates
             SET status = ?1, reviewed_at = ?2
             WHERE id = ?3
               AND ((?4 IS NULL AND user_id IS NULL) OR user_id = ?4)
               AND status = 'pending'",
            params![target.to_string(), now, id, user_id],
        )?;
        self.get_owned(id, user_id)
    }

    pub fn get_owned(&self, id: &str, user_id: Option<&str>) -> Result<Option<LearningCandidate>> {
        load_candidate_owned_from_connection(self.db.conn(), id, user_id)
    }

    pub fn begin_review(
        &self,
        session_id: &str,
        through_message_id: i64,
        model: Option<&str>,
    ) -> Result<bool> {
        let changed = self.db.conn().execute(
            "INSERT INTO mako_learning_runs (
                session_id, through_message_id, status, model, created_at
             ) VALUES (?1, ?2, 'running', ?3, ?4)
             ON CONFLICT(session_id, through_message_id) DO UPDATE SET
                status = 'running',
                model = excluded.model,
                created_at = excluded.created_at,
                completed_at = NULL
             WHERE mako_learning_runs.status = 'failed'
                OR (mako_learning_runs.status = 'running'
                    AND julianday(mako_learning_runs.created_at)
                        <= julianday('now', '-15 minutes'))",
            params![
                session_id,
                through_message_id,
                model,
                Utc::now().to_rfc3339()
            ],
        )?;
        Ok(changed == 1)
    }

    /// Return whether this user message is already covered by a running or
    /// completed review. Autonomous tick turns can produce additional
    /// assistant messages without new canonical user evidence; those ticks
    /// must not fan out duplicate background reviewers.
    pub fn has_nonfailed_review_covering(
        &self,
        session_id: &str,
        user_message_id: i64,
    ) -> Result<bool> {
        self.db
            .conn()
            .query_row(
                "SELECT EXISTS(
                    SELECT 1
                    FROM mako_learning_runs
                    WHERE session_id = ?1
                      AND through_message_id > ?2
                      AND (
                        status = 'completed'
                        OR (status = 'running'
                            AND julianday(created_at) > julianday('now', '-15 minutes'))
                      )
                 )",
                params![session_id, user_message_id],
                |row| row.get(0),
            )
            .map_err(Into::into)
    }

    pub fn finish_review(
        &self,
        session_id: &str,
        through_message_id: i64,
        succeeded: bool,
    ) -> Result<()> {
        self.db.conn().execute(
            "UPDATE mako_learning_runs
             SET status = ?1, completed_at = ?2
             WHERE session_id = ?3
               AND through_message_id = ?4
               AND status = 'running'",
            params![
                if succeeded { "completed" } else { "failed" },
                Utc::now().to_rfc3339(),
                session_id,
                through_message_id
            ],
        )?;
        Ok(())
    }

    fn find_by_evidence(
        &self,
        session_id: &str,
        message_id: i64,
        canonical_key: &str,
    ) -> Result<Option<LearningCandidate>> {
        self.db
            .conn()
            .query_row(
                "SELECT id, user_id, project_dir, canonical_key, kind, proposed_content,
                        evidence_session_id, evidence_message_id, evidence_excerpt,
                        explicit, confidence, sensitivity, status, reason, created_at, reviewed_at
                 FROM mako_learning_candidates
                 WHERE evidence_session_id = ?1
                   AND evidence_message_id = ?2
                   AND canonical_key = ?3",
                params![session_id, message_id, canonical_key],
                map_candidate,
            )
            .optional()
            .map_err(Into::into)
    }
}

pub(crate) fn load_candidate_owned_from_connection(
    conn: &rusqlite::Connection,
    id: &str,
    user_id: Option<&str>,
) -> Result<Option<LearningCandidate>> {
    conn.query_row(
        "SELECT id, user_id, project_dir, canonical_key, kind, proposed_content,
                evidence_session_id, evidence_message_id, evidence_excerpt,
                explicit, confidence, sensitivity, status, reason, created_at, reviewed_at
         FROM mako_learning_candidates
         WHERE id = ?1
           AND ((?2 IS NULL AND user_id IS NULL) OR user_id = ?2)",
        params![id, user_id],
        map_candidate,
    )
    .optional()
    .map_err(Into::into)
}

fn map_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<LearningCandidate> {
    let kind: String = row.get(4)?;
    let sensitivity: String = row.get(11)?;
    let status: String = row.get(12)?;
    Ok(LearningCandidate {
        id: row.get(0)?,
        user_id: row.get(1)?,
        project_dir: row.get(2)?,
        canonical_key: row.get(3)?,
        kind: kind.parse().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                4,
                rusqlite::types::Type::Text,
                std::io::Error::new(std::io::ErrorKind::InvalidData, error).into(),
            )
        })?,
        proposed_content: row.get(5)?,
        evidence_session_id: row.get(6)?,
        evidence_message_id: row.get(7)?,
        evidence_excerpt: row.get(8)?,
        explicit: row.get::<_, i64>(9)? != 0,
        confidence: row.get(10)?,
        sensitivity: sensitivity.parse().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                11,
                rusqlite::types::Type::Text,
                std::io::Error::new(std::io::ErrorKind::InvalidData, error).into(),
            )
        })?,
        status: status.parse().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                std::io::Error::new(std::io::ErrorKind::InvalidData, error).into(),
            )
        })?,
        reason: row.get(13)?,
        created_at: row.get(14)?,
        reviewed_at: row.get(15)?,
    })
}
