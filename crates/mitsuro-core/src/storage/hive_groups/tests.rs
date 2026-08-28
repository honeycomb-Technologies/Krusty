use tempfile::TempDir;

use crate::storage::{Database, HiveWorkerStore, NewHiveWorker};

use super::{
    CappedGroupAppend, HiveGroupExecutionMode, HiveGroupSenderKind, HiveGroupStatus,
    HiveGroupStore, HiveGroupTurn, HiveGroupTurnPolicy, HiveGroupTurnStatus, HiveGroupUpdate,
    NewHiveGroup, NewHiveGroupMessage,
};

struct Fixture {
    store: HiveGroupStore,
    workers: Vec<String>,
    _temp: TempDir,
}

fn fixture() -> Fixture {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("groups.db");
    let workers = {
        let worker_store = HiveWorkerStore::new(Database::new(&path).unwrap());
        ["researcher", "reviewer", "builder"]
            .iter()
            .map(|slug| worker_store.create(&NewHiveWorker::new(*slug)).unwrap().id)
            .collect::<Vec<_>>()
    };
    Fixture {
        store: HiveGroupStore::new(Database::new(&path).unwrap()),
        workers,
        _temp: temp,
    }
}

fn new_group(fixture: &Fixture) -> NewHiveGroup {
    NewHiveGroup {
        title: "Release Room".into(),
        member_worker_ids: fixture.workers.clone(),
        ..NewHiveGroup::default()
    }
}

fn running_turn(group_id: &str, trigger_message_id: &str, plan: Vec<String>) -> HiveGroupTurn {
    let now = chrono::Utc::now().to_rfc3339();
    HiveGroupTurn {
        id: uuid::Uuid::new_v4().to_string(),
        group_id: group_id.to_string(),
        trigger_message_id: trigger_message_id.to_string(),
        execution_mode: HiveGroupExecutionMode::Workbench,
        policy: HiveGroupTurnPolicy {
            max_rounds: 3,
            max_member_messages_per_turn: 2,
            parallelism: 3,
            context_window_messages: 24,
        },
        speaker_plan: plan,
        next_speaker_index: 0,
        status: HiveGroupTurnStatus::Running,
        member_outcomes: None,
        started_at: now.clone(),
        finished_at: None,
        created_at: now.clone(),
        updated_at: now,
    }
}

