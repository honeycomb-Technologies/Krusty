use crate::plan::PlanFile;

#[test]
fn test_try_parse_from_response() {
    let response = r#"
I'll create a plan for implementing authentication.

# Plan: Authentication System

## Phase 1: Database Setup

- [ ] Task 1.1: Create users table
- [ ] Task 1.2: Add password hashing

## Phase 2: API Endpoints

- [ ] Task 2.1: Implement login endpoint
- [x] Task 2.2: Already completed signup

Let me know if you have questions!
"#;

    let plan = PlanFile::try_parse_from_response(response).expect("valid regex pattern");
    assert_eq!(plan.title, "Authentication System");
    assert_eq!(plan.phases.len(), 2);
    assert_eq!(plan.phases[0].tasks.len(), 2);
    assert_eq!(plan.phases[1].tasks.len(), 2);
    assert!(
        !plan
            .find_task("1.1")
            .expect("valid regex pattern")
            .completed
    );
    assert!(
        plan.find_task("2.2")
            .expect("valid regex pattern")
            .completed
    );
}

#[test]
fn test_try_parse_no_explicit_task_ids() {
    let response = r#"
# Plan: Quick Tasks

## Phase 1: Setup

- [ ] Install dependencies
- [ ] Configure environment
- [x] Done with prerequisites
"#;

    let plan = PlanFile::try_parse_from_response(response).expect("valid regex pattern");
    assert_eq!(plan.title, "Quick Tasks");
    assert_eq!(plan.phases[0].tasks.len(), 3);
    assert_eq!(plan.phases[0].tasks[0].id, "1.1");
    assert_eq!(plan.phases[0].tasks[0].description, "Install dependencies");
    assert!(plan.phases[0].tasks[2].completed);
}

#[test]
fn test_try_parse_no_valid_structure() {
    let response = "Just a normal response without any plan structure.";
    assert!(PlanFile::try_parse_from_response(response).is_none());

    let response2 = "# Plan: Title Only";
    assert!(PlanFile::try_parse_from_response(response2).is_none());
}

#[test]
fn test_extract_completed_task_ids() {
    let text1 = "- [x] Task 1.1: Create database\n- [ ] Task 1.2: Add indexes";
    let ids1 = PlanFile::extract_completed_task_ids(text1);
    assert_eq!(ids1, vec!["1.1"]);

    let text2 = "I've finished the work. Task 2.1 is complete and Task 2.2 is done.";
    let ids2 = PlanFile::extract_completed_task_ids(text2);
    assert!(ids2.contains(&"2.1".to_string()));
    assert!(ids2.contains(&"2.2".to_string()));

    let text3 = "I completed Task 3.1 and finished Task 3.2.";
    let ids3 = PlanFile::extract_completed_task_ids(text3);
    assert!(ids3.contains(&"3.1".to_string()));
    assert!(ids3.contains(&"3.2".to_string()));

    let text4 = "✓ Task 4.1\n✅ Task 4.2";
    let ids4 = PlanFile::extract_completed_task_ids(text4);
    assert!(ids4.contains(&"4.1".to_string()));
    assert!(ids4.contains(&"4.2".to_string()));

    let text5 = "Working on the tasks now.";
    let ids5 = PlanFile::extract_completed_task_ids(text5);
    assert!(ids5.is_empty());
}

#[test]
fn test_task_completion_pattern_edge_cases() {
    let test_cases = vec![
        "The task is not completed yet",
        "Working on task number 1.1",
        "This completes our work",
    ];

    for text in test_cases {
        let ids = PlanFile::extract_completed_task_ids(text);
        assert!(
            ids.is_empty(),
            "Text '{}' should not match any task completion patterns",
            text
        );
    }
}

#[test]
fn test_multiple_task_completions_in_one_line() {
    let text = "I've completed Task 1.1, Task 1.2, and finished Task 1.3";
    let ids = PlanFile::extract_completed_task_ids(text);
    assert!(ids.len() >= 2);
    assert!(ids.contains(&"1.1".to_string()));
}
