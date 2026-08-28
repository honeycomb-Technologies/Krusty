use chrono::Utc;
use tempfile::TempDir;

use super::{EpisodeSearch, EpisodeStore};
use crate::storage::{
    Database, HiveGroupStore, HiveGroupWorkerLaneStore, HiveWorkerStore, NewHiveGroup,
    NewHiveGroupWorkerLane, NewHiveWorker,
};

fn setup() -> (TempDir, Database) {
    let temp = TempDir::new().expect("tempdir");
    let db = Database::new(&temp.path().join("episodes.db")).expect("database");
    (temp, db)
}

fn create_session(db: &Database, user_id: Option<&str>, project_dir: &str) -> String {
    create_session_with_type(db, user_id, project_dir, "hive")
}

fn create_session_with_type(
    db: &Database,
    user_id: Option<&str>,
    project_dir: &str,
    session_type: &str,
) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    if let Some(user_id) = user_id {
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO users (id, email, license_tier)
                 VALUES (?1, ?2, 'free')",
                rusqlite::params![user_id, format!("{user_id}@episodes.test")],
            )
            .expect("user");
    }
    db.conn()
        .execute(
            "INSERT INTO sessions (
                id, title, created_at, updated_at, working_dir, project_dir,
                workspace_mode, user_id, session_type
             ) VALUES (?1, 'episode test', ?2, ?2, ?3, ?3, 'selected', ?4, ?5)",
            rusqlite::params![id, now, project_dir, user_id, session_type],
        )
        .expect("session");
    id
}

fn create_message(db: &Database, session_id: &str, role: &str, content: &str, now: &str) -> i64 {
    db.conn()
        .execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, role, content, now],
        )
        .expect("message");
    db.conn().last_insert_rowid()
}

#[test]
fn search_is_user_and_project_scoped() {
    let (_temp, db) = setup();
    let alice = create_session(&db, Some("alice"), "/work/alpha");
    let bob = create_session(&db, Some("bob"), "/work/alpha");
    let alice_other = create_session(&db, Some("alice"), "/work/beta");
    let local = create_session(&db, None, "/work/alpha");
    let store = EpisodeStore::new(&db);
    let now = Utc::now().to_rfc3339();

    for (session, text) in [
        (&alice, "the hive scheduler uses leases"),
        (&bob, "the hive scheduler belongs to bob"),
        (&alice_other, "the beta scheduler is separate"),
        (&local, "the local scheduler is unowned"),
    ] {
        let content = serde_json::json!([{"type": "text", "text": text}]).to_string();
        let message_id = create_message(&db, session, "user", &content, &now);
        store
            .record_message(session, message_id, "user", &content, &now)
            .expect("episode");
    }

    let mut query = EpisodeSearch::new("scheduler", Some("alice"));
    query.project_dir = Some("/work/alpha");
    let results = store.search(&query).expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].session_id, alice);

    let mut local_query = EpisodeSearch::new("scheduler", None);
    local_query.project_dir = Some("/work/alpha");
    let local_results = store.search(&local_query).expect("local search");
    assert_eq!(local_results.len(), 1);
    assert_eq!(local_results[0].session_id, local);
}

#[test]
fn record_is_idempotent_and_excludes_non_text_data() {
    let (_temp, db) = setup();
    let session = create_session(&db, None, "/work/local");
    let store = EpisodeStore::new(&db);
    let now = Utc::now().to_rfc3339();
    let content = serde_json::json!([
        {"type": "thinking", "thinking": "private", "signature": "sig"},
        {"type": "text", "text": "visible memory"},
        {"type": "tool_result", "tool_use_id": "t1", "output": "raw secret"}
    ])
    .to_string();
    let message_id = create_message(&db, &session, "assistant", &content, &now);

    let first = store
        .record_message(&session, message_id, "assistant", &content, &now)
        .expect("first")
        .expect("indexed");
    let second = store
        .record_message(&session, message_id, "assistant", &content, &now)
        .expect("second")
        .expect("indexed");
    assert_eq!(first, second);

    let results = store
        .search(&EpisodeSearch::new("visible", None))
        .expect("search");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].body, "visible memory");
    assert!(!results[0].body.contains("private"));
    assert!(!results[0].body.contains("secret"));
}