#[test]
fn group_create_round_trips_with_ordered_members_and_defaults() {
    let fixture = fixture();
    let group = fixture.store.create(&new_group(&fixture)).unwrap();

    assert_eq!(group.title, "Release Room");
    assert_eq!(group.execution_mode, HiveGroupExecutionMode::Workbench);
    assert_eq!(group.max_rounds, 3);
    assert_eq!(group.max_member_messages_per_turn, 2);
    assert_eq!(group.parallelism, 3);
    assert_eq!(group.context_window_messages, 24);
    assert_eq!(group.status, HiveGroupStatus::Active);

    let members = fixture.store.members(&group.id).unwrap();
    assert_eq!(
        members
            .iter()
            .map(|member| member.worker_id.clone())
            .collect::<Vec<_>>(),
        fixture.workers
    );
    assert_eq!(
        members
            .iter()
            .map(|member| member.position)
            .collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    let roster = fixture.store.member_workers(&group.id).unwrap();
    assert_eq!(roster.len(), 3);
    assert_eq!(roster[0].slug, "researcher");
    assert_eq!(roster[2].slug, "builder");
}

#[test]
fn group_membership_validation_is_exact_owner_scoped() {
    let fixture = fixture();
    // A member owned by a different user is invisible to the local owner.
    let error = fixture
        .store
        .create(&NewHiveGroup {
            user_id: Some("alice".into()),
            ..new_group(&fixture)
        })
        .unwrap_err();
    assert!(error.to_string().contains("not found"), "{error}");

    // Unknown members fail closed.
    let error = fixture
        .store
        .create(&NewHiveGroup {
            member_worker_ids: vec!["missing".into()],
            title: "Ghost".into(),
            ..NewHiveGroup::default()
        })
        .unwrap_err();
    assert!(error.to_string().contains("not found"), "{error}");
}

#[test]
fn group_settings_update_and_membership_replacement() {
    let fixture = fixture();
    let group = fixture.store.create(&new_group(&fixture)).unwrap();

    let updated = fixture
        .store
        .update_settings(
            &group.id,
            &HiveGroupUpdate {
                title: "War Room".into(),
                execution_mode: HiveGroupExecutionMode::Direct,
                max_rounds: 2,
                max_member_messages_per_turn: 1,
                parallelism: 2,
                context_window_messages: 12,
                default_assignee_worker_id: Some(fixture.workers[1].clone()),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(updated.title, "War Room");
    assert_eq!(updated.execution_mode, HiveGroupExecutionMode::Direct);
    assert_eq!(
        updated.default_assignee_worker_id.as_deref(),
        Some(fixture.workers[1].as_str())
    );

    // A non-member default assignee is rejected.
    let error = fixture
        .store
        .update_settings(
            &group.id,
            &HiveGroupUpdate {
                default_assignee_worker_id: Some("missing".into()),
                ..HiveGroupUpdate::from(&updated)
            },
        )
        .unwrap_err();
    assert!(error.to_string().contains("member"), "{error}");

    // Removing the assignee from membership clears the assignment.
    fixture
        .store
        .set_members(&group.id, &[fixture.workers[0].clone()])
        .unwrap();
    let reloaded = fixture.store.get(&group.id).unwrap().unwrap();
    assert!(reloaded.default_assignee_worker_id.is_none());
    assert_eq!(fixture.store.members(&group.id).unwrap().len(), 1);

    // Archive is reversible visibility, not destruction.
    assert!(fixture
        .store
        .set_status(&group.id, HiveGroupStatus::Archived)
        .unwrap());
    assert!(fixture
        .store
        .list_for_owner(None, false)
        .unwrap()
        .is_empty());
    assert_eq!(fixture.store.list_for_owner(None, true).unwrap().len(), 1);
}

#[test]
fn combined_group_update_rejects_atomically_before_roster_mutation() {
    let fixture = fixture();
    let group = fixture.store.create(&new_group(&fixture)).unwrap();
    let original_members = fixture.store.members(&group.id).unwrap();

    let error = fixture
        .store
        .update_settings_and_members(
            &group.id,
            &HiveGroupUpdate {
                title: "   ".into(),
                ..HiveGroupUpdate::from(&group)
            },
            Some(&[fixture.workers[0].clone()]),
        )
        .unwrap_err();
    assert!(error.to_string().contains("title"), "{error}");
    assert_eq!(fixture.store.members(&group.id).unwrap(), original_members);
    assert_eq!(fixture.store.get(&group.id).unwrap(), Some(group));
}

#[test]
fn message_seq_is_monotonic_and_append_is_idempotent() {
    let fixture = fixture();
    let group = fixture.store.create(&new_group(&fixture)).unwrap();

    let first = fixture
        .store
        .append_message(&NewHiveGroupMessage {
            idempotency_key: Some("key-1".into()),
            ..NewHiveGroupMessage::user(&group.id, "hello room")
        })
        .unwrap();
    let second = fixture
        .store
        .append_message(&NewHiveGroupMessage::worker(
            &group.id,
            &fixture.workers[0],
            "hello back",
        ))
        .unwrap();
    assert_eq!(first.seq, 1);
    assert_eq!(second.seq, 2);
    assert_eq!(second.sender_kind, HiveGroupSenderKind::Worker);

    // Replaying the same idempotency key returns the original row.
    let replay = fixture
        .store
        .append_message(&NewHiveGroupMessage {
            idempotency_key: Some("key-1".into()),
            ..NewHiveGroupMessage::user(&group.id, "hello room CHANGED")
        })
        .unwrap();
    assert_eq!(replay.id, first.id);
    assert_eq!(replay.seq, 1);
    assert_eq!(replay.content, "hello room");
    assert_eq!(fixture.store.latest_seq(&group.id).unwrap(), 2);

    // Cursor pagination returns strictly-after rows in order.
    let after = fixture.store.list_messages_after(&group.id, 1, 10).unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].id, second.id);
    let recent = fixture.store.list_recent_messages(&group.id, 1).unwrap();
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].seq, 2);

    // Worker senders advance their cursor as they speak.
    let cursor = fixture
        .store
        .cursor(&group.id, &fixture.workers[0])
        .unwrap()
        .unwrap();
    assert_eq!(cursor.last_spoke_seq, 2);
    assert_eq!(cursor.last_seen_seq, 2);

    // Replies must stay inside the group.
    let other = fixture
        .store
        .create(&NewHiveGroup {
            title: "Other".into(),
            member_worker_ids: vec![fixture.workers[0].clone()],
            ..NewHiveGroup::default()
        })
        .unwrap();
    let error = fixture
        .store
        .append_message(&NewHiveGroupMessage {
            reply_to_message_id: Some(first.id),
            ..NewHiveGroupMessage::user(&other.id, "cross-group reply")
        })
        .unwrap_err();
    assert!(error.to_string().contains("not a message in this group"));
}

#[test]
fn capped_append_enforces_per_run_message_budget() {
    let fixture = fixture();
    let group = fixture.store.create(&new_group(&fixture)).unwrap();
    let trigger = fixture
        .store
        .append_message(&NewHiveGroupMessage::user(&group.id, "go"))
        .unwrap();
    let turn = running_turn(&group.id, &trigger.id, fixture.workers.clone());
    super::insert_turn_with_conn(fixture.store.conn(), &turn).unwrap();

    let post = |run: &str, text: &str| {
        fixture.store.append_worker_message_capped(
            &NewHiveGroupMessage {
                sender_run_id: Some(run.into()),
                turn_id: Some(turn.id.clone()),
                ..NewHiveGroupMessage::worker(&group.id, &fixture.workers[0], text)
            },
            2,
        )
    };
    assert!(matches!(
        post("run-1", "first").unwrap(),
        CappedGroupAppend::Appended { posted: 1, .. }
    ));
    assert!(matches!(
        post("run-1", "second").unwrap(),
        CappedGroupAppend::Appended { posted: 2, .. }
    ));
    assert_eq!(
        post("run-1", "third").unwrap(),
        CappedGroupAppend::CapExceeded { cap: 2, posted: 2 }
    );
    // Another run of the same worker (a later roundtable round) gets a
    // fresh budget.
    assert!(matches!(
        post("run-2", "fourth").unwrap(),
        CappedGroupAppend::Appended { posted: 1, .. }
    ));
}

