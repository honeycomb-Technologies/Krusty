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
        project_dir: Some("/work/mako".to_string()),
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
    assert!(store.get_owned(&first.id, Some("bob")).unwrap().is_none());
    assert!(store.get_owned(&first.id, Some("alice")).unwrap().is_some());
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
