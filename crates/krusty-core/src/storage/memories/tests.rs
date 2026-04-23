use tempfile::TempDir;

use super::{MemoryStore, MemoryType};
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
