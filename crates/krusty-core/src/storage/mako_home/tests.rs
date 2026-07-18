use super::{
    bootstrap_mako_home, is_valid_crew_slug, summarize_channel_bindings, summarize_crew_runtime,
    write_mako_crew_document, write_mako_home_document, MakoChannelKind, MakoCrewDocumentKind,
    MakoCrewRuntimeStatus, MakoHomeDocumentKind, MakoHomeProfile,
};
use crate::agent::DelegatedRunStage;
use crate::paths;
use crate::storage::{
    AutonomousTaskStore, Database, DelegatedRunAgentSnapshot, DelegatedRunRole, DelegatedRunScope,
    DelegatedRunSnapshot, DelegatedRunStartInput, DelegatedRunStore, SessionManager, SessionType,
    WorkspaceMode,
};
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

#[test]
fn load_mako_home_prefers_branded_top_level_files() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join(paths::MAKO_SOUL_FILE), "Soul.").unwrap();
    fs::write(temp.path().join(paths::MAKO_IDENTITY_FILE), "Identity.").unwrap();
    fs::write(temp.path().join(paths::MAKO_USER_FILE), "User.").unwrap();

    let profile = MakoHomeProfile::load_from(temp.path());

    assert_eq!(profile.soul.unwrap().file_name, paths::MAKO_SOUL_FILE);
    assert_eq!(
        profile.identity.unwrap().file_name,
        paths::MAKO_IDENTITY_FILE
    );
    assert_eq!(profile.user.unwrap().file_name, paths::MAKO_USER_FILE);
}

#[test]
fn load_mako_home_falls_back_to_legacy_generic_files() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join("SOUL.md"), "Soul.").unwrap();
    fs::write(temp.path().join("USER.md"), "User.").unwrap();
    fs::write(temp.path().join("CHANNELS.md"), "Channels.").unwrap();

    let profile = MakoHomeProfile::load_from(temp.path());

    assert_eq!(profile.soul.unwrap().file_name, "SOUL.md");
    assert_eq!(profile.user.unwrap().file_name, "USER.md");
    assert_eq!(profile.channels.unwrap().file_name, "CHANNELS.md");
}

#[test]
fn load_mako_home_discovers_sorted_crew_profiles() {
    let temp = TempDir::new().unwrap();
    let reviewer = temp.path().join("crew").join("reviewer");
    let builder = temp.path().join("crew").join("builder");
    fs::create_dir_all(&reviewer).unwrap();
    fs::create_dir_all(&builder).unwrap();
    fs::write(reviewer.join("IDENTITY.md"), "Reviewer").unwrap();
    fs::write(builder.join("SOUL.md"), "Builder soul").unwrap();

    let profile = MakoHomeProfile::load_from(temp.path());
    let slugs = profile
        .crew
        .iter()
        .map(|member| member.slug.clone())
        .collect::<Vec<_>>();

    assert_eq!(slugs, vec!["builder".to_string(), "reviewer".to_string()]);
}

#[test]
fn bootstrap_mako_home_creates_branded_files_and_default_crew() {
    let temp = TempDir::new().unwrap();

    let result = bootstrap_mako_home(temp.path()).unwrap();

    assert!(result
        .created_files
        .iter()
        .any(|path| path == paths::MAKO_SOUL_FILE));
    assert!(result
        .created_files
        .iter()
        .any(|path| path == paths::MAKO_USER_FILE));
    assert!(result
        .created_files
        .iter()
        .any(|path| path == "crew/builder/IDENTITY.md"));
    assert_eq!(result.profile.crew.len(), 3);
}

#[test]
fn active_legacy_context_layers_include_user_but_exclude_memory() {
    let temp = TempDir::new().unwrap();
    fs::write(temp.path().join(paths::MAKO_SOUL_FILE), "Soul.").unwrap();
    fs::write(temp.path().join(paths::MAKO_IDENTITY_FILE), "Identity.").unwrap();
    fs::write(temp.path().join(paths::MAKO_USER_FILE), "User.").unwrap();
    fs::write(temp.path().join(paths::MAKO_MEMORY_FILE), "Legacy memory.").unwrap();

    let profile = MakoHomeProfile::load_from(temp.path());
    let kinds = profile
        .context_layers()
        .into_iter()
        .map(|layer| layer.kind)
        .collect::<Vec<_>>();

    assert_eq!(kinds, vec!["SOUL", "IDENTITY", "USER"]);
    assert!(profile.memory.is_some(), "legacy memory remains importable");
}

#[test]
fn write_document_helpers_use_preferred_file_names() {
    let temp = TempDir::new().unwrap();

    let home_doc =
        write_mako_home_document(temp.path(), MakoHomeDocumentKind::Identity, "Mako").unwrap();
    let crew_doc = write_mako_crew_document(
        temp.path(),
        "researcher",
        MakoCrewDocumentKind::Soul,
        "Read widely",
    )
    .unwrap();

    assert_eq!(home_doc.file_name, paths::MAKO_IDENTITY_FILE);
    assert_eq!(crew_doc.file_name, "SOUL.md");
    assert!(temp.path().join(paths::MAKO_IDENTITY_FILE).is_file());
    assert!(temp
        .path()
        .join("crew")
        .join("researcher")
        .join("SOUL.md")
        .is_file());
}

#[test]
fn crew_slug_validation_is_conservative() {
    assert!(is_valid_crew_slug("reviewer"));
    assert!(is_valid_crew_slug("ops_1"));
    assert!(!is_valid_crew_slug("Reviewer"));
    assert!(!is_valid_crew_slug("../evil"));
    assert!(!is_valid_crew_slug(""));
}

