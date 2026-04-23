use crate::plan::PlanFile;

#[test]
fn test_create_plan() {
    let mut plan = PlanFile::new("Test Plan");
    let phase = plan.add_phase("Setup");
    phase.add_task("Install dependencies");
    phase.add_task("Configure environment");

    assert_eq!(plan.title, "Test Plan");
    assert_eq!(plan.phases.len(), 1);
    assert_eq!(plan.phases[0].tasks.len(), 2);
    assert_eq!(plan.phases[0].tasks[0].id, "1.1");
    assert_eq!(plan.phases[0].tasks[1].id, "1.2");
}

#[test]
fn test_check_task() {
    let mut plan = PlanFile::new("Test Plan");
    {
        let phase = plan.add_phase("Phase 1");
        phase.add_task("Task one");
    }

    assert!(
        !plan
            .find_task("1.1")
            .expect("valid regex pattern")
            .completed
    );
    assert!(plan.check_task("1.1"));
    assert!(
        plan.find_task("1.1")
            .expect("valid regex pattern")
            .completed
    );
}

#[test]
fn test_progress() {
    let mut plan = PlanFile::new("Test Plan");
    {
        let phase = plan.add_phase("Phase 1");
        phase.add_task("Task one");
        phase.add_task("Task two");
    }

    assert_eq!(plan.progress(), (0, 2));
    plan.check_task("1.1");
    assert_eq!(plan.progress(), (1, 2));
    plan.check_task("1.2");
    assert_eq!(plan.progress(), (2, 2));
    assert!(plan.is_complete());
}

#[test]
fn test_merge_plans() {
    let mut plan1 = PlanFile::new("Original Plan");
    {
        let phase = plan1.add_phase("Setup");
        phase.add_task("Task one");
        phase.add_task("Task two");
    }

    let mut plan2 = PlanFile::new("Updated Plan");
    {
        let phase = plan2.add_phase("Setup");
        phase.add_task("Task one");
    }
    plan2.check_task("1.1");

    plan1.merge_from(&plan2);

    assert!(
        plan1
            .find_task("1.1")
            .expect("valid regex pattern")
            .completed
    );
    assert!(plan1.find_task("1.2").is_some());
}

#[test]
fn test_find_nonexistent_task() {
    let mut plan = PlanFile::new("Test Plan");
    {
        let phase = plan.add_phase("Setup");
        phase.add_task("Task one");
    }

    assert!(plan.find_task("9.9").is_none());
    assert!(plan.find_task("invalid").is_none());
}

#[test]
fn test_check_nonexistent_task() {
    let mut plan = PlanFile::new("Test Plan");
    {
        let phase = plan.add_phase("Setup");
        phase.add_task("Task one");
    }

    assert!(!plan.check_task("9.9"));
    assert!(!plan.check_task("invalid"));
}

#[test]
fn test_merge_empty_plans() {
    let mut plan1 = PlanFile::new("Plan 1");
    let plan2 = PlanFile::new("Plan 2");

    plan1.merge_from(&plan2);
    assert_eq!(plan1.phases.len(), 0);
}

#[test]
fn test_merge_plans_different_phase_counts() {
    let mut plan1 = PlanFile::new("Plan 1");
    {
        let phase = plan1.add_phase("Phase 1");
        phase.add_task("Task 1.1");
    }
    {
        let phase = plan1.add_phase("Phase 2");
        phase.add_task("Task 2.1");
    }

    let mut plan2 = PlanFile::new("Plan 2");
    {
        let phase = plan2.add_phase("Phase 1");
        phase.add_task("Task 1.1");
    }

    plan2.check_task("1.1");
    plan1.merge_from(&plan2);

    assert_eq!(plan1.phases.len(), 2);
    assert!(
        plan1
            .find_task("1.1")
            .expect("valid regex pattern")
            .completed
    );
}
