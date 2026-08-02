use super::{create_store_with_users, CreateReportInput};

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
