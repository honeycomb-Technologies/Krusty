use std::fs;
use std::io::Write;

use tempfile::TempDir;
use tokio::sync::RwLock;

use super::mako::{build_mako_context_sections, build_mako_context_sections_with_home};
use super::reports::build_mako_knowledge_context;
use super::workspace::{build_environment_context, summarize_git_status};
use super::{
    bound_dynamic_context_messages, build_plan_context, build_project_context,
    build_skills_context, inject_context, MAX_DYNAMIC_CONTEXT_BYTES,
};

use crate::agent::DelegatedRunStage;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::paths;
use crate::plan::PlanManager;
use crate::skills::SkillsManager;
use crate::storage::reports::CreateReportInput;
use crate::storage::{
    AutonomousTaskStore, CanonicalMemoryInput, Database, DelegatedRunRole, DelegatedRunScope,
    DelegatedRunStartInput, DelegatedRunStore, MemoryNamespace, MemoryStore, MemoryType,
    ReportStore, SessionManager, WorkMode,
};

#[test]
fn project_context_loads_hierarchical_instruction_files() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    let nested = repo.join("a").join("b");
    fs::create_dir_all(&nested).unwrap();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("AGENTS.md"), "root instructions").unwrap();
    fs::write(repo.join("a").join("CLAUDE.md"), "nested instructions").unwrap();

    let context = build_project_context(&nested);

    assert!(context.contains("root instructions"));
    assert!(context.contains("nested instructions"));
}

#[test]
fn build_mako_context_loads_global_home_files_and_project_overlay() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    let mako_home = temp.path().join("mako-home");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&mako_home).unwrap();

    fs::write(mako_home.join(paths::MAKO_SOUL_FILE), "Keep moving.").unwrap();
    fs::write(mako_home.join(paths::MAKO_IDENTITY_FILE), "Name: Mako").unwrap();
    fs::write(
        mako_home.join(paths::MAKO_HEARTBEAT_FILE),
        "Check queued work.",
    )
    .unwrap();
    fs::write(repo.join("MAKO.md"), "Project-specific operating notes.").unwrap();

    let context = build_mako_context_sections_with_home(&repo, &mako_home, None).join("\n\n");

    assert!(context.contains("[MAKO SOUL - MAKO_SOUL.md]"));
    assert!(context.contains("Keep moving."));
    assert!(context.contains("[MAKO IDENTITY - MAKO_IDENTITY.md]"));
    assert!(context.contains("Name: Mako"));
    assert!(context.contains("[MAKO HEARTBEAT - MAKO_HEARTBEAT.md]"));
    assert!(context.contains("Check queued work."));
    assert!(context.contains("[MAKO PROJECT OVERLAY - MAKO.md]"));
    assert!(context.contains("Project-specific operating notes."));
}

#[test]
fn build_mako_context_falls_back_to_project_overlay_when_global_home_is_empty() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    let mako_home = temp.path().join("mako-home");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&mako_home).unwrap();
    fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

    let context = build_mako_context_sections_with_home(&repo, &mako_home, None).join("\n\n");

    assert!(context.contains("[MAKO PROJECT OVERLAY - MAKO.md]"));
    assert!(context.contains("Always Swimming."));
}

#[test]
fn build_mako_context_accepts_legacy_generic_home_file_names() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    let mako_home = temp.path().join("mako-home");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&mako_home).unwrap();

    fs::write(mako_home.join("SOUL.md"), "Legacy soul.").unwrap();
    fs::write(mako_home.join("IDENTITY.md"), "Legacy identity.").unwrap();

    let context = build_mako_context_sections_with_home(&repo, &mako_home, None).join("\n\n");

    assert!(context.contains("[MAKO SOUL - SOUL.md]"));
    assert!(context.contains("Legacy soul."));
    assert!(context.contains("[MAKO IDENTITY - IDENTITY.md]"));
    assert!(context.contains("Legacy identity."));
}

#[test]
fn build_mako_context_never_activates_legacy_crew_memory_as_instructions() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    let mako_home = temp.path().join("mako-home");
    let crew = mako_home.join("crew").join("reviewer");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&crew).unwrap();
    fs::write(crew.join("IDENTITY.md"), "Reviewer identity.").unwrap();
    fs::write(crew.join("SOUL.md"), "Evidence first.").unwrap();
    fs::write(crew.join("MEMORY.md"), "legacy-secret-memory-marker").unwrap();

    let context =
        build_mako_context_sections_with_home(&repo, &mako_home, Some("reviewer")).join("\n\n");

    assert!(context.contains("Reviewer identity."));
    assert!(context.contains("Evidence first."));
    assert!(!context.contains("legacy-secret-memory-marker"));
    assert!(!context.contains("CREW MEMORY"));
}

#[test]
fn build_mako_context_uses_global_home_path_helper_without_panic() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

    let context = build_mako_context_sections(&repo, None).join("\n\n");

    assert!(context.contains("Always Swimming."));
}

