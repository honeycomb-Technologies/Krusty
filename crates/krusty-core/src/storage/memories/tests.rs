use tempfile::TempDir;

use super::{
    CanonicalMemoryInput, MemoryNamespace, MemoryRevisionEvent, MemorySensitivity, MemorySource,
    MemoryStatus, MemoryStore, MemoryType,
};
use crate::storage::Database;

fn create_store() -> (MemoryStore, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("memories.db");
    let db = Database::new(&db_path).expect("database");
    (MemoryStore::new(db), temp_dir)
}

#[test]
fn save_and_list_memories() {
    let (store, _tmp) = create_store();
    store
        .save(MemoryType::User, "Role", "Backend developer", None, None)
        .unwrap();
    store
        .save(
            MemoryType::Feedback,
            "Testing",
            "Use integration tests",
            None,
            None,
        )
        .unwrap();

    let all = store.list(None, None);
    assert_eq!(all.len(), 2);

    let users = store.list_by_type(MemoryType::User, None, None);
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].title, "Role");
    assert_eq!(users[0].namespace, MemoryNamespace::Shared);
    assert_eq!(users[0].status, MemoryStatus::Active);
    assert_eq!(users[0].source, MemorySource::Legacy);
    assert_eq!(users[0].confidence, 1.0);
    assert_eq!(users[0].access_count, 0);
}

#[test]
fn list_without_user_id_excludes_user_scoped_memories() {
    let (store, _tmp) = create_store();
    store
        .save(MemoryType::User, "Global", "shared", None, None)
        .unwrap();
    store
        .save(MemoryType::User, "Alice", "alice-only", None, Some("alice"))
        .unwrap();

    let unscoped = store.list(None, None);
    assert_eq!(unscoped.len(), 1);
    assert_eq!(unscoped[0].title, "Global");

    let alice = store.list(None, Some("alice"));
    assert_eq!(alice.len(), 2);
    assert!(alice.iter().any(|memory| memory.title == "Alice"));
    assert!(alice.iter().any(|memory| memory.title == "Global"));
}

#[test]
fn exact_owner_listing_never_inherits_local_or_foreign_memory() {
    let (store, _tmp) = create_store();
    for (title, content, project_dir, user_id) in [
        ("Local global", "local-global", None, None),
        ("Local project", "local-project", Some("/repo"), None),
        ("Alice global", "alice-global", None, Some("alice")),
        (
            "Alice project",
            "alice-project",
            Some("/repo"),
            Some("alice"),
        ),
        ("Bob project", "bob-project", Some("/repo"), Some("bob")),
    ] {
        store
            .save(MemoryType::Project, title, content, project_dir, user_id)
            .unwrap();
    }

    let alice = store.list_for_exact_owner(Some("/repo"), Some("alice"));
    assert_eq!(alice.len(), 2);
    assert!(alice
        .iter()
        .all(|memory| memory.user_id.as_deref() == Some("alice")));
    assert!(alice.iter().any(|memory| memory.content == "alice-global"));
    assert!(alice.iter().any(|memory| memory.content == "alice-project"));
    assert!(alice
        .iter()
        .all(|memory| !memory.content.starts_with("local-")));
    assert!(alice
        .iter()
        .all(|memory| !memory.content.starts_with("bob-")));

    let local = store.list_for_exact_owner(Some("/repo"), None);
    assert_eq!(local.len(), 2);
    assert!(local.iter().all(|memory| memory.user_id.is_none()));
    assert!(local.iter().any(|memory| memory.content == "local-global"));
    assert!(local.iter().any(|memory| memory.content == "local-project"));
    assert!(local
        .iter()
        .all(|memory| !memory.content.starts_with("alice-")));
    assert!(local
        .iter()
        .all(|memory| !memory.content.starts_with("bob-")));
}

#[test]
fn update_memory() {
    let (store, _tmp) = create_store();
    let mem = store
        .save(MemoryType::Project, "Sprint", "Sprint 4", None, None)
        .unwrap();
    store
        .update(&mem.id, Some("Sprint goal"), Some("Ship auth"))
        .unwrap();

    let updated = store.get(&mem.id).unwrap().unwrap();
    assert_eq!(updated.title, "Sprint goal");
    assert_eq!(updated.content, "Ship auth");
}

#[test]
fn delete_memory() {
    let (store, _tmp) = create_store();
    let mem = store
        .save(
            MemoryType::Reference,
            "Tracker",
            "Linear INGEST",
            None,
            None,
        )
        .unwrap();
    assert!(store.get(&mem.id).unwrap().is_some());
    store.delete(&mem.id).unwrap();
    assert!(store.get(&mem.id).unwrap().is_none());
}

