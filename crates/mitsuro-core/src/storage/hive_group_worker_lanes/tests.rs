use std::sync::{Arc, Barrier};

use tempfile::TempDir;

use crate::storage::{Database, HiveGroupStore, HiveWorkerStore, NewHiveGroup, NewHiveWorker};

use super::{HiveGroupWorkerLaneStore, NewHiveGroupWorkerLane};

struct Fixture {
    path: std::path::PathBuf,
    group_id: String,
    worker_id: String,
    _temp: TempDir,
}

fn fixture() -> Fixture {
    let temp = TempDir::new().expect("temp dir");
    let path = temp.path().join("group-worker-lanes.db");
    let worker = HiveWorkerStore::new(Database::new(&path).expect("worker database"))
        .create(&NewHiveWorker::new("researcher"))
        .expect("create Worker");
    let group = HiveGroupStore::new(Database::new(&path).expect("group database"))
        .create(&NewHiveGroup {
            title: "Research room".into(),
            member_worker_ids: vec![worker.id.clone()],
            ..NewHiveGroup::default()
        })
        .expect("create group");
    Fixture {
        path,
        group_id: group.id,
        worker_id: worker.id,
        _temp: temp,
    }
}

fn insert_hive_session(path: &std::path::Path, id: &str, user_id: Option<&str>) {
    let db = Database::new(path).expect("session database");
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (
                 id, title, created_at, updated_at, session_type, user_id
             ) VALUES (?1, ?2, ?3, ?3, 'hive', ?4)",
            rusqlite::params![id, format!("Lane {id}"), now, user_id],
        )
        .expect("insert Hive session");
}

#[test]
fn upsert_adopts_the_first_canonical_session_without_rebinding() {
    let fixture = fixture();
    insert_hive_session(&fixture.path, "lane-a", None);
    insert_hive_session(&fixture.path, "lane-b", None);
    let store = HiveGroupWorkerLaneStore::new(Database::new(&fixture.path).unwrap());

    let first = store
        .upsert(&NewHiveGroupWorkerLane::new(
            &fixture.group_id,
            &fixture.worker_id,
            "lane-a",
        ))
        .expect("insert first lane");
    let adopted = store
        .upsert(&NewHiveGroupWorkerLane::new(
            &fixture.group_id,
            &fixture.worker_id,
            "lane-b",
        ))
        .expect("adopt existing lane");

    assert_eq!(first.session_id, "lane-a");
    assert_eq!(adopted, first);
    assert_eq!(
        store
            .load(&fixture.group_id, &fixture.worker_id)
            .expect("load lane"),
        Some(first)
    );
}