#[test]
fn build_mako_context_sections_preserve_layer_order() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    let mako_home = temp.path().join("mako-home");
    fs::create_dir_all(&repo).unwrap();
    fs::create_dir_all(&mako_home).unwrap();

    fs::write(mako_home.join(paths::MAKO_SOUL_FILE), "Soul.").unwrap();
    fs::write(mako_home.join(paths::MAKO_IDENTITY_FILE), "Identity.").unwrap();
    fs::write(mako_home.join(paths::MAKO_USER_FILE), "User.").unwrap();
    fs::write(mako_home.join(paths::MAKO_HEARTBEAT_FILE), "Heartbeat.").unwrap();
    fs::write(mako_home.join(paths::MAKO_MEMORY_FILE), "Memory.").unwrap();
    fs::write(mako_home.join(paths::MAKO_CHANNELS_FILE), "Channels.").unwrap();
    fs::write(repo.join("MAKO.md"), "Overlay.").unwrap();

    let sections = build_mako_context_sections_with_home(&repo, &mako_home, None);
    let labels = sections
        .iter()
        .map(|section| {
            section
                .lines()
                .next()
                .unwrap_or_default()
                .trim()
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        labels,
        vec![
            "[MAKO SOUL - MAKO_SOUL.md]".to_string(),
            "[MAKO IDENTITY - MAKO_IDENTITY.md]".to_string(),
            "[MAKO USER - MAKO_USER.md]".to_string(),
            "[MAKO HEARTBEAT - MAKO_HEARTBEAT.md]".to_string(),
            "[MAKO CHANNELS - MAKO_CHANNELS.md]".to_string(),
            "[MAKO PROJECT OVERLAY - MAKO.md]".to_string(),
        ]
    );
    assert!(sections.iter().all(|section| !section.contains("Memory.")));
}

#[test]
fn build_plan_context_falls_back_to_generic_plan_mode_when_store_unavailable() {
    let temp = TempDir::new().unwrap();
    let missing_db_path = temp.path().join("missing").join("krusty.db");

    let context = build_plan_context(&missing_db_path, "session-id", WorkMode::Plan);

    assert!(context.contains("[PLAN MODE ACTIVE]"));
    assert!(context.contains("You CANNOT write, edit, or create files"));
}

#[test]
fn build_plan_context_returns_empty_when_store_unavailable_in_build_mode() {
    let temp = TempDir::new().unwrap();
    let missing_db_path = temp.path().join("missing").join("krusty.db");

    let context = build_plan_context(&missing_db_path, "session-id", WorkMode::Build);

    assert!(context.is_empty());
}

#[test]
fn build_plan_context_uses_compact_active_task_view_in_build_mode() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("plans.db");
    let session_id = SessionManager::new(Database::new(&db_path).unwrap())
        .create_session("plan context test", None, None)
        .unwrap();
    let manager = PlanManager::new(db_path.clone()).unwrap();
    let mut plan = manager
        .create_plan("Token efficient plan", &session_id, None)
        .unwrap();
    let phase = plan.add_phase("Implementation");
    for index in 0..18 {
        phase.add_task(format!("Implement bounded item {index}"));
    }
    manager.save_plan(&plan).unwrap();

    let context = build_plan_context(&db_path, &session_id, WorkMode::Build);

    assert!(context.contains("[ACTIVE PLAN"));
    assert!(context.contains("Additional tasks omitted from prompt"));
    assert!(!context.contains("## Current Plan"));
    assert!(!context.contains("Task Workflow Protocol"));
}

#[test]
fn build_skills_context_returns_empty_when_manager_is_busy() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let _guard = skills
        .try_write()
        .unwrap_or_else(|_| panic!("test should acquire write lock"));

    let context = build_skills_context(&skills, true);

    assert!(context.is_empty());
}

#[test]
fn build_environment_context_does_not_execute_repo_local_git_commands() {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().join("repo");
    fs::create_dir_all(&repo_path).unwrap();
    git2::Repository::init(&repo_path).unwrap();

    let payload_path = repo_path.join("fsmonitor-payload.sh");
    let marker_path = repo_path.join("fsmonitor-marker");
    fs::write(
        &payload_path,
        format!("#!/bin/sh\necho vulnerable > {}\n", marker_path.display()),
    )
    .unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&payload_path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&payload_path, permissions).unwrap();
    }

    fs::OpenOptions::new()
        .append(true)
        .open(repo_path.join(".git").join("config"))
        .unwrap()
        .write_all(format!("\n[core]\n	fsmonitor = {}\n", payload_path.display()).as_bytes())
        .unwrap();

    let context = build_environment_context(&repo_path, Some("test-model"));

    assert!(context.contains("Git repository: yes"));
    assert!(context.contains("Model: test-model"));
    assert!(
        !marker_path.exists(),
        "environment context must not execute repo-local git helpers"
    );
}

