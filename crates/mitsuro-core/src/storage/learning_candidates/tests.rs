use tempfile::TempDir;

use super::{
    LearningCandidateInput, LearningCandidateStatus, LearningCandidateStore, LearningKind,
    LearningSensitivity,
};
use crate::storage::{Database, MessageStore, SessionManager};

fn seed_evidence(db: &Database, user_id: Option<&str>) -> (String, i64) {
    if let Some(user_id) = user_id {
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO users (id, email, license_tier) VALUES (?1, ?2, 'free')",
                rusqlite::params![user_id, format!("{user_id}@learning.test")],
            )
            .unwrap();
    }
    let manager = SessionManager::new(
        Database::new(std::path::Path::new(db.conn().path().unwrap())).unwrap(),
    );
    let session_id = manager
        .create_session_for_user("learning", Some("model"), None, user_id)
        .unwrap();
    manager
        .save_message(
            &session_id,
            "user",
            r#"[{"type":"text","text":"keep updates concise"}]"#,
        )
        .unwrap();
    let message_id = MessageStore::new(db)
        .load_session_message_records(&session_id)
        .unwrap()[0]
        .id;
    (session_id, message_id)
}

fn store_input(
    user_id: Option<&str>,
    evidence_session_id: String,
    evidence_message_id: i64,
) -> LearningCandidateInput {
    LearningCandidateInput {
        user_id: user_id.map(ToOwned::to_owned),
        project_dir: Some("/work/hive".to_string()),
        canonical_key: "communication.conciseness".to_string(),
        kind: LearningKind::UserPreference,
        proposed_content: "Prefer concise progress updates.".to_string(),
        evidence_session_id,
        evidence_message_id,
        evidence_excerpt: "keep updates concise".to_string(),
        explicit: true,
        confidence: 0.99,
        sensitivity: LearningSensitivity::Normal,
        status: LearningCandidateStatus::Pending,
        reason: "explicit preference".to_string(),
    }
}

#[test]
fn candidate_insert_is_idempotent_and_user_scoped() {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("learning.db")).unwrap();
    let store = LearningCandidateStore::new(&db);
    let (session_id, message_id) = seed_evidence(&db, Some("alice"));
    let input = store_input(Some("alice"), session_id, message_id);
    let first = store.insert(&input).unwrap();
    let second = store.insert(&input).unwrap();
    assert_eq!(first.id, second.id);
    assert_eq!(
        first.memory_namespace,
        crate::storage::MemoryNamespace::Shared
    );
    assert_eq!(
        first.memory_acl_scope,
        crate::storage::MemoryAclScope::Owner
    );
    assert!(first.memory_namespace_id.is_none());
    assert!(first.memory_scope_resolved);
    assert!(store.get_owned(&first.id, Some("bob")).unwrap().is_none());
    assert!(store.get_owned(&first.id, Some("alice")).unwrap().is_some());
}

#[test]
fn unresolved_group_run_learning_cannot_default_to_shared_memory() {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("learning.db")).unwrap();
    let (session_id, message_id) = seed_evidence(&db, None);
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO hive_controllers (
                 id, scope_key, session_id, status, timezone,
                 max_concurrent_runs, created_at, updated_at
             ) VALUES (
                 'group-controller', 'local:unresolved-group', '{session_id}',
                 'active', 'UTC', 1, '2026-08-24T00:00:00Z',
                 '2026-08-24T00:00:00Z'
             );
             INSERT INTO hive_runs (
                 id, controller_id, session_id, kind, objective, config_json,
                 status, available_at, max_attempts, created_at, updated_at,
                 group_id, group_turn_id
             ) VALUES (
                 'unresolved-group-run', 'group-controller', '{session_id}',
                 'group_turn', 'test unresolved binding', '{{}}', 'queued',
                 '2026-08-24T00:00:00Z', 3, '2026-08-24T00:00:00Z',
                 '2026-08-24T00:00:00Z', 'missing-group', 'missing-turn'
             );"
        ))
        .unwrap();

    let error = LearningCandidateStore::new(&db)
        .insert(&store_input(None, session_id, message_id))
        .unwrap_err();
    assert!(error.to_string().contains("no persisted lane binding"));
    let candidate_count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM hive_learning_candidates", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(candidate_count, 0);
}

#[test]
fn review_high_water_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("learning.db")).unwrap();
    let store = LearningCandidateStore::new(&db);
    let (session_id, message_id) = seed_evidence(&db, None);
    assert!(store
        .begin_review(&session_id, message_id, Some("model"))
        .unwrap());
    assert!(!store
        .begin_review(&session_id, message_id, Some("model"))
        .unwrap());
    assert!(store
        .has_nonfailed_review_covering(&session_id, message_id - 1)
        .unwrap());
    store.finish_review(&session_id, message_id, true).unwrap();
    assert!(store
        .has_nonfailed_review_covering(&session_id, message_id - 1)
        .unwrap());
}

#[test]
fn failed_review_can_be_reclaimed_but_completed_review_cannot() {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("learning.db")).unwrap();
    let store = LearningCandidateStore::new(&db);
    let (session_id, message_id) = seed_evidence(&db, None);
    assert!(store
        .begin_review(&session_id, message_id, Some("model-a"))
        .unwrap());
    store.finish_review(&session_id, message_id, false).unwrap();
    assert!(!store
        .has_nonfailed_review_covering(&session_id, message_id - 1)
        .unwrap());
    assert!(store
        .begin_review(&session_id, message_id, Some("model-b"))
        .unwrap());
    store.finish_review(&session_id, message_id, true).unwrap();
    assert!(!store
        .begin_review(&session_id, message_id, Some("model-c"))
        .unwrap());
}