#[test]
fn project_scoped_memories() {
    let (store, _tmp) = create_store();
    store
        .save(
            MemoryType::Project,
            "Global",
            "applies everywhere",
            None,
            None,
        )
        .unwrap();
    store
        .save(
            MemoryType::Project,
            "Scoped",
            "only in project-a",
            Some("/home/user/project-a"),
            None,
        )
        .unwrap();

    // Listing with project_dir returns both scoped + global
    let project_a = store.list(Some("/home/user/project-a"), None);
    assert_eq!(project_a.len(), 2);

    // Listing without project_dir returns only global
    let global = store.list(None, None);
    assert_eq!(global.len(), 1);
    assert_eq!(global[0].title, "Global");
}

#[test]
fn find_by_title() {
    let (store, _tmp) = create_store();
    store
        .save(MemoryType::User, "Preferred language", "Rust", None, None)
        .unwrap();
    let found = store.find_by_title("Preferred language", None);
    assert!(found.is_some());
    assert_eq!(found.unwrap().content, "Rust");

    let missing = store.find_by_title("Nonexistent", None);
    assert!(missing.is_none());
}

#[test]
fn find_by_title_for_user_respects_scope_and_owner() {
    let (store, _tmp) = create_store();
    store
        .save(
            MemoryType::Project,
            "Deployment",
            "Blue/green",
            Some("/proj-a"),
            Some("alice"),
        )
        .unwrap();
    store
        .save(
            MemoryType::Project,
            "Deployment",
            "Canary",
            Some("/proj-a"),
            Some("bob"),
        )
        .unwrap();

    let alice = store.find_by_title_for_user("Deployment", Some("/proj-a"), Some("alice"));
    let bob = store.find_by_title_for_user("Deployment", Some("/proj-a"), Some("bob"));

    assert_eq!(alice.unwrap().content, "Blue/green");
    assert_eq!(bob.unwrap().content, "Canary");
    assert!(store
        .find_by_title_for_user("Deployment", Some("/proj-a"), None)
        .is_none());
}

#[test]
fn save_or_update_by_title_updates_existing_memory() {
    let (store, _tmp) = create_store();
    let (created, created_new) = store
        .save_or_update_by_title(
            MemoryType::Project,
            "Architecture",
            "Use typed boundaries",
            Some("/proj-a"),
            Some("alice"),
        )
        .unwrap();
    let (updated, updated_new) = store
        .save_or_update_by_title(
            MemoryType::Project,
            "Architecture",
            "Use typed boundaries and explicit contracts",
            Some("/proj-a"),
            Some("alice"),
        )
        .unwrap();

    assert!(created_new);
    assert!(!updated_new);
    assert_eq!(created.id, updated.id);
    assert_eq!(
        updated.content,
        "Use typed boundaries and explicit contracts"
    );
}

#[test]
fn save_or_update_by_title_does_not_overwrite_global_memory_for_project_scope() {
    let (store, _tmp) = create_store();
    let global = store
        .save_or_update_by_title(
            MemoryType::Project,
            "Architecture",
            "Global guidance",
            None,
            Some("alice"),
        )
        .unwrap()
        .0;
    let scoped = store
        .save_or_update_by_title(
            MemoryType::Project,
            "Architecture",
            "Project-specific guidance",
            Some("/proj-a"),
            Some("alice"),
        )
        .unwrap()
        .0;

    assert_ne!(global.id, scoped.id);
    assert_eq!(store.list(None, Some("alice")).len(), 1);
    assert_eq!(store.list(Some("/proj-a"), Some("alice")).len(), 2);
}

#[test]
fn canonical_memory_preserves_scope_provenance_and_policy_metadata() {
    let (store, _tmp) = create_store();
    let mut input = CanonicalMemoryInput::new(
        MemoryType::Feedback,
        "communication.concise",
        "Communication preference",
        "Be concise during operational work.",
    );
    input.project_dir = Some("/repo".to_string());
    input.user_id = Some("alice".to_string());
    input.namespace = MemoryNamespace::Mako;
    input.namespace_id = Some("primary".to_string());
    input.source = MemorySource::User;
    input.source_session_id = Some("session-1".to_string());
    input.source_message_id = Some("message-7".to_string());
    input.confidence = 0.95;
    input.sensitivity = MemorySensitivity::Sensitive;
    input.pinned = true;

    let memory = store.save_canonical(&input).unwrap();

    assert_eq!(
        memory.canonical_key.as_deref(),
        Some("communication.concise")
    );
    assert_eq!(memory.project_dir.as_deref(), Some("/repo"));
    assert_eq!(memory.user_id.as_deref(), Some("alice"));
    assert_eq!(memory.namespace, MemoryNamespace::Mako);
    assert_eq!(memory.namespace_id.as_deref(), Some("primary"));
    assert_eq!(memory.source, MemorySource::User);
    assert_eq!(memory.source_session_id.as_deref(), Some("session-1"));
    assert_eq!(memory.source_message_id.as_deref(), Some("message-7"));
    assert_eq!(memory.confidence, 0.95);
    assert_eq!(memory.sensitivity, MemorySensitivity::Sensitive);
    assert!(memory.pinned);
    assert_eq!(memory.status, MemoryStatus::Active);

    let revisions = store
        .list_revisions_for_owner(&memory.id, Some("alice"))
        .unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0].event, MemoryRevisionEvent::Created);
    assert_eq!(revisions[0].snapshot, memory);
}