#[test]
fn summarize_git_status_counts_modified_staged_and_untracked() {
    let summary = summarize_git_status(" M src/lib.rs\nA  Cargo.toml\n?? scratch.txt\n");

    assert_eq!(
        summary.as_deref(),
        Some("1 modified, 1 staged, 1 untracked")
    );
}

#[test]
fn summarize_git_status_returns_none_for_clean_status() {
    let summary = summarize_git_status("");

    assert!(summary.is_none());
}

#[test]
fn inject_context_skips_project_instructions_without_explicit_project_dir() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("AGENTS.md"), "repo instructions").unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "hello".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        repo.join("krusty.db").as_path(),
        "session-id",
        repo,
        None,
        WorkMode::Build,
        &skills,
        None,
        None,
        None,
        None,
    );

    assert!(injected.len() >= 3);
    assert_eq!(injected[0].role, Role::System);
    assert!(matches!(
        &injected[0].content[0],
        Content::Text { text } if text.contains("[WORKSPACE MODE: NEUTRAL]")
    ));
    assert_eq!(injected[1].role, Role::System);
    assert!(matches!(
        &injected[1].content[0],
        Content::Text { text } if text.contains("[ENVIRONMENT]")
    ));
    assert!(matches!(
        injected.last(),
        Some(message) if message.role == Role::User
    ));
    assert!(injected.iter().all(|message| {
        message.content.iter().all(|content| {
            !matches!(content, Content::Text { text } if text.contains("repo instructions"))
        })
    }));
}

#[test]
fn chat_context_does_not_inject_a_conflicting_tool_policy() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "research the current release".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        repo.join("krusty.db").as_path(),
        "chat-session",
        repo,
        None,
        WorkMode::Build,
        &skills,
        None,
        Some("chat"),
        None,
        None,
    );

    assert!(injected.iter().all(|message| {
        message.content.iter().all(|content| match content {
            Content::Text { text } => !text.contains("do NOT have access to any tools"),
            _ => true,
        })
    }));
    assert_eq!(injected.last().unwrap().role, Role::User);
}

#[test]
fn aggregate_dynamic_context_budget_preserves_high_priority_sections() {
    let system_message = |text: String| ModelMessage {
        role: Role::System,
        content: vec![Content::Text { text }],
    };
    let mut messages = vec![
        system_message(format!("[PERSISTENT MEMORY]\n{}", "memory ".repeat(8_000))),
        system_message(format!(
            "[PROJECT INSTRUCTIONS - AGENTS.md]\n{}",
            "instructions ".repeat(4_000)
        )),
        system_message("[ACTIVE PLAN - audit]\nFinish the selected task.".to_string()),
    ];

    bound_dynamic_context_messages(&mut messages);

    let retained_bytes = messages
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::Text { text } => Some(text.len()),
            _ => None,
        })
        .sum::<usize>();
    assert!(retained_bytes <= MAX_DYNAMIC_CONTEXT_BYTES);
    assert!(messages.iter().any(|message| {
        matches!(&message.content[0], Content::Text { text } if text.starts_with("[ACTIVE PLAN"))
    }));
    assert!(messages.iter().any(|message| {
        matches!(&message.content[0], Content::Text { text } if text.starts_with("[PROJECT INSTRUCTIONS"))
    }));
}

