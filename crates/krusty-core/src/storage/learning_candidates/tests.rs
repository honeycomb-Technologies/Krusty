use tempfile::TempDir;

use super::{
    LearningCandidateInput, LearningCandidateStatus, LearningCandidateStore, LearningKind,
    LearningSensitivity,
};
use crate::storage::Database;

fn store_input(user_id: Option<&str>) -> LearningCandidateInput {
    LearningCandidateInput {
        user_id: user_id.map(ToOwned::to_owned),
        project_dir: Some("/work/mako".to_string()),
        canonical_key: "communication.conciseness".to_string(),
        kind: LearningKind::UserPreference,
        proposed_content: "Prefer concise progress updates.".to_string(),
        evidence_session_id: "session-1".to_string(),
        evidence_message_id: 7,
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
    let first = store.insert(&store_input(Some("alice"))).unwrap();
    let second = store.insert(&store_input(Some("alice"))).unwrap();
    assert_eq!(first.id, second.id);
    assert!(store.get_owned(&first.id, Some("bob")).unwrap().is_none());
    assert!(store.get_owned(&first.id, Some("alice")).unwrap().is_some());
}

#[test]
fn review_high_water_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("learning.db")).unwrap();
    let store = LearningCandidateStore::new(&db);
    assert!(store.begin_review("session", 42, Some("model")).unwrap());
    assert!(!store.begin_review("session", 42, Some("model")).unwrap());
    store.finish_review("session", 42, true).unwrap();
}
