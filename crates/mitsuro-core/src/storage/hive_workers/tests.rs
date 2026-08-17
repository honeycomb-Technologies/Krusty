use tempfile::TempDir;

use crate::ai::models::{ApiFormat, ModelKey};
use crate::ai::providers::ProviderId;
use crate::storage::Database;
use crate::tools::registry::PermissionMode;

use super::{
    display_name_from_slug, HiveWorkerAutonomy, HiveWorkerDocumentKind, HiveWorkerProfileUpdate,
    HiveWorkerStatus, HiveWorkerStore, NewHiveWorker,
};

fn store() -> (HiveWorkerStore, TempDir) {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("workers.db")).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('dm-1', 'Worker DM', '2026-08-01T00:00:00.000000Z',
                     '2026-08-01T00:00:00.000000Z', 'hive');
             INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('dm-2', 'Worker DM 2', '2026-08-01T00:00:00.000000Z',
                     '2026-08-01T00:00:00.000000Z', 'hive');",
        )
        .unwrap();
    (HiveWorkerStore::new(db), temp)
}

#[test]
fn worker_create_round_trips_and_scopes_by_owner() {
    let (store, _temp) = store();
    let local = store
        .create(&NewHiveWorker::new("builder"))
        .expect("create local builder");
    assert_eq!(local.slug, "builder");
    assert_eq!(local.display_name, "Builder");
    assert_eq!(local.memory_namespace_id, "builder");
    assert_eq!(local.status, HiveWorkerStatus::Active);
    assert_eq!(local.autonomy, HiveWorkerAutonomy::Manual);
    assert_eq!(local.permission_mode, PermissionMode::Autonomous);
    assert!(local.user_id.is_none());

    let alice = store
        .create(&NewHiveWorker {
            user_id: Some("alice".into()),
            model: Some("grok-code-fast-1".into()),
            model_key: Some(ModelKey::new(
                ProviderId::Grok,
                "grok-code-fast-1",
                ApiFormat::OpenAIResponses,
            )),
            model_catalog_revision: Some("catalog-42".into()),
            ..NewHiveWorker::new("builder")
        })
        .expect("create alice builder");
    assert_eq!(alice.user_id.as_deref(), Some("alice"));
    assert_eq!(
        alice.model_key.as_ref().map(|key| key.model_id.as_str()),
        Some("grok-code-fast-1")
    );

    // Same slug under different exact owners is two distinct workers.
    let local_loaded = store.get_by_slug(None, "builder").unwrap().unwrap();
    let alice_loaded = store
        .get_by_slug(Some("alice"), "builder")
        .unwrap()
        .unwrap();
    assert_eq!(local_loaded.id, local.id);
    assert_eq!(alice_loaded.id, alice.id);
    assert_ne!(local_loaded.id, alice_loaded.id);

    assert_eq!(store.list_for_owner(None, false).unwrap().len(), 1);
    assert_eq!(store.list_for_owner(Some("alice"), false).unwrap().len(), 1);
    assert!(store.list_for_owner(Some("bob"), false).unwrap().is_empty());
    assert_eq!(store.get(&local.id).unwrap().unwrap().slug, "builder");
}

#[test]
fn always_on_create_defaults_heartbeat_interval() {
    let (store, _temp) = store();
    let worker = store
        .create(&NewHiveWorker {
            autonomy: HiveWorkerAutonomy::AlwaysOn,
            ..NewHiveWorker::new("pulse")
        })
        .expect("create always-on worker");
    assert_eq!(worker.autonomy, HiveWorkerAutonomy::AlwaysOn);
    assert_eq!(
        worker.heartbeat_interval_secs,
        Some(crate::storage::DEFAULT_WORKER_HEARTBEAT_INTERVAL_SECS)
    );
}