#[test]
fn aggregate_context_pressure_preserves_complete_mako_identity_before_optional_context() {
    let system_message = |text: String| ModelMessage {
        role: Role::System,
        content: vec![Content::Text { text }],
    };
    let mut messages = vec![
        system_message(format!(
            "[PERSISTENT MEMORY]\n{}",
            "optional memory ".repeat(4_000)
        )),
        system_message(format!(
            "[PROJECT INSTRUCTIONS - AGENTS.md]\n{}",
            "project instruction ".repeat(3_000)
        )),
        system_message("[MAKO COORDINATOR]\nCoordinate deliberately.".to_string()),
        system_message(format!(
            "[MAKO SOUL - profile:local]\n{}",
            "soul ".repeat(1_000)
        )),
        system_message(format!(
            "[MAKO IDENTITY - profile:local]\n{}",
            "identity ".repeat(800)
        )),
        system_message(format!(
            "[MAKO USER - profile:local]\n{}",
            "user preference ".repeat(600)
        )),
        system_message(format!(
            "[MAKO HEARTBEAT - profile:local]\n{}",
            "heartbeat ".repeat(3_000)
        )),
        system_message(format!(
            "[MAKO CHANNELS - profile:local]\n{}",
            "channels ".repeat(3_000)
        )),
    ];

    bound_dynamic_context_messages(&mut messages);

    let texts = messages
        .iter()
        .filter_map(|message| match &message.content[0] {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for prefix in [
        "[MAKO COORDINATOR]",
        "[MAKO SOUL",
        "[MAKO IDENTITY",
        "[MAKO USER",
    ] {
        let retained = texts
            .iter()
            .find(|text| text.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing stable Mako section {prefix}"));
        assert!(!retained.contains("TRUNCATED AT AGGREGATE CONTEXT BUDGET"));
    }
    assert!(texts.iter().all(|text| {
        !text.starts_with("[PERSISTENT MEMORY]")
            || text.contains("TRUNCATED AT AGGREGATE CONTEXT BUDGET")
    }));
    let retained_bytes = texts.iter().map(|text| text.len()).sum::<usize>();
    assert!(retained_bytes <= MAX_DYNAMIC_CONTEXT_BYTES);
}

#[test]
fn inject_context_filters_persistent_memory_by_user_id() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    let db_path = repo.join("krusty.db");
    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
    memory_store
        .save(
            MemoryType::User,
            "Alice secret",
            "alice-only instruction",
            None,
            Some("alice"),
        )
        .unwrap();
    memory_store
        .save(
            MemoryType::User,
            "Bob preference",
            "bob-only preference",
            None,
            Some("bob"),
        )
        .unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "please use bob preference".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        db_path.as_path(),
        "session-id",
        repo,
        None,
        WorkMode::Build,
        &skills,
        None,
        None,
        None,
        Some("bob"),
    );

    let context = injected
        .iter()
        .filter_map(|message| match &message.content[0] {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(context.contains("Bob preference"));
    assert!(context.contains("bob-only preference"));
    assert!(!context.contains("Alice secret"));
    assert!(!context.contains("alice-only instruction"));
}

#[test]
fn inject_context_uses_latest_user_memory_relevance_and_hides_compaction_flushes() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    let db_path = repo.join("krusty.db");
    let project_dir = repo.to_string_lossy();
    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
    memory_store
        .save(
            MemoryType::Project,
            "Compaction archive",
            "Prior compaction cleanup notes that should not leak into unrelated trivia.",
            Some(project_dir.as_ref()),
            None,
        )
        .unwrap();
    memory_store
        .save(
            MemoryType::Project,
            "Space notes",
            "Use space telescope examples when the user asks about space facts.",
            Some(project_dir.as_ref()),
            None,
        )
        .unwrap();
    memory_store
        .save(
            MemoryType::Project,
            &format!("{}1", crate::storage::COMPACTION_FLUSH_TITLE_PREFIX),
            "Full old conversation transcript should never be injected as memory.",
            Some(project_dir.as_ref()),
            None,
        )
        .unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![
        ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "we were discussing compaction cleanup".to_string(),
            }],
        },
        ModelMessage {
            role: Role::Assistant,
            content: vec![Content::Text {
                text: "Understood.".to_string(),
            }],
        },
        ModelMessage {
            role: Role::User,
            content: vec![Content::Text {
                text: "tell me facts about space".to_string(),
            }],
        },
    ];

    let injected = inject_context(
        &conversation,
        db_path.as_path(),
        "session-id",
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("code"),
        None,
        None,
    );

    let context = injected
        .iter()
        .filter_map(|message| match &message.content[0] {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(context.contains("Space notes"));
    assert!(!context.contains("Compaction archive"));
    assert!(!context.contains("Full old conversation transcript"));
}

#[test]
fn generic_project_words_do_not_trigger_persistent_memory_injection() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    let db_path = repo.join("krusty.db");
    let project_dir = repo.to_string_lossy();
    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
    for index in 0..12 {
        memory_store
            .save(
                MemoryType::Project,
                &format!("Project note {index}"),
                "Generic project system context and coding work notes.",
                Some(project_dir.as_ref()),
                None,
            )
            .unwrap();
    }

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "Tell me about this project and its code.".to_string(),
        }],
    }];
    let injected = inject_context(
        &conversation,
        &db_path,
        "session-id",
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("code"),
        None,
        None,
    );

    assert!(injected.iter().all(|message| {
        !matches!(&message.content[0], Content::Text { text } if text.contains("[PERSISTENT MEMORY]"))
    }));
}

#[test]
fn inject_context_includes_mako_identity_only_for_mako_sessions() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("AGENTS.md"), "repo instructions").unwrap();
    fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "hello".to_string(),
        }],
    }];

    let mako_injected = inject_context(
        &conversation,
        repo.join("krusty.db").as_path(),
        "session-id",
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("mako"),
        None,
        None,
    );
    let code_injected = inject_context(
        &conversation,
        repo.join("krusty.db").as_path(),
        "session-id",
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("code"),
        None,
        None,
    );

    assert!(mako_injected.iter().any(|message| {
        matches!(
            &message.content[0],
            Content::Text { text } if text.contains("[MAKO PROJECT OVERLAY - MAKO.md]") && text.contains("Always Swimming.")
        )
    }));
    assert!(mako_injected.iter().all(|message| {
        !matches!(
            &message.content[0],
            Content::Text { text } if text.contains("[MAKO HOME ")
        )
    }));
    assert!(!code_injected.iter().any(|message| {
        matches!(
            &message.content[0],
            Content::Text { text } if text.contains("[MAKO PROJECT OVERLAY - MAKO.md]")
        )
    }));
}