#[test]
fn canonical_save_supersedes_the_same_active_key_transactionally() {
    let (store, _tmp) = create_store();
    let mut first_input = CanonicalMemoryInput::new(
        MemoryType::Project,
        "architecture.auth",
        "Auth boundary",
        "Use the original boundary.",
    );
    first_input.project_dir = Some("/repo".to_string());
    first_input.user_id = Some("alice".to_string());
    first_input.source_session_id = Some("session-1".to_string());
    let first = store.save_canonical(&first_input).unwrap();

    let mut replacement_input = first_input;
    replacement_input.content = "Use the revised boundary.".to_string();
    replacement_input.source_session_id = Some("session-2".to_string());
    let replacement = store.save_canonical(&replacement_input).unwrap();

    assert_ne!(first.id, replacement.id);
    assert_eq!(
        replacement.supersedes_id.as_deref(),
        Some(first.id.as_str())
    );
    assert!(store.get(&first.id).unwrap().is_none());
    assert_eq!(
        store.get(&replacement.id).unwrap(),
        Some(replacement.clone())
    );

    let visible = store.list(Some("/repo"), Some("alice"));
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].content, "Use the revised boundary.");

    let first_revisions = store
        .list_revisions_for_owner(&first.id, Some("alice"))
        .unwrap();
    assert_eq!(first_revisions.len(), 2);
    assert_eq!(first_revisions[0].event, MemoryRevisionEvent::Created);
    assert_eq!(first_revisions[1].event, MemoryRevisionEvent::Superseded);
    assert_eq!(first_revisions[1].snapshot.status, MemoryStatus::Superseded);
    let replacement_revisions = store
        .list_revisions_for_owner(&replacement.id, Some("alice"))
        .unwrap();
    assert_eq!(replacement_revisions.len(), 1);
    assert_eq!(replacement_revisions[0].event, MemoryRevisionEvent::Created);
}

