use super::{create_store_with_users, CreateReportInput, ReportScope};
use crate::storage::{Database, HiveWorkerStore, NewHiveWorker, ReportStore};

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
            scope: ReportScope::owner_shared(),
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
            scope: ReportScope::owner_shared(),
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
fn exact_owner_report_listing_isolates_alice_bob_and_local() {
    let (store, _tmp) = create_store_with_users();
    for (title, session_id) in [
        ("Local Queue", "sess-1"),
        ("Alice Queue", "sess-a"),
        ("Bob Queue", "sess-b"),
    ] {
        store
            .create_report(CreateReportInput {
                title,
                session_id,
                project_dir: Some("/shared"),
                report_root: None,
                content: title,
                summary: "queue ownership evidence",
                tags: &[],
                sources: &[],
                scope: ReportScope::owner_shared(),
            })
            .unwrap();
    }

    let alice = store
        .list_reports_for_exact_owner(Some("/shared"), Some("user-a"))
        .unwrap();
    let bob = store
        .list_reports_for_exact_owner(Some("/shared"), Some("user-b"))
        .unwrap();
    let local = store
        .list_reports_for_exact_owner(Some("/shared"), None)
        .unwrap();

    assert_eq!(alice.len(), 1);
    assert_eq!(alice[0].title, "Alice Queue");
    assert_eq!(bob.len(), 1);
    assert_eq!(bob[0].title, "Bob Queue");
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].title, "Local Queue");
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
            scope: ReportScope::owner_shared(),
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

#[test]
fn search_reports_for_user_honors_owner_scope() {
    let (store, _tmp) = create_store_with_users();
    store
        .create_report(CreateReportInput {
            title: "Alice Architecture",
            session_id: "sess-a",
            project_dir: Some("/proj-a"),
            report_root: None,
            content: "content a",
            summary: "queue policy notes",
            tags: &["ops".into()],
            sources: &["alice.md".into()],
            scope: ReportScope::owner_shared(),
        })
        .unwrap();
    store
        .create_report(CreateReportInput {
            title: "Bob Architecture",
            session_id: "sess-b",
            project_dir: Some("/proj-b"),
            report_root: None,
            content: "content b",
            summary: "queue policy notes",
            tags: &["ops".into()],
            sources: &["bob.md".into()],
            scope: ReportScope::owner_shared(),
        })
        .unwrap();

    let user_a_results = store
        .search_reports_for_user("queue policy", None, Some("user-a"))
        .unwrap();
    assert_eq!(user_a_results.len(), 1);
    assert_eq!(user_a_results[0].title, "Alice Architecture");

    let scoped_results = store
        .search_reports_for_user("alice.md", Some("/proj-a"), Some("user-a"))
        .unwrap();
    assert_eq!(scoped_results.len(), 1);
    assert_eq!(scoped_results[0].title, "Alice Architecture");

    let hidden_results = store
        .search_reports_for_user("queue policy", None, Some("user-b"))
        .unwrap();
    assert_eq!(hidden_results.len(), 1);
    assert_eq!(hidden_results[0].title, "Bob Architecture");
}

