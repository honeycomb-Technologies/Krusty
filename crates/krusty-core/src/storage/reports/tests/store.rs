use super::{create_store, CreateReportInput};

#[test]
fn create_and_get_report() {
    let (store, _tmp) = create_store();
    let id = store
        .create_report(CreateReportInput {
            title: "Auth Analysis",
            session_id: "sess-1",
            project_dir: Some("/home/user/project"),
            report_root: None,
            content: "# Auth\nDetailed analysis...",
            summary: "OAuth2 flow review",
            tags: &["auth".into(), "security".into()],
            sources: &["RFC 6749".into()],
        })
        .unwrap();

    let report = store.get_report(&id).unwrap().unwrap();
    assert_eq!(report.title, "Auth Analysis");
    assert_eq!(report.summary, "OAuth2 flow review");
    assert_eq!(report.tags, vec!["auth", "security"]);
    assert_eq!(report.sources, vec!["RFC 6749"]);
    assert_eq!(report.project_dir.as_deref(), Some("/home/user/project"));
}

#[test]
fn list_reports_by_project() {
    let (store, _tmp) = create_store();
    store
        .create_report(CreateReportInput {
            title: "Report A",
            session_id: "sess-1",
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
            title: "Report B",
            session_id: "sess-1",
            project_dir: Some("/proj-b"),
            report_root: None,
            content: "content b",
            summary: "",
            tags: &[],
            sources: &[],
        })
        .unwrap();
    store
        .create_report(CreateReportInput {
            title: "Report C",
            session_id: "sess-1",
            project_dir: Some("/proj-a"),
            report_root: None,
            content: "content c",
            summary: "",
            tags: &[],
            sources: &[],
        })
        .unwrap();

    let proj_a = store.list_reports(Some("/proj-a")).unwrap();
    assert_eq!(proj_a.len(), 2);

    let all = store.list_reports(None).unwrap();
    assert_eq!(all.len(), 3);
}

#[test]
fn search_reports_by_title_summary_tags_and_sources() {
    let (store, _tmp) = create_store();
    store
        .create_report(CreateReportInput {
            title: "Database Migration Guide",
            session_id: "sess-1",
            project_dir: None,
            report_root: None,
            content: "content",
            summary: "Steps for a safe schema rollout",
            tags: &["database".into(), "migration".into()],
            sources: &["docs/schema.md".into()],
        })
        .unwrap();
    store
        .create_report(CreateReportInput {
            title: "API Design",
            session_id: "sess-1",
            project_dir: None,
            report_root: None,
            content: "content",
            summary: "",
            tags: &["api".into()],
            sources: &[],
        })
        .unwrap();

    let results = store.search_reports("migration", None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Database Migration Guide");

    let results = store.search_reports("api", None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "API Design");

    let results = store.search_reports("safe schema", None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Database Migration Guide");

    let results = store.search_reports("docs/schema", None).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Database Migration Guide");
}

#[test]
fn delete_report() {
    let (store, _tmp) = create_store();
    let id = store
        .create_report(CreateReportInput {
            title: "Temporary",
            session_id: "sess-1",
            project_dir: None,
            report_root: None,
            content: "content",
            summary: "",
            tags: &[],
            sources: &[],
        })
        .unwrap();
    assert!(store.get_report(&id).unwrap().is_some());

    store.delete_report(&id).unwrap();
    assert!(store.get_report(&id).unwrap().is_none());
}