#[test]
fn search_filters_by_session_type_and_conversation() {
    let (_temp, db) = setup();
    let hive = create_session_with_type(&db, None, "/work/local", "hive");
    let chat = create_session_with_type(&db, None, "/work/local", "chat");
    let store = EpisodeStore::new(&db);
    let now = Utc::now().to_rfc3339();
    for (session, text) in [
        (&hive, "the hive scheduler uses leases"),
        (&chat, "the chat scheduler is a different conversation"),
    ] {
        let content = serde_json::json!([{"type": "text", "text": text}]).to_string();
        let message_id = create_message(&db, session, "user", &content, &now);
        store
            .record_message(session, message_id, "user", &content, &now)
            .expect("episode");
    }

    let mut hive_query = EpisodeSearch::new("scheduler", None);
    hive_query.session_type = Some("hive");
    let hive_results = store.search(&hive_query).expect("hive search");
    assert_eq!(hive_results.len(), 1);
    assert_eq!(hive_results[0].session_id, hive);

    let mut chat_query = EpisodeSearch::new("scheduler", None);
    chat_query.session_type = Some("chat");
    let chat_results = store.search(&chat_query).expect("chat search");
    assert_eq!(chat_results.len(), 1);
    assert_eq!(chat_results[0].session_id, chat);

    let mut scoped = EpisodeSearch::new("scheduler", None);
    scoped.session_id = Some(&chat);
    let scoped_results = store.search(&scoped).expect("conversation search");
    assert_eq!(scoped_results.len(), 1);
    assert_eq!(scoped_results[0].session_id, chat);
}

#[test]
fn owner_wide_episode_search_excludes_worker_dm_and_group_lanes_by_default() {
    let (temp, db) = setup();
    let db_path = temp.path().join("episodes.db");
    let ordinary = create_session(&db, Some("alice"), "/work/isolation");
    let worker_a_dm = create_session(&db, Some("alice"), "/work/isolation");
    let worker_b_dm = create_session(&db, Some("alice"), "/work/isolation");
    let worker_a_group = create_session(&db, Some("alice"), "/work/isolation");
    let workers = HiveWorkerStore::new(Database::new(&db_path).unwrap());
    let worker_a = workers
        .create(&NewHiveWorker {
            user_id: Some("alice".into()),
            dm_session_id: Some(worker_a_dm.clone()),
            ..NewHiveWorker::new("worker-a")
        })
        .unwrap();
    let worker_b = workers
        .create(&NewHiveWorker {
            user_id: Some("alice".into()),
            dm_session_id: Some(worker_b_dm.clone()),
            ..NewHiveWorker::new("worker-b")
        })
        .unwrap();
    let group = HiveGroupStore::new(Database::new(&db_path).unwrap())
        .create(&NewHiveGroup {
            user_id: Some("alice".into()),
            title: "Isolation Room".into(),
            member_worker_ids: vec![worker_a.id.clone(), worker_b.id.clone()],
            ..NewHiveGroup::default()
        })
        .unwrap();
    HiveGroupWorkerLaneStore::new(Database::new(&db_path).unwrap())
        .upsert(&NewHiveGroupWorkerLane::new(
            group.id,
            worker_a.id.clone(),
            worker_a_group.clone(),
        ))
        .unwrap();

    let store = EpisodeStore::new(&db);
    let now = Utc::now().to_rfc3339();
    for (session, label) in [
        (&ordinary, "ORDINARY-EPISODE"),
        (&worker_a_dm, "WORKER-A-DM-EPISODE"),
        (&worker_b_dm, "WORKER-B-DM-EPISODE"),
        (&worker_a_group, "WORKER-A-GROUP-EPISODE"),
    ] {
        let content = serde_json::json!([{
            "type": "text",
            "text": format!("isolation_canary {label}")
        }])
        .to_string();
        let message_id = create_message(&db, session, "user", &content, &now);
        store
            .record_message(session, message_id, "user", &content, &now)
            .unwrap();
    }

    let mut default_search = EpisodeSearch::new("isolation_canary", Some("alice"));
    default_search.project_dir = Some("/work/isolation");
    let default_results = store.search(&default_search).unwrap();
    assert_eq!(default_results.len(), 1);
    assert_eq!(default_results[0].session_id, ordinary);

    let mut worker_a_search = default_search.clone();
    worker_a_search.worker_id = Some(&worker_a.id);
    let worker_a_results = store.search(&worker_a_search).unwrap();
    let worker_a_sessions = worker_a_results
        .iter()
        .map(|episode| episode.session_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(worker_a_sessions.len(), 2);
    assert!(worker_a_sessions.contains(worker_a_dm.as_str()));
    assert!(worker_a_sessions.contains(worker_a_group.as_str()));
    assert!(!worker_a_sessions.contains(worker_b_dm.as_str()));
    assert!(!worker_a_sessions.contains(ordinary.as_str()));

    let mut worker_b_search = default_search.clone();
    worker_b_search.worker_id = Some(&worker_b.id);
    let worker_b_results = store.search(&worker_b_search).unwrap();
    assert_eq!(worker_b_results.len(), 1);
    assert_eq!(worker_b_results[0].session_id, worker_b_dm);

    default_search.include_worker_sessions = true;
    let diagnostic_results = store.search(&default_search).unwrap();
    assert_eq!(diagnostic_results.len(), 4);
}