#[test]
fn inject_context_places_all_mako_layers_before_project_settings() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join(".krusty")).unwrap();
    fs::write(repo.join("AGENTS.md"), "repo instructions").unwrap();
    fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();
    fs::write(
        repo.join(".krusty").join("settings.json"),
        r#"{ "system_prompt_append": "Project append." }"#,
    )
    .unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "hello".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        repo.join("krusty.db").as_path(),
        "session-id",
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("mako"),
        None,
        None,
    );

    let texts = injected
        .iter()
        .filter_map(|message| match &message.content[0] {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let settings_index = texts
        .iter()
        .position(|text| text.contains("[PROJECT SETTINGS]"))
        .unwrap();
    let mako_indices = texts
        .iter()
        .enumerate()
        .filter_map(|(index, text)| text.contains("[MAKO ").then_some(index))
        .collect::<Vec<_>>();

    assert!(!mako_indices.is_empty());
    assert!(mako_indices.iter().all(|index| *index < settings_index));
}

#[test]
fn inject_context_includes_mako_coordinator_prompt_for_mako_sessions() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "hello".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        repo.join("krusty.db").as_path(),
        "session-id",
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("mako"),
        None,
        None,
    );

    assert!(injected.iter().any(|message| {
        matches!(
            &message.content[0],
            Content::Text { text } if text.contains("[MAKO COORDINATOR]")
        )
    }));
}

