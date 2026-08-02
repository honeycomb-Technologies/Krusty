use chrono::Utc;

use crate::plan::{PlanFile, PlanStatus};

#[test]
fn test_markdown_roundtrip() {
    let mut plan = PlanFile::new("Test Plan");
    plan.session_id = Some("test-session".to_string());
    plan.working_dir = Some("/tmp/test".to_string());
    {
        let phase = plan.add_phase("Setup");
        phase.add_task("Install deps");
        phase.add_task("Configure");
    }
    plan.check_task("1.1");
    plan.notes = Some("Some notes here".to_string());

    let markdown = plan.to_markdown();
    let parsed = PlanFile::from_markdown(&markdown).expect("valid regex pattern");

    assert_eq!(parsed.title, plan.title);
    assert_eq!(parsed.session_id, plan.session_id);
    assert_eq!(parsed.working_dir, plan.working_dir);
    assert_eq!(parsed.phases.len(), plan.phases.len());
    assert_eq!(parsed.phases[0].tasks.len(), 2);
    assert!(
        parsed
            .find_task("1.1")
            .expect("valid regex pattern")
            .completed
    );
    assert!(
        !parsed
            .find_task("1.2")
            .expect("valid regex pattern")
            .completed
    );
    assert!(parsed.notes.is_some());
}

#[test]
fn test_parse_empty_plan() {
    let result = PlanFile::from_markdown("");
    assert!(result.is_err(), "Empty plan should error");
}

#[test]
fn test_parse_plan_no_title() {
    let markdown = r#"
## Phase 1: Setup

- [ ] Task 1.1: Some task
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_err() || result.expect("valid regex pattern").phases.is_empty());
}

#[test]
fn test_parse_plan_no_phases() {
    let markdown = "# Plan: Test Plan\n";
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.phases.len(), 0);
}

#[test]
fn test_parse_plan_invalid_status() {
    let markdown = r#"
# Plan: Test Plan

Status: invalid_status_value

## Phase 1: Setup

- [ ] Task 1.1: Some task
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.status, PlanStatus::InProgress);
}

#[test]
fn test_parse_plan_invalid_date() {
    let markdown = r#"
# Plan: Test Plan

Created: not-a-date

## Phase 1: Setup

- [ ] Task 1.1: Some task
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_ne!(plan.created_at, Utc::now());
}

#[test]
fn test_parse_plan_invalid_phase_number() {
    let markdown = r#"
# Plan: Test Plan

## Phase not-a-number: Setup

- [ ] Task 1.1: Some task
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.phases.len(), 1);
    assert_eq!(plan.phases[0].number, 1);
}

#[test]
fn test_parse_plan_task_without_phase() {
    let markdown = r#"
# Plan: Test Plan

- [ ] Task 1.1: Orphan task
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.phases.len(), 0);
}

#[test]
fn test_parse_plan_empty_task_description() {
    let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1:
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.phases[0].tasks[0].id, "1.1");
    assert_eq!(plan.phases[0].tasks[0].description, "");
}

#[test]
fn test_parse_plan_task_with_colon_in_description() {
    let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1: Install: configure, and test
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(
        plan.phases[0].tasks[0].description,
        "Install: configure, and test"
    );
}

#[test]
fn test_parse_plan_mixed_task_formats() {
    let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1: Explicit ID
- [ ] Just a description
- [x] Task 1.3: With checkbox
- [ ] Another description
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.phases[0].tasks.len(), 4);
    assert_eq!(plan.phases[0].tasks[0].id, "1.1");
    assert_eq!(plan.phases[0].tasks[1].id, "1.2");
    assert_eq!(plan.phases[0].tasks[2].id, "1.3");
    assert!(plan.phases[0].tasks[2].completed);
    assert_eq!(plan.phases[0].tasks[3].id, "1.4");
}

#[test]
fn test_parse_plan_empty_phase_name() {
    let markdown = r#"
# Plan: Test Plan

## Phase 1:

- [ ] Task 1.1: Some task
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.phases[0].name, "");
}

#[test]
fn test_parse_plan_notes_section() {
    let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1: Some task

## Notes

These are important notes
that span multiple lines.
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert!(plan.notes.is_some());
    assert!(plan
        .notes
        .as_ref()
        .expect("valid regex pattern")
        .contains("important notes"));
}

#[test]
fn test_parse_plan_notes_before_tasks() {
    let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.1: Some task

## Notes

Some notes here

## Phase 2: Next Phase

- [ ] Task 2.1: Another task
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.phases.len(), 1);
    assert!(plan
        .notes
        .as_ref()
        .expect("valid regex pattern")
        .contains("## Phase 2"));
}

#[test]
fn test_parse_plan_with_metadata_only() {
    let markdown = r#"
# Plan: Test Plan

Created: 2024-01-15 10:00 UTC
Session: abc123
Working Directory: /tmp/test
Status: completed
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.title, "Test Plan");
    assert_eq!(plan.session_id, Some("abc123".to_string()));
    assert_eq!(plan.working_dir, Some("/tmp/test".to_string()));
    assert_eq!(plan.status, PlanStatus::Completed);
    assert_eq!(plan.phases.len(), 0);
}

#[test]
fn test_parse_plan_multiple_spaces_in_checkbox() {
    let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [  ] Task 1.1: Extra spaces in bracket
- [x] Task 1.2: Normal completed
- [X] Task 1.3: Uppercase X
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let _plan = result.expect("valid regex pattern");
}

#[test]
fn test_parse_plan_status_variations() {
    let test_cases = vec![
        ("in_progress", PlanStatus::InProgress),
        ("inprogress", PlanStatus::InProgress),
        ("completed", PlanStatus::Completed),
        ("complete", PlanStatus::Completed),
        ("done", PlanStatus::Completed),
        ("abandoned", PlanStatus::Abandoned),
        ("cancelled", PlanStatus::Abandoned),
        ("canceled", PlanStatus::Abandoned),
    ];

    for (status_str, expected) in test_cases {
        let markdown = format!(
            r#"
# Plan: Test Plan

Status: {}

## Phase 1: Setup

- [ ] Task 1.1: Some task
"#,
            status_str
        );

        let result = PlanFile::from_markdown(&markdown);
        assert!(result.is_ok());
        let plan = result.expect("valid regex pattern");
        assert_eq!(
            plan.status, expected,
            "Status '{}' should parse to {:?}",
            status_str, expected
        );
    }
}

#[test]
fn test_parse_plan_task_id_with_decimals() {
    let markdown = r#"
# Plan: Test Plan

## Phase 1: Setup

- [ ] Task 1.10: Task with decimal
- [ ] Task 1.2: Another task
"#;
    let result = PlanFile::from_markdown(markdown);
    assert!(result.is_ok());
    let plan = result.expect("valid regex pattern");
    assert_eq!(plan.phases[0].tasks[0].id, "1.10");
    assert_eq!(plan.phases[0].tasks[1].id, "1.2");
}
