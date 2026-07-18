use chrono::Utc;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use thiserror::Error;

use crate::storage::{
    load_candidate_owned_from_connection, load_canonical_for_provenance_from_connection,
    save_canonical_in_transaction, Database, LearningCandidate, LearningCandidateStatus,
    LearningCandidateStore, LearningKind,
};

use super::promotion::{canonical_input_for_candidate, scope_for_candidate};
use super::transcript::{canonical_text_content, normalize_whitespace};
use super::{LearningPolicy, LearningProposal, LearningScope};

const MAX_LIST_LIMIT: usize = 100;

#[derive(Debug, Error)]
pub enum LearningReviewServiceError {
    #[error("learning candidate not found")]
    NotFound,
    #[error("learning candidate is already {status}")]
    Conflict { status: LearningCandidateStatus },
    #[error("learning candidate cannot be promoted: {0}")]
    Policy(String),
    #[error("learning candidate evidence is invalid: {0}")]
    InvalidEvidence(String),
    #[error("learning review storage failure")]
    Storage(#[source] anyhow::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GovernedLearningReviewResult {
    pub candidate: LearningCandidate,
    pub memory_id: Option<String>,
    pub replayed: bool,
}

/// Exact-owner review boundary for Mako's pending durable-learning proposals.
///
/// Acceptance validates the current deterministic policy and original Mako
/// evidence, then writes canonical memory and the terminal candidate state in
/// one immediate transaction. Repeated terminal requests are idempotent only
/// when they request the same terminal state.
pub struct GovernedLearningReviewService {
    db: Database,
}

impl GovernedLearningReviewService {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub fn list_candidates(
        &self,
        user_id: Option<&str>,
        status: Option<LearningCandidateStatus>,
        limit: usize,
    ) -> Result<Vec<LearningCandidate>, LearningReviewServiceError> {
        LearningCandidateStore::new(&self.db)
            .list(user_id, status, limit.clamp(1, MAX_LIST_LIMIT))
            .map_err(storage_error)
    }

    pub fn accept_pending(
        &self,
        id: &str,
        user_id: Option<&str>,
    ) -> Result<GovernedLearningReviewResult, LearningReviewServiceError> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let candidate = owned_candidate(&tx, id, user_id)?;

        match candidate.status {
            LearningCandidateStatus::Accepted => {
                let canonical_input = canonical_input_for_candidate(&candidate)
                    .map_err(|error| LearningReviewServiceError::Policy(error.to_string()))?;
                let memory = load_canonical_for_provenance_from_connection(&tx, &canonical_input)
                    .map_err(storage_error)?
                    .ok_or_else(|| {
                        LearningReviewServiceError::Storage(anyhow::anyhow!(
                            "accepted learning candidate has no canonical memory"
                        ))
                    })?;
                tx.commit().map_err(storage_error)?;
                return Ok(GovernedLearningReviewResult {
                    candidate,
                    memory_id: Some(memory.id),
                    replayed: true,
                });
            }
            LearningCandidateStatus::Pending => {}
            status => return Err(LearningReviewServiceError::Conflict { status }),
        }

        validate_policy_compatibility(&candidate)?;
        validate_exact_evidence(&tx, &candidate)?;
        let canonical_input = canonical_input_for_candidate(&candidate)
            .map_err(|error| LearningReviewServiceError::Policy(error.to_string()))?;
        let memory = save_canonical_in_transaction(&tx, &canonical_input).map_err(storage_error)?;
        let reviewed_at = Utc::now().to_rfc3339();
        let changed = tx
            .execute(
                "UPDATE mako_learning_candidates
                 SET status = 'accepted',
                     reason = 'accepted by exact owner after governed review',
                     reviewed_at = ?1
                 WHERE id = ?2
                   AND ((?3 IS NULL AND user_id IS NULL) OR user_id = ?3)
                   AND status = 'pending'",
                params![reviewed_at, id, user_id],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            return Err(LearningReviewServiceError::Storage(anyhow::anyhow!(
                "candidate transition changed {changed} rows"
            )));
        }
        let candidate = owned_candidate(&tx, id, user_id)?;
        tx.commit().map_err(storage_error)?;

        Ok(GovernedLearningReviewResult {
            candidate,
            memory_id: Some(memory.id),
            replayed: false,
        })
    }