#[test]
fn exact_canonical_replay_is_idempotent() {
    let (store, _tmp) = create_store();
    let input =
        CanonicalMemoryInput::new(MemoryType::User, "operator.name", "Preferred name", "Alice");

    let first = store.save_canonical(&input).unwrap();
    let replay = store.save_canonical(&input).unwrap();

    assert_eq!(first.id, replay.id);
    assert_eq!(store.list(None, None).len(), 1);
    assert_eq!(
        store
            .list_revisions_for_owner(&first.id, None)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn canonical_keys_are_isolated_by_owner_and_crew_namespace() {
    let (store, _tmp) = create_store();
    let mut alice = CanonicalMemoryInput::new(
        MemoryType::Feedback,
        "review.style",
        "Review style",
        "Alice prefers evidence first.",
    );
    alice.user_id = Some("alice".to_string());
    alice.namespace = MemoryNamespace::Crew;
    alice.namespace_id = Some("reviewer".to_string());
    let mut builder = alice.clone();
    builder.namespace_id = Some("builder".to_string());
    builder.content = "Builder should be implementation first.".to_string();
    let mut bob = alice.clone();
    bob.user_id = Some("bob".to_string());
    bob.content = "Bob prefers compact reviews.".to_string();

    let alice_reviewer = store.save_canonical(&alice).unwrap();
    let alice_builder = store.save_canonical(&builder).unwrap();
    let bob_reviewer = store.save_canonical(&bob).unwrap();

    assert_ne!(alice_reviewer.id, alice_builder.id);
    assert_ne!(alice_reviewer.id, bob_reviewer.id);
    assert_eq!(store.list(None, Some("alice")).len(), 2);
    assert_eq!(store.list(None, Some("bob")).len(), 1);
}

#[test]
fn owner_scoped_mutations_do_not_reveal_or_modify_other_users() {
    let (store, _tmp) = create_store();
    let memory = store
        .save(
            MemoryType::User,
            "Preferred language",
            "Rust",
            None,
            Some("alice"),
        )
        .unwrap();

    assert!(store
        .get_for_owner(&memory.id, Some("bob"))
        .unwrap()
        .is_none());
    assert!(store
        .update_for_owner(
            &memory.id,
            Some("bob"),
            Some("Wrong owner"),
            Some("TypeScript")
        )
        .unwrap()
        .is_none());
    assert!(!store.delete_for_owner(&memory.id, Some("bob")).unwrap());

    let updated = store
        .update_for_owner(
            &memory.id,
            Some("alice"),
            Some("Preferred systems language"),
            Some("Rust"),
        )
        .unwrap()
        .unwrap();
    assert_eq!(updated.title, "Preferred systems language");

    let accessed = store
        .record_access_for_owner(&memory.id, Some("alice"))
        .unwrap()
        .unwrap();
    assert_eq!(accessed.access_count, 1);
    assert!(accessed.last_accessed_at.is_some());

    assert!(store.delete_for_owner(&memory.id, Some("alice")).unwrap());
    assert!(store.get(&memory.id).unwrap().is_none());
    assert!(store.list(None, Some("alice")).is_empty());

    let revisions = store
        .list_revisions_for_owner(&memory.id, Some("alice"))
        .unwrap();
    assert_eq!(
        revisions
            .iter()
            .map(|revision| revision.event)
            .collect::<Vec<_>>(),
        vec![
            MemoryRevisionEvent::Created,
            MemoryRevisionEvent::Updated,
            MemoryRevisionEvent::Deleted,
        ]
    );
    assert_eq!(
        revisions.last().unwrap().snapshot.status,
        MemoryStatus::Deleted
    );
}

#[test]
fn canonical_tombstone_is_exactly_scoped_revisioned_and_idempotent() {
    let (store, _tmp) = create_store();
    let mut alice_repo = CanonicalMemoryInput::new(
        MemoryType::Feedback,
        "communication.progress",
        "Progress style",
        "Use concise progress updates.",
    );
    alice_repo.user_id = Some("alice".to_string());
    alice_repo.project_dir = Some("/repo-a".to_string());
    let mut alice_other_repo = alice_repo.clone();
    alice_other_repo.project_dir = Some("/repo-b".to_string());
    let mut bob_repo = alice_repo.clone();
    bob_repo.user_id = Some("bob".to_string());

    let target = store.save_canonical(&alice_repo).unwrap();
    let other_repo = store.save_canonical(&alice_other_repo).unwrap();
    let other_owner = store.save_canonical(&bob_repo).unwrap();

    assert!(store
        .tombstone_canonical_for_owner(
            "communication.progress",
            Some("/repo-a"),
            Some("bob"),
            MemoryNamespace::Shared,
            None,
        )
        .unwrap()
        .is_some());
    assert!(store.get(&other_owner.id).unwrap().is_none());
    assert!(store.get(&target.id).unwrap().is_some());
    assert!(store.get(&other_repo.id).unwrap().is_some());

    let deleted = store
        .tombstone_canonical_for_owner(
            "communication.progress",
            Some("/repo-a"),
            Some("alice"),
            MemoryNamespace::Shared,
            None,
        )
        .unwrap()
        .unwrap();
    assert_eq!(deleted.id, target.id);
    assert_eq!(deleted.status, MemoryStatus::Deleted);
    assert!(store.get(&target.id).unwrap().is_none());
    assert!(store.get(&other_repo.id).unwrap().is_some());
    assert!(store
        .tombstone_canonical_for_owner(
            "communication.progress",
            Some("/repo-a"),
            Some("alice"),
            MemoryNamespace::Shared,
            None,
        )
        .unwrap()
        .is_none());

    let revisions = store
        .list_revisions_for_owner(&target.id, Some("alice"))
        .unwrap();
    assert_eq!(
        revisions
            .iter()
            .map(|revision| revision.event)
            .collect::<Vec<_>>(),
        vec![MemoryRevisionEvent::Created, MemoryRevisionEvent::Deleted]
    );
}

#[test]
fn canonical_validation_rejects_ambiguous_crew_and_confidence() {
    let (store, _tmp) = create_store();
    let mut crew = CanonicalMemoryInput::new(
        MemoryType::Project,
        "crew.focus",
        "Crew focus",
        "Review persistence.",
    );
    crew.namespace = MemoryNamespace::Crew;
    assert!(store.save_canonical(&crew).is_err());

    crew.namespace_id = Some("reviewer".to_string());
    crew.confidence = 1.1;
    assert!(store.save_canonical(&crew).is_err());
    assert!(store.list(None, None).is_empty());
}