#[test]
fn frozen_worker_report_scope_survives_dm_and_session_owner_rebinds() {
    let temp = tempfile::TempDir::new().expect("temp dir");
    let path = temp.path().join("frozen-report-scope.db");
    let db = Database::new(&path).expect("database");
    db.conn()
        .execute_batch(
            "INSERT INTO users (id, email, license_tier) VALUES
                 ('alice', 'alice@example.com', 'free'),
                 ('bob', 'bob@example.com', 'free');
             INSERT INTO sessions (
                 id, title, created_at, updated_at, session_type, user_id
             ) VALUES
                 ('ordinary', 'ordinary', datetime('now'), datetime('now'), 'code', 'alice'),
                 ('primary-hive', 'primary', datetime('now'), datetime('now'), 'hive', 'alice'),
                 ('worker-a-old', 'old dm', datetime('now'), datetime('now'), 'hive', 'alice'),
                 ('worker-a-new', 'new dm', datetime('now'), datetime('now'), 'hive', 'alice'),
                 ('worker-b-dm', 'worker b', datetime('now'), datetime('now'), 'hive', 'alice');",
        )
        .expect("seed owners and sessions");
    drop(db);

    let worker_store = HiveWorkerStore::new(Database::new(&path).unwrap());
    let worker_a = worker_store
        .create(&NewHiveWorker {
            user_id: Some("alice".into()),
            dm_session_id: Some("worker-a-old".into()),
            memory_namespace_id: Some("crew-a".into()),
            ..NewHiveWorker::new("worker-a")
        })
        .unwrap();
    let worker_b = worker_store
        .create(&NewHiveWorker {
            user_id: Some("alice".into()),
            dm_session_id: Some("worker-b-dm".into()),
            memory_namespace_id: Some("crew-b".into()),
            ..NewHiveWorker::new("worker-b")
        })
        .unwrap();

    let reports = ReportStore::new(Database::new(&path).unwrap());
    let shared_id = reports
        .create_report(CreateReportInput {
            title: "Shared report",
            session_id: "ordinary",
            project_dir: Some("/project"),
            report_root: None,
            content: "shared",
            summary: "frozen-scope-canary",
            tags: &[],
            sources: &[],
            scope: ReportScope::owner_shared(),
        })
        .unwrap();
    let private_id = reports
        .create_report(CreateReportInput {
            title: "Worker A old report",
            session_id: "worker-a-old",
            project_dir: Some("/project"),
            report_root: None,
            content: "private",
            summary: "frozen-scope-canary",
            tags: &[],
            sources: &[],
            scope: ReportScope::worker_private(
                worker_a.id.clone(),
                worker_a.memory_namespace_id.clone(),
            )
            .unwrap(),
        })
        .unwrap();
    assert!(reports
        .create_report(CreateReportInput {
            title: "Forged shared report",
            session_id: "worker-a-old",
            project_dir: None,
            report_root: None,
            content: "forged",
            summary: "",
            tags: &[],
            sources: &[],
            scope: ReportScope::owner_shared(),
        })
        .is_err());
    assert!(reports
        .create_report(CreateReportInput {
            title: "Forged Worker report",
            session_id: "ordinary",
            project_dir: None,
            report_root: None,
            content: "forged",
            summary: "",
            tags: &[],
            sources: &[],
            scope: ReportScope::worker_private(
                worker_a.id.clone(),
                worker_a.memory_namespace_id.clone(),
            )
            .unwrap(),
        })
        .is_err());

    worker_store
        .bind_dm_session(&worker_a.id, Some("worker-a-new"))
        .unwrap();
    Database::new(&path)
        .unwrap()
        .conn()
        .execute(
            "UPDATE sessions SET user_id = 'bob' WHERE id = 'worker-a-old'",
            [],
        )
        .expect("mutate historical source session owner");

    let reports = ReportStore::new(Database::new(&path).unwrap());
    assert!(reports
        .get_report_for_memory_reader(&private_id, Some("alice"), Some(&worker_a.id))
        .unwrap()
        .is_some());
    for reader in [None, Some(worker_b.id.as_str())] {
        assert!(reports
            .get_report_for_memory_reader(&private_id, Some("alice"), reader)
            .unwrap()
            .is_none());
    }
    assert!(reports
        .get_report_for_memory_reader(&private_id, Some("bob"), Some(&worker_a.id))
        .unwrap()
        .is_none());
    assert!(reports
        .get_report_for_user(&private_id, Some("alice"))
        .unwrap()
        .is_none());
    assert!(reports
        .get_report_for_memory_reader(&shared_id, Some("alice"), None)
        .unwrap()
        .is_some());

    let listed = reports
        .list_reports_for_memory_reader(Some("/project"), Some("alice"), Some(&worker_a.id))
        .unwrap();
    assert_eq!(listed.len(), 2);
    let searched = reports
        .search_reports_for_memory_reader(
            "frozen-scope-canary",
            Some("/project"),
            Some("alice"),
            Some(&worker_a.id),
        )
        .unwrap();
    assert_eq!(searched.len(), 2);
    let primary = reports
        .list_reports_for_memory_reader(Some("/project"), Some("alice"), None)
        .unwrap();
    assert_eq!(primary.len(), 1);
    assert_eq!(primary[0].id, shared_id);

    let raw_db = Database::new(&path).unwrap();
    assert!(raw_db
        .conn()
        .execute(
            "UPDATE reports SET source_worker_id = NULL WHERE id = ?1",
            [&private_id],
        )
        .is_err());
    assert!(raw_db
        .conn()
        .execute(
            "INSERT INTO reports (
                 id, title, session_id, content, owner_user_id,
                 memory_namespace, namespace_id, acl_scope, source_worker_id
             ) VALUES (
                 'malformed', 'malformed', 'ordinary', 'malformed', 'alice',
                 'shared', NULL, 'worker', ?1
             )",
            [&worker_a.id],
        )
        .is_err());
    assert!(raw_db
        .conn()
        .execute(
            "INSERT INTO reports (
                 id, title, session_id, content, owner_user_id,
                 memory_namespace, namespace_id, acl_scope, source_worker_id
             ) VALUES (
                 'forged-shared', 'forged', 'worker-a-new', 'forged', 'alice',
                 'shared', NULL, 'owner', NULL
             )",
            [],
        )
        .is_err());
}