#[test]
fn concurrent_candidates_converge_on_one_canonical_lane() {
    let fixture = fixture();
    insert_hive_session(&fixture.path, "lane-a", None);
    insert_hive_session(&fixture.path, "lane-b", None);
    let barrier = Arc::new(Barrier::new(2));

    let handles = ["lane-a", "lane-b"].map(|session_id| {
        let path = fixture.path.clone();
        let group_id = fixture.group_id.clone();
        let worker_id = fixture.worker_id.clone();
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            let store = HiveGroupWorkerLaneStore::new(Database::new(&path).unwrap());
            barrier.wait();
            store
                .upsert(&NewHiveGroupWorkerLane::new(
                    group_id, worker_id, session_id,
                ))
                .expect("concurrent lane upsert")
        })
    });
    let lanes = handles.map(|handle| handle.join().expect("join lane creator"));

    assert_eq!(lanes[0], lanes[1]);
    assert!(matches!(lanes[0].session_id.as_str(), "lane-a" | "lane-b"));
    let db = Database::new(&fixture.path).unwrap();
    let count: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM hive_group_worker_lanes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn lane_requires_current_same_owner_membership_and_a_hive_session() {
    let fixture = fixture();
    let db = Database::new(&fixture.path).unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    db.conn()
        .execute(
            "INSERT INTO sessions (
                 id, title, created_at, updated_at, session_type, user_id
             ) VALUES ('wrong-kind', 'Chat', ?1, ?1, 'chat', NULL)",
            [&now],
        )
        .unwrap();
    let store = HiveGroupWorkerLaneStore::new(db);
    let error = store
        .upsert(&NewHiveGroupWorkerLane::new(
            &fixture.group_id,
            &fixture.worker_id,
            "wrong-kind",
        ))
        .unwrap_err();
    assert!(error.to_string().contains("same-owner member"), "{error}");

    insert_hive_session(&fixture.path, "direct-dm", None);
    let db = Database::new(&fixture.path).unwrap();
    db.conn()
        .execute(
            "UPDATE hive_workers SET dm_session_id = 'direct-dm' WHERE id = ?1",
            [&fixture.worker_id],
        )
        .unwrap();
    let error = HiveGroupWorkerLaneStore::new(db)
        .upsert(&NewHiveGroupWorkerLane::new(
            &fixture.group_id,
            &fixture.worker_id,
            "direct-dm",
        ))
        .unwrap_err();
    assert!(error.to_string().contains("non-DM"), "{error}");

    // Schema FKs cannot express the cross-table DM exclusion. A malformed
    // legacy row inserted outside the store must still fail closed when the
    // runtime loads or adopts it.
    let db = Database::new(&fixture.path).unwrap();
    db.conn()
        .execute(
            "INSERT INTO hive_group_worker_lanes (
                 group_id, worker_id, session_id, created_at, updated_at
             ) VALUES (?1, ?2, 'direct-dm', ?3, ?3)",
            rusqlite::params![fixture.group_id, fixture.worker_id, now],
        )
        .unwrap();
    let store = HiveGroupWorkerLaneStore::new(db);
    let error = store
        .load(&fixture.group_id, &fixture.worker_id)
        .unwrap_err();
    assert!(error.to_string().contains("non-DM"), "{error}");
    Database::new(&fixture.path)
        .unwrap()
        .conn()
        .execute(
            "DELETE FROM hive_group_worker_lanes WHERE group_id = ?1 AND worker_id = ?2",
            rusqlite::params![fixture.group_id, fixture.worker_id],
        )
        .unwrap();

    insert_hive_session(&fixture.path, "valid-lane", None);
    let error = HiveGroupWorkerLaneStore::new(Database::new(&fixture.path).unwrap())
        .upsert(&NewHiveGroupWorkerLane::new(
            &fixture.group_id,
            "missing-worker",
            "valid-lane",
        ))
        .unwrap_err();
    assert!(error.to_string().contains("same-owner member"), "{error}");
}

#[test]
fn group_and_worker_deletes_are_restricted_while_session_delete_cascades() {
    let fixture = fixture();
    insert_hive_session(&fixture.path, "lane-session", None);
    HiveGroupWorkerLaneStore::new(Database::new(&fixture.path).unwrap())
        .upsert(&NewHiveGroupWorkerLane::new(
            &fixture.group_id,
            &fixture.worker_id,
            "lane-session",
        ))
        .unwrap();
    let db = Database::new(&fixture.path).unwrap();

    assert!(db
        .conn()
        .execute("DELETE FROM hive_groups WHERE id = ?1", [&fixture.group_id])
        .is_err());
    db.conn()
        .execute(
            "DELETE FROM hive_group_members WHERE group_id = ?1 AND worker_id = ?2",
            rusqlite::params![fixture.group_id, fixture.worker_id],
        )
        .unwrap();
    assert!(db
        .conn()
        .execute(
            "DELETE FROM hive_workers WHERE id = ?1",
            [&fixture.worker_id]
        )
        .is_err());

    db.conn()
        .execute("DELETE FROM sessions WHERE id = 'lane-session'", [])
        .unwrap();
    let lanes: i64 = db
        .conn()
        .query_row("SELECT COUNT(*) FROM hive_group_worker_lanes", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(lanes, 0);
}