    pub fn reject_pending(
        &self,
        id: &str,
        user_id: Option<&str>,
    ) -> Result<GovernedLearningReviewResult, LearningReviewServiceError> {
        let tx = Transaction::new_unchecked(self.db.conn(), TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let candidate = owned_candidate(&tx, id, user_id)?;
        match candidate.status {
            LearningCandidateStatus::Rejected => {
                tx.commit().map_err(storage_error)?;
                return Ok(GovernedLearningReviewResult {
                    candidate,
                    memory_id: None,
                    replayed: true,
                });
            }
            LearningCandidateStatus::Pending => {}
            status => return Err(LearningReviewServiceError::Conflict { status }),
        }

        let changed = tx
            .execute(
                "UPDATE mako_learning_candidates
                 SET status = 'rejected',
                     reason = 'rejected by exact owner',
                     reviewed_at = ?1
                 WHERE id = ?2
                   AND ((?3 IS NULL AND user_id IS NULL) OR user_id = ?3)
                   AND status = 'pending'",
                params![Utc::now().to_rfc3339(), id, user_id],
            )
            .map_err(storage_error)?;
        if changed != 1 {
            return Err(LearningReviewServiceError::Storage(anyhow::anyhow!(
                "candidate transition changed {changed} rows"
            )));
        }
        let candidate = owned_candidate(&tx, id, user_id)?;
        tx.commit().map_err(storage_error)?;

        Ok(GovernedLearningReviewResult {
            candidate,
            memory_id: None,
            replayed: false,
        })
    }
}

fn owned_candidate(
    conn: &rusqlite::Connection,
    id: &str,
    user_id: Option<&str>,
) -> Result<LearningCandidate, LearningReviewServiceError> {
    load_candidate_owned_from_connection(conn, id, user_id)
        .map_err(storage_error)?
        .ok_or(LearningReviewServiceError::NotFound)
}

fn validate_policy_compatibility(
    candidate: &LearningCandidate,
) -> Result<(), LearningReviewServiceError> {
    if candidate.kind == LearningKind::Forget {
        return Err(LearningReviewServiceError::Policy(
            "forget candidates require the exact tombstone path and cannot be accepted as memory"
                .to_string(),
        ));
    }
    let scope = scope_for_candidate(candidate);
    match scope {
        LearningScope::User if candidate.project_dir.is_some() => {
            return Err(LearningReviewServiceError::Policy(
                "user-scoped candidates cannot target a project".to_string(),
            ));
        }
        LearningScope::Project if candidate.project_dir.is_none() => {
            return Err(LearningReviewServiceError::Policy(
                "project-scoped candidates require an exact project".to_string(),
            ));
        }
        LearningScope::User | LearningScope::Project => {}
    }
    let proposal = LearningProposal {
        canonical_key: candidate.canonical_key.clone(),
        kind: candidate.kind,
        scope,
        content: candidate.proposed_content.clone(),
        evidence_message_id: candidate.evidence_message_id,
        evidence_excerpt: candidate.evidence_excerpt.clone(),
        explicit: candidate.explicit,
        confidence: candidate.confidence,
        sensitivity: candidate.sensitivity,
    };
    let decision = LearningPolicy::evaluate(&proposal);
    if decision.status == LearningCandidateStatus::Rejected {
        return Err(LearningReviewServiceError::Policy(decision.reason));
    }
    Ok(())
}

fn validate_exact_evidence(
    conn: &rusqlite::Connection,
    candidate: &LearningCandidate,
) -> Result<(), LearningReviewServiceError> {
    let evidence = conn
        .query_row(
            "SELECT messages.role, messages.content, sessions.session_type, sessions.project_dir
             FROM messages
             JOIN sessions ON sessions.id = messages.session_id
             WHERE messages.id = ?1
               AND messages.session_id = ?2
               AND ((?3 IS NULL AND sessions.user_id IS NULL) OR sessions.user_id = ?3)",
            params![
                candidate.evidence_message_id,
                candidate.evidence_session_id,
                candidate.user_id
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(storage_error)?
        .ok_or_else(|| {
            LearningReviewServiceError::InvalidEvidence(
                "the cited message is not in an exact-owner session".to_string(),
            )
        })?;
    let (role, content_json, session_type, session_project_dir) = evidence;
    if role != "user" || session_type != "mako" {
        return Err(LearningReviewServiceError::InvalidEvidence(
            "the cited evidence must be a canonical user message in a Mako session".to_string(),
        ));
    }
    if scope_for_candidate(candidate) == LearningScope::Project
        && session_project_dir.as_deref() != candidate.project_dir.as_deref()
    {
        return Err(LearningReviewServiceError::InvalidEvidence(
            "the evidence session does not match the exact candidate project".to_string(),
        ));
    }

    let text = canonical_text_content(&content_json).ok_or_else(|| {
        LearningReviewServiceError::InvalidEvidence(
            "the cited message has no canonical text".to_string(),
        )
    })?;
    let excerpt = normalize_whitespace(&candidate.evidence_excerpt);
    if excerpt.is_empty() || !normalize_whitespace(&text).contains(&excerpt) {
        return Err(LearningReviewServiceError::InvalidEvidence(
            "the excerpt is not exact text from the cited user message".to_string(),
        ));
    }
    Ok(())
}

fn storage_error(error: impl Into<anyhow::Error>) -> LearningReviewServiceError {
    LearningReviewServiceError::Storage(error.into())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    use tempfile::TempDir;

    use super::{
        GovernedLearningReviewService, LearningCandidateStatus, LearningReviewServiceError,
    };
    use crate::storage::{
        Database, LearningCandidateInput, LearningCandidateStore, LearningKind,
        LearningSensitivity, MemorySource, MemoryStore, SessionManager, SessionType, WorkspaceMode,
    };

    struct Fixture {
        _temp: TempDir,
        db_path: PathBuf,
        candidate_id: String,
        session_id: String,
        message_id: i64,
    }

    fn fixture(key: &str, kind: LearningKind, content: &str, excerpt: &str) -> Fixture {
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("governed-learning-review.db");
        let db = Database::new(&db_path).unwrap();
        db.conn()
            .execute_batch(
                "INSERT INTO users (id, email, license_tier)
                     VALUES ('alice', 'alice@review.test', 'free');
                 INSERT INTO users (id, email, license_tier)
                     VALUES ('bob', 'bob@review.test', 'free');",
            )
            .unwrap();
        let manager = SessionManager::new(db);
        let session_id = manager
            .create_session_for_user_with_config(
                "Mako review",
                Some("test-model"),
                Some("/work/project"),
                Some("/work/project"),
                WorkspaceMode::Selected,
                Some("alice"),
                None,
                SessionType::Mako,
            )
            .unwrap();
        manager
            .save_message(
                &session_id,
                "user",
                &serde_json::json!([{"type": "text", "text": excerpt}]).to_string(),
            )
            .unwrap();
        manager
            .save_message(
                &session_id,
                "assistant",
                r#"[{"type":"text","text":"Understood."}]"#,
            )
            .unwrap();
        let db = Database::new(&db_path).unwrap();
        let message_id = db
            .conn()
            .query_row(
                "SELECT id FROM messages WHERE session_id = ?1 AND role = 'user'",
                [&session_id],
                |row| row.get(0),
            )
            .unwrap();
        let project_dir = matches!(kind, LearningKind::ProjectFact | LearningKind::Procedure)
            .then(|| "/work/project".to_string());
        let candidate = LearningCandidateStore::new(&db)
            .insert(&LearningCandidateInput {
                user_id: Some("alice".to_string()),
                project_dir,
                canonical_key: key.to_string(),
                kind,
                proposed_content: content.to_string(),
                evidence_session_id: session_id.clone(),
                evidence_message_id: message_id,
                evidence_excerpt: excerpt.to_string(),
                explicit: true,
                confidence: 0.99,
                sensitivity: LearningSensitivity::Normal,
                status: LearningCandidateStatus::Pending,
                reason: "requires review".to_string(),
            })
            .unwrap();
        Fixture {
            _temp: temp,
            db_path,
            candidate_id: candidate.id,
            session_id,
            message_id,
        }
    }

    #[test]
    fn accepting_project_candidate_is_atomic_idempotent_and_provenanced() {
        let fixture = fixture(
            "project.service.port",
            LearningKind::ProjectFact,
            "The project service listens on port 8443.",
            "The project service listens on port 8443.",
        );
        let service = GovernedLearningReviewService::new(Database::new(&fixture.db_path).unwrap());
        let accepted = service
            .accept_pending(&fixture.candidate_id, Some("alice"))
            .unwrap();
        assert_eq!(accepted.candidate.status, LearningCandidateStatus::Accepted);
        assert!(!accepted.replayed);

        let memory_id = accepted.memory_id.unwrap();
        let memory = MemoryStore::new(Database::new(&fixture.db_path).unwrap())
            .get_for_owner(&memory_id, Some("alice"))
            .unwrap()
            .unwrap();
        assert_eq!(memory.project_dir.as_deref(), Some("/work/project"));
        assert_eq!(memory.source, MemorySource::User);
        assert_eq!(
            memory.source_session_id.as_deref(),
            Some(fixture.session_id.as_str())
        );
        assert_eq!(
            memory.source_message_id.as_deref(),
            Some(fixture.message_id.to_string().as_str())
        );

        let replay = service
            .accept_pending(&fixture.candidate_id, Some("alice"))
            .unwrap();
        assert!(replay.replayed);
        assert_eq!(replay.memory_id.as_deref(), Some(memory_id.as_str()));
        let count: i64 = Database::new(&fixture.db_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM agent_memories
                 WHERE canonical_key = 'project.service.port'
                   AND user_id = 'alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn rejection_is_idempotent_and_blocks_later_acceptance() {
        let fixture = fixture(
            "project.service.port",
            LearningKind::ProjectFact,
            "The project service listens on port 8443.",
            "The project service listens on port 8443.",
        );
        let service = GovernedLearningReviewService::new(Database::new(&fixture.db_path).unwrap());
        let rejected = service
            .reject_pending(&fixture.candidate_id, Some("alice"))
            .unwrap();
        assert!(!rejected.replayed);
        assert!(
            service
                .reject_pending(&fixture.candidate_id, Some("alice"))
                .unwrap()
                .replayed
        );
        assert!(matches!(
            service.accept_pending(&fixture.candidate_id, Some("alice")),
            Err(LearningReviewServiceError::Conflict {
                status: LearningCandidateStatus::Rejected
            })
        ));
        assert!(MemoryStore::new(Database::new(&fixture.db_path).unwrap())
            .list(Some("/work/project"), Some("alice"))
            .is_empty());
    }

    #[test]
    fn exact_owner_mismatch_is_not_found() {
        let fixture = fixture(
            "project.service.port",
            LearningKind::ProjectFact,
            "The project service listens on port 8443.",
            "The project service listens on port 8443.",
        );
        let service = GovernedLearningReviewService::new(Database::new(&fixture.db_path).unwrap());
        assert!(matches!(
            service.accept_pending(&fixture.candidate_id, Some("bob")),
            Err(LearningReviewServiceError::NotFound)
        ));
        assert!(matches!(
            service.reject_pending(&fixture.candidate_id, None),
            Err(LearningReviewServiceError::NotFound)
        ));
    }

    #[test]
    fn policy_or_evidence_failure_leaves_candidate_pending_without_memory() {
        let policy_fixture = fixture(
            "preference.service.port",
            LearningKind::ProjectFact,
            "The project service listens on port 8443.",
            "The project service listens on port 8443.",
        );
        let service =
            GovernedLearningReviewService::new(Database::new(&policy_fixture.db_path).unwrap());
        assert!(matches!(
            service.accept_pending(&policy_fixture.candidate_id, Some("alice")),
            Err(LearningReviewServiceError::Policy(_))
        ));

        let evidence_fixture = fixture(
            "project.service.port",
            LearningKind::ProjectFact,
            "The project service listens on port 8443.",
            "The project service listens on port 8443.",
        );
        let db = Database::new(&evidence_fixture.db_path).unwrap();
        db.conn()
            .execute(
                "UPDATE mako_learning_candidates SET evidence_excerpt = 'not in the message'
                 WHERE id = ?1",
                [&evidence_fixture.candidate_id],
            )
            .unwrap();
        let evidence_service = GovernedLearningReviewService::new(db);
        assert!(matches!(
            evidence_service.accept_pending(&evidence_fixture.candidate_id, Some("alice")),
            Err(LearningReviewServiceError::InvalidEvidence(_))
        ));

        for fixture in [&policy_fixture, &evidence_fixture] {
            let db = Database::new(&fixture.db_path).unwrap();
            assert_eq!(
                LearningCandidateStore::new(&db)
                    .get_owned(&fixture.candidate_id, Some("alice"))
                    .unwrap()
                    .unwrap()
                    .status,
                LearningCandidateStatus::Pending
            );
            assert!(MemoryStore::new(Database::new(&fixture.db_path).unwrap())
                .list(Some("/work/project"), Some("alice"))
                .is_empty());
        }
    }

    #[test]
    fn concurrent_acceptance_creates_one_memory_and_one_replay() {
        let fixture = fixture(
            "project.service.port",
            LearningKind::ProjectFact,
            "The project service listens on port 8443.",
            "The project service listens on port 8443.",
        );
        let barrier = Arc::new(Barrier::new(2));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let service =
                GovernedLearningReviewService::new(Database::new(&fixture.db_path).unwrap());
            let barrier = barrier.clone();
            let candidate_id = fixture.candidate_id.clone();
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                service
                    .accept_pending(&candidate_id, Some("alice"))
                    .unwrap()
            }));
        }
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.replayed).count(), 1);
        assert_eq!(
            results[0].memory_id.as_deref(),
            results[1].memory_id.as_deref()
        );
        let count: i64 = Database::new(&fixture.db_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM agent_memories
                 WHERE canonical_key = 'project.service.port'
                   AND user_id = 'alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }
}