#[test]
fn summarize_crew_runtime_merges_profile_tasks_and_delegated_runs() {
    let temp = TempDir::new().unwrap();
    bootstrap_mako_home(temp.path()).unwrap();
    let db_path = temp.path().join("krusty.db");
    let db = Database::new(&db_path).unwrap();
    db.conn()
        .execute(
            "INSERT INTO users (id, email, license_tier) VALUES (?1, ?2, ?3)",
            ("alice", "alice@example.com", "free"),
        )
        .unwrap();
    let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
    let session_id = session_manager
        .create_session_for_user_with_config(
            "Mako",
            None,
            Some(temp.path().to_string_lossy().as_ref()),
            Some(temp.path().to_string_lossy().as_ref()),
            WorkspaceMode::Selected,
            Some("alice"),
            None,
            SessionType::Mako,
        )
        .unwrap();

    let task_store = AutonomousTaskStore::new(Database::new(&db_path).unwrap());
    let delegated_store = DelegatedRunStore::new(Database::new(&db_path).unwrap());
    let task_id = task_store
        .create_task(&session_id, "Build", "Implement feature", &[])
        .unwrap();
    task_store.claim_task(&task_id, "builder").unwrap();

    delegated_store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: "run-1".to_string(),
            parent_session_id: session_id.clone(),
            parent_tool_call_id: None,
            role: DelegatedRunRole::Build,
            stage: DelegatedRunStage::Running,
            provider: None,
            model: None,
            resumable: false,
            resumed_from_run_id: None,
            target_scope: vec![DelegatedRunScope {
                label: "repo".to_string(),
                path: temp.path().to_string_lossy().to_string(),
                kind: "dir".to_string(),
            }],
        })
        .unwrap();
    delegated_store
        .update_snapshot(
            "run-1",
            DelegatedRunStage::Running,
            &DelegatedRunSnapshot {
                stage: DelegatedRunStage::Running,
                agents: vec![DelegatedRunAgentSnapshot {
                    task_id: "task-1".to_string(),
                    agent_name: "builder".to_string(),
                    status: "running".to_string(),
                    tool_count: 1,
                    tokens: 10,
                    current_action: None,
                    completion_summary: None,
                    lines_added: 0,
                    lines_removed: 0,
                    completed_plan_task: None,
                }],
            },
        )
        .unwrap();
    delegated_store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: "run-2".to_string(),
            parent_session_id: session_id.clone(),
            parent_tool_call_id: None,
            role: DelegatedRunRole::Verifier,
            stage: DelegatedRunStage::Failed,
            provider: None,
            model: None,
            resumable: false,
            resumed_from_run_id: None,
            target_scope: vec![DelegatedRunScope {
                label: "repo".to_string(),
                path: temp.path().to_string_lossy().to_string(),
                kind: "dir".to_string(),
            }],
        })
        .unwrap();
    delegated_store
        .update_snapshot(
            "run-2",
            DelegatedRunStage::Failed,
            &DelegatedRunSnapshot {
                stage: DelegatedRunStage::Failed,
                agents: vec![DelegatedRunAgentSnapshot {
                    task_id: "task-2".to_string(),
                    agent_name: "reviewer".to_string(),
                    status: "failed".to_string(),
                    tool_count: 1,
                    tokens: 10,
                    current_action: None,
                    completion_summary: None,
                    lines_added: 0,
                    lines_removed: 0,
                    completed_plan_task: None,
                }],
            },
        )
        .unwrap();

    let profile = MakoHomeProfile::load_from(temp.path());
    let sessions = vec![session_manager.get_session(&session_id).unwrap().unwrap()];
    let summary = summarize_crew_runtime(
        &profile,
        &sessions,
        &HashMap::new(),
        &task_store,
        &delegated_store,
    )
    .unwrap();

    let builder = summary
        .iter()
        .find(|member| member.slug == "builder")
        .unwrap();
    assert_eq!(builder.status, MakoCrewRuntimeStatus::Running);
    assert_eq!(builder.active_task_count, 1);
    assert_eq!(builder.active_run_count, 1);

    let reviewer = summary
        .iter()
        .find(|member| member.slug == "reviewer")
        .unwrap();
    assert_eq!(reviewer.status, MakoCrewRuntimeStatus::Degraded);
    assert_eq!(reviewer.failed_run_count, 1);
    assert!(summary.iter().any(|member| member.slug == "researcher"));
}

#[test]
fn summarize_channel_bindings_combines_system_and_home_entries() {
    let temp = TempDir::new().unwrap();
    bootstrap_mako_home(temp.path()).unwrap();
    fs::write(
            temp.path().join(paths::MAKO_CHANNELS_FILE),
            "# Mako Channels\n- [x] iPhone push: urgent approvals and wake alerts\n- [ ] email: weekly digest only",
        )
        .unwrap();

    let profile = MakoHomeProfile::load_from(temp.path());
    let bindings = summarize_channel_bindings(&profile);

    assert!(bindings.iter().any(|binding| binding.id == "main-thread"));
    let push = bindings
        .iter()
        .find(|binding| binding.id == "iphone-push")
        .expect("push binding should exist");
    assert_eq!(push.kind, MakoChannelKind::MobilePush);
    assert!(push.enabled);

    let email = bindings
        .iter()
        .find(|binding| binding.id == "email")
        .expect("email binding should exist");
    assert!(!email.enabled);
}