#[test]
fn inject_context_includes_mako_knowledge_from_memory_and_reports() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

    let db_path = repo.join("krusty.db");
    let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
    let session_id = session_manager
        .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
        .unwrap();

    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
    memory_store
        .save(
            MemoryType::Project,
            "Auth decision",
            "Use the daemon loop as the canonical wake path.",
            Some(repo.to_string_lossy().as_ref()),
            None,
        )
        .unwrap();
    memory_store
        .save(
            MemoryType::Project,
            &format!("{}1", crate::storage::COMPACTION_FLUSH_TITLE_PREFIX),
            "Full compacted transcript should not appear in Mako knowledge.",
            Some(repo.to_string_lossy().as_ref()),
            None,
        )
        .unwrap();

    let report_store = ReportStore::new(Database::new(&db_path).unwrap());
    report_store
        .create_report(CreateReportInput {
            title: "Wake pipeline check",
            session_id: &session_id,
            project_dir: Some(repo.to_string_lossy().as_ref()),
            report_root: Some(repo),
            content: "The wake pipeline is healthy.",
            summary: "Validated the wake pipeline end to end.",
            tags: &[],
            sources: &[],
        })
        .unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "Check the wake pipeline health.".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        db_path.as_path(),
        &session_id,
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("mako"),
        None,
        None,
    );

    let context = injected
        .iter()
        .filter_map(|message| match &message.content[0] {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    assert!(context.contains("[MAKO KNOWLEDGE]"));
    assert!(context.contains("## Carry Forward"));
    assert!(context.contains("Auth decision"));
    assert!(context.contains("## Relevant Reports"));
    assert!(context.contains("Wake pipeline check"));
    assert!(!context.contains("Full compacted transcript"));
}

#[test]
fn mako_knowledge_prompt_is_exact_owner_for_alice_bob_and_local() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("mako-owner-knowledge.db");
    let project = "/shared/project";
    let db = Database::new(&db_path).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO users (id, email, license_tier)
                 VALUES ('alice', 'alice@knowledge.test', 'free');
             INSERT INTO users (id, email, license_tier)
                 VALUES ('bob', 'bob@knowledge.test', 'free');",
        )
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    for (session_id, title, user_id) in [
        ("mako-local", "Local Mako", None),
        ("mako-alice", "Alice Mako", Some("alice")),
        ("mako-bob", "Bob Mako", Some("bob")),
    ] {
        db.conn()
            .execute(
                "INSERT INTO sessions (
                    id, title, created_at, updated_at, working_dir, project_dir,
                    workspace_mode, user_id, session_type
                 ) VALUES (?1, ?2, ?3, ?3, ?4, ?4, 'selected', ?5, 'mako')",
                rusqlite::params![session_id, title, now, project, user_id],
            )
            .unwrap();
    }

    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
    for (title, content, user_id) in [
        ("Local queue ownership", "local-memory-marker", None),
        (
            "Alice queue ownership",
            "alice-memory-marker",
            Some("alice"),
        ),
        ("Bob queue ownership", "bob-memory-marker", Some("bob")),
    ] {
        memory_store
            .save(MemoryType::Project, title, content, Some(project), user_id)
            .unwrap();
    }

    let report_store = ReportStore::new(Database::new(&db_path).unwrap());
    for (session_id, title, summary) in [
        ("mako-local", "Local queue ownership", "local-report-marker"),
        ("mako-alice", "Alice queue ownership", "alice-report-marker"),
        ("mako-bob", "Bob queue ownership", "bob-report-marker"),
    ] {
        report_store
            .create_report(CreateReportInput {
                title,
                session_id,
                project_dir: Some(project),
                report_root: None,
                content: summary,
                summary,
                tags: &["queue".to_string(), "ownership".to_string()],
                sources: &[],
            })
            .unwrap();
    }

    let task_store = AutonomousTaskStore::new(Database::new(&db_path).unwrap());
    for (session_id, subject) in [
        ("mako-local", "local-task-marker"),
        ("mako-alice", "alice-task-marker"),
        ("mako-bob", "bob-task-marker"),
    ] {
        task_store
            .create_task(session_id, subject, "", &[])
            .unwrap();
    }

    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "Review queue ownership evidence.".to_string(),
        }],
    }];
    let alice = build_mako_knowledge_context(
        &db_path,
        Some(project),
        Some("alice"),
        None,
        "mako-alice",
        &conversation,
    );
    let bob = build_mako_knowledge_context(
        &db_path,
        Some(project),
        Some("bob"),
        None,
        "mako-bob",
        &conversation,
    );
    let local = build_mako_knowledge_context(
        &db_path,
        Some(project),
        None,
        None,
        "mako-local",
        &conversation,
    );

    for (context, owned, foreign_a, foreign_b) in [
        (
            alice.as_str(),
            "alice-memory-marker",
            "bob-memory-marker",
            "local-memory-marker",
        ),
        (
            bob.as_str(),
            "bob-memory-marker",
            "alice-memory-marker",
            "local-memory-marker",
        ),
        (
            local.as_str(),
            "local-memory-marker",
            "alice-memory-marker",
            "bob-memory-marker",
        ),
    ] {
        assert!(context.contains(owned));
        assert!(!context.contains(foreign_a));
        assert!(!context.contains(foreign_b));
    }
    assert!(alice.contains("alice-report-marker"));
    assert!(alice.contains("alice-task-marker"));
    assert!(!alice.contains("bob-report-marker"));
    assert!(!alice.contains("bob-task-marker"));
    assert!(!alice.contains("local-report-marker"));
    assert!(!alice.contains("local-task-marker"));
    assert!(bob.contains("bob-report-marker"));
    assert!(bob.contains("bob-task-marker"));
    assert!(!bob.contains("alice-report-marker"));
    assert!(!bob.contains("alice-task-marker"));
    assert!(!bob.contains("local-report-marker"));
    assert!(!bob.contains("local-task-marker"));
    assert!(local.contains("local-report-marker"));
    assert!(local.contains("local-task-marker"));
    assert!(!local.contains("alice-report-marker"));
    assert!(!local.contains("alice-task-marker"));
    assert!(!local.contains("bob-report-marker"));
    assert!(!local.contains("bob-task-marker"));
}

#[test]
fn mako_knowledge_prompt_isolated_by_primary_and_named_crew_namespace() {
    let temp = TempDir::new().unwrap();
    let db_path = temp.path().join("mako-crew-knowledge.db");
    let project = "/crew/project";
    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());

    for (canonical_key, title, content, namespace, namespace_id) in [
        (
            "shared-style",
            "Shared style",
            "shared-memory-marker",
            MemoryNamespace::Shared,
            None,
        ),
        (
            "primary-style",
            "Primary style",
            "primary-mako-marker",
            MemoryNamespace::Mako,
            None,
        ),
        (
            "reviewer-style",
            "Reviewer style",
            "reviewer-crew-marker",
            MemoryNamespace::Crew,
            Some("reviewer"),
        ),
        (
            "researcher-style",
            "Researcher style",
            "researcher-crew-marker",
            MemoryNamespace::Crew,
            Some("researcher"),
        ),
    ] {
        let mut input =
            CanonicalMemoryInput::new(MemoryType::Project, canonical_key, title, content);
        input.project_dir = Some(project.to_string());
        input.namespace = namespace;
        input.namespace_id = namespace_id.map(str::to_string);
        memory_store.save_canonical(&input).unwrap();
    }

    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "Recall the correct working style.".to_string(),
        }],
    }];
    let primary = build_mako_knowledge_context(
        &db_path,
        Some(project),
        None,
        None,
        "mako-primary",
        &conversation,
    );
    let reviewer = build_mako_knowledge_context(
        &db_path,
        Some(project),
        None,
        Some("reviewer"),
        "mako-reviewer",
        &conversation,
    );

    assert!(primary.contains("shared-memory-marker"));
    assert!(primary.contains("primary-mako-marker"));
    assert!(!primary.contains("reviewer-crew-marker"));
    assert!(!primary.contains("researcher-crew-marker"));

    assert!(reviewer.contains("shared-memory-marker"));
    assert!(reviewer.contains("reviewer-crew-marker"));
    assert!(!reviewer.contains("primary-mako-marker"));
    assert!(!reviewer.contains("researcher-crew-marker"));
    assert!(!reviewer.contains("## Current Snapshot"));
}