#[test]
fn worker_slug_validation_rejects_invalid_slugs() {
    let (store, _temp) = store();
    for slug in ["", "Bad Slug", "UPPER", "slash/slug", &"x".repeat(65)] {
        assert!(
            store.create(&NewHiveWorker::new(slug)).is_err(),
            "slug {slug:?} must be rejected"
        );
    }
    assert_eq!(
        store
            .conn()
            .query_row("SELECT COUNT(*) FROM hive_workers", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn worker_active_slug_is_unique_until_archived() {
    let (store, _temp) = store();
    let first = store.create(&NewHiveWorker::new("builder")).unwrap();
    assert!(
        store.create(&NewHiveWorker::new("builder")).is_err(),
        "duplicate active slug for one owner must be rejected"
    );

    assert!(store
        .set_status(&first.id, HiveWorkerStatus::Archived)
        .unwrap());
    let replacement = store
        .create(&NewHiveWorker::new("builder"))
        .expect("archived slug is reusable");
    assert_ne!(replacement.id, first.id);
    assert_eq!(
        store.get_by_slug(None, "builder").unwrap().unwrap().id,
        replacement.id
    );
    // The archived worker remains visible only when requested.
    assert_eq!(store.list_for_owner(None, false).unwrap().len(), 1);
    assert_eq!(store.list_for_owner(None, true).unwrap().len(), 2);
}

#[test]
fn worker_profile_updates_and_status_transitions_persist() {
    let (store, _temp) = store();
    let worker = store.create(&NewHiveWorker::new("researcher")).unwrap();

    let updated = store
        .update_profile(
            &worker.id,
            &HiveWorkerProfileUpdate {
                display_name: "Deep Researcher".into(),
                avatar_color: Some("#7743DB".into()),
                model: Some("grok-code-fast-1".into()),
                model_key: Some(ModelKey::new(
                    ProviderId::Grok,
                    "grok-code-fast-1",
                    ApiFormat::OpenAIResponses,
                )),
                model_catalog_revision: Some("catalog-42".into()),
                permission_mode: PermissionMode::Supervised,
            },
        )
        .unwrap()
        .expect("worker exists");
    assert_eq!(updated.display_name, "Deep Researcher");
    assert_eq!(updated.avatar_color.as_deref(), Some("#7743DB"));
    assert_eq!(updated.permission_mode, PermissionMode::Supervised);

    // A model key that disagrees with the plain model slug is rejected.
    assert!(store
        .update_profile(
            &worker.id,
            &HiveWorkerProfileUpdate {
                display_name: "Deep Researcher".into(),
                avatar_color: None,
                model: Some("other-model".into()),
                model_key: Some(ModelKey::new(
                    ProviderId::Grok,
                    "grok-code-fast-1",
                    ApiFormat::OpenAIResponses,
                )),
                model_catalog_revision: None,
                permission_mode: PermissionMode::Supervised,
            },
        )
        .is_err());

    assert!(store
        .set_autonomy(&worker.id, HiveWorkerAutonomy::AlwaysOn, None)
        .unwrap());
    assert_eq!(
        store
            .get(&worker.id)
            .unwrap()
            .unwrap()
            .heartbeat_interval_secs,
        Some(crate::storage::DEFAULT_WORKER_HEARTBEAT_INTERVAL_SECS)
    );
    assert!(store
        .set_autonomy(&worker.id, HiveWorkerAutonomy::AlwaysOn, Some(900))
        .unwrap());
    assert!(store
        .set_autonomy(&worker.id, HiveWorkerAutonomy::AlwaysOn, Some(0))
        .is_err());
    assert!(store
        .set_status(&worker.id, HiveWorkerStatus::Paused)
        .unwrap());
    let reloaded = store.get(&worker.id).unwrap().unwrap();
    assert_eq!(reloaded.autonomy, HiveWorkerAutonomy::AlwaysOn);
    assert_eq!(reloaded.heartbeat_interval_secs, Some(900));
    assert_eq!(reloaded.status, HiveWorkerStatus::Paused);
    assert!(store
        .update_profile(
            "missing-worker",
            &HiveWorkerProfileUpdate {
                display_name: "Ghost".into(),
                avatar_color: None,
                model: None,
                model_key: None,
                model_catalog_revision: None,
                permission_mode: PermissionMode::Autonomous,
            },
        )
        .unwrap()
        .is_none());
}

#[test]
fn worker_dm_session_binding_is_exclusive() {
    let (store, _temp) = store();
    let first = store.create(&NewHiveWorker::new("builder")).unwrap();
    let second = store.create(&NewHiveWorker::new("reviewer")).unwrap();

    assert!(store.bind_dm_session(&first.id, Some("dm-1")).unwrap());
    assert_eq!(
        store
            .get(&first.id)
            .unwrap()
            .unwrap()
            .dm_session_id
            .as_deref(),
        Some("dm-1")
    );
    // One session can be the DM lane of at most one worker.
    assert!(store.bind_dm_session(&second.id, Some("dm-1")).is_err());
    assert!(store.bind_dm_session(&second.id, Some("dm-2")).unwrap());
    // Unknown sessions are rejected by the foreign key.
    assert!(store.bind_dm_session(&second.id, Some("missing")).is_err());
    // Rebinding to nothing frees the session.
    assert!(store.bind_dm_session(&first.id, None).unwrap());
    assert!(store.bind_dm_session(&second.id, Some("dm-1")).unwrap());
}

#[test]
fn worker_resolves_by_dm_session_binding() {
    let (store, _temp) = store();
    let builder = store.create(&NewHiveWorker::new("builder")).unwrap();
    let reviewer = store.create(&NewHiveWorker::new("reviewer")).unwrap();
    assert!(store.bind_dm_session(&builder.id, Some("dm-1")).unwrap());
    assert!(store.bind_dm_session(&reviewer.id, Some("dm-2")).unwrap());

    assert_eq!(
        store.get_by_dm_session("dm-1").unwrap().unwrap().id,
        builder.id
    );
    assert_eq!(
        store.get_by_dm_session("dm-2").unwrap().unwrap().id,
        reviewer.id
    );
    assert!(store.get_by_dm_session("missing").unwrap().is_none());

    // Clearing the binding stops resolution for that session.
    assert!(store.bind_dm_session(&builder.id, None).unwrap());
    assert!(store.get_by_dm_session("dm-1").unwrap().is_none());
}

#[test]
fn worker_documents_round_trip() {
    let (store, _temp) = store();
    let worker = store.create(&NewHiveWorker::new("builder")).unwrap();

    store
        .upsert_document(&worker.id, HiveWorkerDocumentKind::Identity, "You build.")
        .unwrap();
    store
        .upsert_document(&worker.id, HiveWorkerDocumentKind::Soul, "Calm and exact.")
        .unwrap();
    store
        .upsert_document(
            &worker.id,
            HiveWorkerDocumentKind::Identity,
            "You build v2.",
        )
        .unwrap();

    let identity = store
        .document(&worker.id, HiveWorkerDocumentKind::Identity)
        .unwrap()
        .unwrap();
    assert_eq!(identity.content, "You build v2.");
    assert_eq!(store.documents(&worker.id).unwrap().len(), 2);

    assert!(store
        .upsert_document(&worker.id, HiveWorkerDocumentKind::Soul, "   ")
        .is_err());
    assert!(store
        .upsert_document("missing-worker", HiveWorkerDocumentKind::Soul, "text")
        .is_err());
}

#[test]
fn resolve_worker_for_crew_slug_prefers_slug_then_namespace() {
    let (store, _temp) = store();
    let builder = store.create(&NewHiveWorker::new("builder")).unwrap();
    // A renamed worker that kept its crew-compatible memory namespace.
    let renamed = store
        .create(&NewHiveWorker {
            memory_namespace_id: Some("reviewer".into()),
            ..NewHiveWorker::new("quality-lead")
        })
        .unwrap();

    assert_eq!(
        store
            .resolve_worker_for_crew_slug(None, "builder")
            .unwrap()
            .unwrap()
            .id,
        builder.id
    );
    assert_eq!(
        store
            .resolve_worker_for_crew_slug(None, "reviewer")
            .unwrap()
            .unwrap()
            .id,
        renamed.id
    );
    assert!(store
        .resolve_worker_for_crew_slug(None, "unknown")
        .unwrap()
        .is_none());
    // Exact-owner scoping: alice has no workers yet.
    assert!(store
        .resolve_worker_for_crew_slug(Some("alice"), "builder")
        .unwrap()
        .is_none());
    // Archived workers no longer resolve.
    assert!(store
        .set_status(&builder.id, HiveWorkerStatus::Archived)
        .unwrap());
    assert!(store
        .resolve_worker_for_crew_slug(None, "builder")
        .unwrap()
        .is_none());
}

#[test]
fn display_name_from_slug_titles_words() {
    assert_eq!(display_name_from_slug("builder"), "Builder");
    assert_eq!(display_name_from_slug("code-reviewer"), "Code Reviewer");
    assert_eq!(display_name_from_slug("deep_research_2"), "Deep Research 2");
}