#[test]
fn turn_lifecycle_progress_and_single_finalization() {
    let fixture = fixture();
    let group = fixture.store.create(&new_group(&fixture)).unwrap();
    let trigger = fixture
        .store
        .append_message(&NewHiveGroupMessage::user(&group.id, "go"))
        .unwrap();
    let turn = running_turn(&group.id, &trigger.id, fixture.workers.clone());
    super::insert_turn_with_conn(fixture.store.conn(), &turn).unwrap();

    let active = fixture.store.active_turn(&group.id).unwrap().unwrap();
    assert_eq!(active.id, turn.id);
    assert_eq!(active.speaker_plan, fixture.workers);
    assert_eq!(active.policy.max_member_messages_per_turn, 2);

    let now = chrono::Utc::now().to_rfc3339();
    assert!(
        super::update_turn_progress_with_conn(fixture.store.conn(), &turn.id, 1, &now).unwrap()
    );
    let outcomes = serde_json::json!({
        &fixture.workers[0]: {"status": "succeeded"},
        &fixture.workers[1]: {"status": "failed", "error": "provider unavailable"},
    });
    assert!(super::finalize_turn_with_conn(
        fixture.store.conn(),
        &turn.id,
        HiveGroupTurnStatus::Partial,
        Some(&outcomes),
        &now,
    )
    .unwrap());
    // A second finalization is a no-op: the terminal state is written once.
    assert!(!super::finalize_turn_with_conn(
        fixture.store.conn(),
        &turn.id,
        HiveGroupTurnStatus::Completed,
        None,
        &now,
    )
    .unwrap());

    let finished = fixture.store.get_turn(&turn.id).unwrap().unwrap();
    assert_eq!(finished.status, HiveGroupTurnStatus::Partial);
    assert_eq!(finished.next_speaker_index, 1);
    assert!(finished.finished_at.is_some());
    assert_eq!(
        finished.member_outcomes.unwrap()[&fixture.workers[1]]["status"],
        "failed"
    );
    assert!(fixture.store.active_turn(&group.id).unwrap().is_none());
    assert_eq!(fixture.store.list_turns(&group.id, 10).unwrap().len(), 1);
}

#[test]
fn cursors_only_move_forward() {
    let fixture = fixture();
    let group = fixture.store.create(&new_group(&fixture)).unwrap();
    fixture
        .store
        .advance_cursor(&group.id, &fixture.workers[0], Some(5), None)
        .unwrap();
    fixture
        .store
        .advance_cursor(&group.id, &fixture.workers[0], Some(3), Some(4))
        .unwrap();
    let cursor = fixture
        .store
        .cursor(&group.id, &fixture.workers[0])
        .unwrap()
        .unwrap();
    assert_eq!(cursor.last_seen_seq, 5);
    assert_eq!(cursor.last_spoke_seq, 4);
}

#[test]
fn deleting_a_group_cascades_membership_and_timeline_but_never_workers() {
    let fixture = fixture();
    let group = fixture.store.create(&new_group(&fixture)).unwrap();
    let trigger = fixture
        .store
        .append_message(&NewHiveGroupMessage::user(&group.id, "hello"))
        .unwrap();
    fixture
        .store
        .append_message(&NewHiveGroupMessage {
            reply_to_message_id: Some(trigger.id.clone()),
            ..NewHiveGroupMessage::worker(&group.id, &fixture.workers[0], "on it")
        })
        .unwrap();
    let turn = running_turn(&group.id, &trigger.id, fixture.workers.clone());
    super::insert_turn_with_conn(fixture.store.conn(), &turn).unwrap();
    fixture
        .store
        .advance_cursor(&group.id, &fixture.workers[0], Some(1), None)
        .unwrap();

    fixture
        .store
        .conn()
        .execute("DELETE FROM hive_groups WHERE id = ?1", [&group.id])
        .unwrap();

    for table in [
        "hive_group_members",
        "hive_group_messages",
        "hive_group_turns",
        "hive_member_cursors",
    ] {
        let count: i64 = fixture
            .store
            .conn()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE group_id = ?1"),
                [&group.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "{table} should cascade");
    }
    let workers: i64 = fixture
        .store
        .conn()
        .query_row("SELECT COUNT(*) FROM hive_workers", [], |row| row.get(0))
        .unwrap();
    assert_eq!(workers, 3, "workers must survive group deletion");
}