#[test]
fn inject_context_prioritizes_relevant_reports_over_recent_reports() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();

    let db_path = repo.join("krusty.db");
    let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
    let session_id = session_manager
        .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
        .unwrap();

    let report_store = ReportStore::new(Database::new(&db_path).unwrap());
    report_store
        .create_report(CreateReportInput {
            title: "Queue Scheduling Audit",
            session_id: &session_id,
            project_dir: Some(repo.to_string_lossy().as_ref()),
            report_root: Some(repo),
            content: "Investigated overdue runs and wake cadence.",
            summary: "Queue scheduling and overdue run analysis.",
            tags: &["queue".into(), "scheduling".into()],
            sources: &[],
        })
        .unwrap();
    for index in 0..5 {
        report_store
            .create_report(CreateReportInput {
                title: &format!("Unrelated report {index}"),
                session_id: &session_id,
                project_dir: Some(repo.to_string_lossy().as_ref()),
                report_root: Some(repo),
                content: "Miscellaneous notes.",
                summary: "General project notes.",
                tags: &["misc".into()],
                sources: &[],
            })
            .unwrap();
    }

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "Please stabilize queue scheduling and overdue runs.".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        db_path.as_path(),
        &session_id,
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("code"),
        None,
        None,
    );

    assert!(injected.iter().any(|message| {
        matches!(
            &message.content[0],
            Content::Text { text }
                if text.contains("[RELEVANT REPORTS]")
                    && text.contains("Queue Scheduling Audit")
        )
    }));

    let joined = injected
        .iter()
        .flat_map(|message| &message.content)
        .filter_map(|content| match content {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("Unrelated report"));
}

#[test]
fn inject_context_omits_reports_when_none_are_relevant() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    let db_path = repo.join("krusty.db");
    let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
    let session_id = session_manager
        .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
        .unwrap();
    ReportStore::new(Database::new(&db_path).unwrap())
        .create_report(CreateReportInput {
            title: "Database migration retrospective",
            session_id: &session_id,
            project_dir: Some(repo.to_string_lossy().as_ref()),
            report_root: Some(repo),
            content: "Postgres migration notes.",
            summary: "Schema rollout findings.",
            tags: &["database".into()],
            sources: &[],
        })
        .unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "Tune the mobile animation easing.".to_string(),
        }],
    }];
    let injected = inject_context(
        &conversation,
        &db_path,
        &session_id,
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("code"),
        None,
        None,
    );

    assert!(injected.iter().all(|message| {
        message.content.iter().all(|content| {
            !matches!(content, Content::Text { text } if text.contains("[RELEVANT REPORTS]"))
        })
    }));
}

#[test]
fn inject_context_uses_active_mako_tasks_for_report_relevance() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

    let db_path = repo.join("krusty.db");
    let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
    let session_id = session_manager
        .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
        .unwrap();

    let report_store = ReportStore::new(Database::new(&db_path).unwrap());
    report_store
        .create_report(CreateReportInput {
            title: "Scheduler Drift Runbook",
            session_id: &session_id,
            project_dir: Some(repo.to_string_lossy().as_ref()),
            report_root: Some(repo),
            content: "Drift handling and wake diagnostics.",
            summary: "How to investigate scheduler drift safely.",
            tags: &["scheduler".into(), "drift".into()],
            sources: &[],
        })
        .unwrap();
    for index in 0..5 {
        report_store
            .create_report(CreateReportInput {
                title: &format!("Background note {index}"),
                session_id: &session_id,
                project_dir: Some(repo.to_string_lossy().as_ref()),
                report_root: Some(repo),
                content: "Background knowledge.",
                summary: "General notes.",
                tags: &["misc".into()],
                sources: &[],
            })
            .unwrap();
    }

    let task_store = AutonomousTaskStore::new(Database::new(&db_path).unwrap());
    task_store
        .create_task(&session_id, "Investigate scheduler drift", "", &[])
        .unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "Keep watch and continue.".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        db_path.as_path(),
        &session_id,
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("mako"),
        None,
        None,
    );

    assert!(injected.iter().any(|message| {
        matches!(
            &message.content[0],
            Content::Text { text }
                if text.contains("[MAKO KNOWLEDGE]")
                    && text.contains("## Relevant Reports")
                    && text.contains("Scheduler Drift Runbook")
        )
    }));
}

