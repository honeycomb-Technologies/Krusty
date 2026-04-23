use super::{create_store, slugify, CreateReportInput};

#[test]
fn writes_reports_to_project_local_directory() {
    let (store, tmp) = create_store();
    let project_root = tmp.path().join("workspace");
    std::fs::create_dir_all(&project_root).unwrap();

    store
        .create_report(CreateReportInput {
            title: "Workspace Report",
            session_id: "sess-1",
            project_dir: Some(project_root.to_str().unwrap()),
            report_root: Some(&project_root),
            content: "content",
            summary: "",
            tags: &[],
            sources: &[],
        })
        .unwrap();

    let reports_dir = crate::paths::project_reports_dir(&project_root);
    let entries = std::fs::read_dir(reports_dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(entries.len(), 1);
    let content = std::fs::read_to_string(entries[0].path()).unwrap();
    assert!(content.contains("title: Workspace Report"));
    assert!(content.contains("session_id: sess-1"));
    assert!(content.contains("project_dir:"));
    assert!(content.contains("\n---\n\ncontent"));
}

#[test]
fn duplicate_titles_do_not_overwrite_report_files() {
    let (store, tmp) = create_store();
    let project_root = tmp.path().join("workspace");
    std::fs::create_dir_all(&project_root).unwrap();

    store
        .create_report(CreateReportInput {
            title: "Repeated Title",
            session_id: "sess-1",
            project_dir: Some(project_root.to_str().unwrap()),
            report_root: Some(&project_root),
            content: "first",
            summary: "",
            tags: &[],
            sources: &[],
        })
        .unwrap();
    store
        .create_report(CreateReportInput {
            title: "Repeated Title",
            session_id: "sess-1",
            project_dir: Some(project_root.to_str().unwrap()),
            report_root: Some(&project_root),
            content: "second",
            summary: "",
            tags: &[],
            sources: &[],
        })
        .unwrap();

    let reports_dir = crate::paths::project_reports_dir(&project_root);
    let mut names = std::fs::read_dir(reports_dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    names.sort();

    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1]);
}

#[test]
fn slugify_works() {
    assert_eq!(slugify("Auth Analysis"), "auth-analysis");
    assert_eq!(slugify("Hello, World!"), "hello-world");
    assert_eq!(slugify("  spaces  "), "spaces");
    assert_eq!(slugify("a--b"), "a-b");
    assert_eq!(slugify("!!!"), "report");
}