#[test]
fn inject_context_does_not_duplicate_generic_memory_and_report_blocks_for_mako() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::write(repo.join("MAKO.md"), "Always Swimming.").unwrap();

    let db_path = repo.join("krusty.db");
    let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
    let session_id = session_manager
        .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
        .unwrap();

    let memory_store = MemoryStore::new(Database::new(&db_path).unwrap());
    memory_store
        .save(
            MemoryType::Feedback,
            "Status preference",
            "Show upcoming wakes before aggregate counters.",
            Some(repo.to_string_lossy().as_ref()),
            None,
        )
        .unwrap();

    let report_store = ReportStore::new(Database::new(&db_path).unwrap());
    report_store
        .create_report(CreateReportInput {
            title: "Queue audit",
            session_id: &session_id,
            project_dir: Some(repo.to_string_lossy().as_ref()),
            report_root: Some(repo),
            content: "Queue ordering is stable.",
            summary: "Queue ordering remains stable.",
            tags: &[],
            sources: &[],
        })
        .unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "hello".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        db_path.as_path(),
        &session_id,
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("mako"),
        None,
        None,
    );

    let texts = injected
        .iter()
        .filter_map(|message| match &message.content[0] {
            Content::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert!(texts.iter().any(|text| text.contains("[MAKO KNOWLEDGE]")));
    assert!(!texts
        .iter()
        .any(|text| text.contains("[PERSISTENT MEMORY]")));
    assert!(!texts.iter().any(|text| text.contains("[RECENT REPORTS]")));
}

#[test]
fn inject_context_includes_recent_delegated_run_guidance() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    let db_path = repo.join("krusty.db");
    let db = Database::new(&db_path).unwrap();
    let session_manager = SessionManager::new(Database::new(&db_path).unwrap());
    let session_id = session_manager
        .create_session("test", None, Some(repo.to_string_lossy().as_ref()))
        .unwrap();
    let store = DelegatedRunStore::new(db);
    store
        .create_run(&DelegatedRunStartInput {
            delegated_run_id: "run-1".to_string(),
            parent_session_id: session_id.clone(),
            parent_tool_call_id: Some("tool-1".to_string()),
            role: DelegatedRunRole::Explore,
            stage: DelegatedRunStage::Created,
            provider: Some("MiniMax".to_string()),
            model: Some("MiniMax-M2.5".to_string()),
            resumable: true,
            resumed_from_run_id: None,
            target_scope: vec![DelegatedRunScope {
                label: "src/storage".to_string(),
                path: "crates/krusty-core/src/storage".to_string(),
                kind: "directory".to_string(),
            }],
        })
        .unwrap();
    store
        .finalize_run(
            "run-1",
            DelegatedRunStage::Complete,
            &serde_json::json!({"outcome":"success"}),
            Some("Architecture review completed across 1 targets."),
            true,
        )
        .unwrap();

    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "use explore again".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        db_path.as_path(),
        &session_id,
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        None,
        None,
        None,
    );

    assert!(injected.iter().any(|message| {
        matches!(
            &message.content[0],
            Content::Text { text }
                if text.contains("[RECENT DELEGATED RUNS]")
                    && text.contains("prefer calling `explore` again")
                    && text.contains("run-1")
                    && text.contains("src/storage")
        )
    }));
}

#[test]
fn inject_context_adds_balanced_delegation_contract_for_code_sessions() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "hello".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        repo.join("krusty.db").as_path(),
        "session-id",
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("code"),
        None,
        None,
    );

    assert!(injected.iter().any(|message| {
        matches!(
            &message.content[0],
            Content::Text { text }
                if text.contains("[DELEGATION MODE: BALANCED]")
                    && text.contains("simple, tightly coupled, or sequential")
        )
    }));
}

#[test]
fn inject_context_honors_explicit_only_delegation_setting() {
    let temp = TempDir::new().unwrap();
    let repo = temp.path();
    fs::create_dir_all(repo.join(".git")).unwrap();
    fs::create_dir_all(repo.join(".krusty")).unwrap();
    fs::write(
        repo.join(".krusty").join("settings.json"),
        r#"{ "delegation_mode": "explicit_only" }"#,
    )
    .unwrap();
    let skills = RwLock::new(SkillsManager::with_defaults(repo));
    let conversation = vec![ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "hello".to_string(),
        }],
    }];

    let injected = inject_context(
        &conversation,
        repo.join("krusty.db").as_path(),
        "session-id",
        repo,
        Some(repo),
        WorkMode::Build,
        &skills,
        None,
        Some("code"),
        None,
        None,
    );

    assert!(injected.iter().any(|message| {
        matches!(
            &message.content[0],
            Content::Text { text }
                if text.contains("[DELEGATION MODE: EXPLICIT_ONLY]")
                    && text.contains("only when the user explicitly requests")
        )
    }));
}
