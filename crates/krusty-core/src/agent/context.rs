//! Context injection for the agentic loop.
//!
//! Builds plan, skills, and project context strings that get injected as
//! system messages at the head of the conversation before each AI call.
//! This ensures the AI is always aware of the active plan, available skills,
//! and project-specific instructions.

mod mako;
mod memory;
mod plan;
mod project;
mod reports;
mod runtime_state;
mod skills;
mod workspace;

#[cfg(test)]
mod tests;

use std::path::Path;

use tokio::sync::RwLock;
use tracing::warn;

use crate::ai::types::{Content, ModelMessage, Role};
use crate::skills::SkillsManager;
use crate::storage::{Database, ProjectSettings, WorkMode};

pub use plan::build_plan_context;
pub use project::build_project_context;
pub use skills::build_skills_context;
pub use workspace::build_subagent_project_context;

/// Build a conversation clone with context system messages prepended.
///
/// Injects plan, skills, and project context in the same order as the TUI:
/// project → plan → skills → original conversation.
pub fn inject_context(
    conversation: &[ModelMessage],
    db_path: &Path,
    session_id: &str,
    working_dir: &Path,
    project_dir: Option<&Path>,
    work_mode: WorkMode,
    skills_manager: &RwLock<SkillsManager>,
    model_id: Option<&str>,
    session_type: Option<&str>,
    mako_crew_slug: Option<&str>,
    user_id: Option<&str>,
) -> Vec<ModelMessage> {
    let is_chat = session_type == Some("chat");

    if is_chat {
        let mut injected = Vec::with_capacity(conversation.len() + 2);
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: "You are Krusty, a helpful conversational assistant. This is a chat session — you are having a natural conversation with the user.\n\nIMPORTANT: You do NOT have access to any tools in this session. Do not mention, list, or describe any tools. You cannot read files, run commands, or edit code. If the user asks about tools, explain that this is a chat-only session and suggest they switch to Code mode for coding tasks.\n\nBe friendly, helpful, and conversational.".to_string(),
            }],
        });
        let memory_ctx = memory::build_memory_context(db_path, None, user_id);
        if !memory_ctx.is_empty() {
            injected.push(ModelMessage {
                role: Role::System,
                content: vec![Content::Text { text: memory_ctx }],
            });
        }
        injected.extend_from_slice(conversation);
        return injected;
    }

    let workspace_ctx = workspace::build_workspace_context(working_dir, project_dir);
    let env_ctx = workspace::build_environment_context(working_dir, model_id);
    let context_project_dir = project_dir.map(|p| p.to_string_lossy().to_string());
    let is_mako = session_type == Some("mako");
    let memory_ctx = if is_mako {
        String::new()
    } else {
        memory::build_memory_context(db_path, context_project_dir.as_deref(), user_id)
    };
    let plan_ctx = build_plan_context(db_path, session_id, work_mode);
    let delegated_ctx = runtime_state::build_delegated_context(db_path, session_id);
    let task_ctx = runtime_state::build_autonomous_task_context(db_path, session_id);
    let report_ctx = if is_mako {
        String::new()
    } else {
        reports::build_report_context(db_path, context_project_dir.as_deref(), conversation)
    };
    let mako_knowledge_ctx = if is_mako {
        reports::build_mako_knowledge_context(
            db_path,
            context_project_dir.as_deref(),
            user_id,
            session_id,
            conversation,
        )
    } else {
        String::new()
    };
    let skills_ctx = build_skills_context(skills_manager, project_dir.is_some());
    let project_ctx = project_dir.map(build_project_context).unwrap_or_default();
    let mako_ctx_sections = if is_mako {
        mako::build_mako_context_sections(project_dir.unwrap_or(working_dir), mako_crew_slug)
    } else {
        Vec::new()
    };
    let project_settings = project_dir.map(ProjectSettings::load).unwrap_or_default();

    let mut injected = Vec::with_capacity(conversation.len() + 7);

    if !workspace_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: workspace_ctx,
            }],
        });
    }
    if !env_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: env_ctx }],
        });
    }
    if !memory_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: memory_ctx }],
        });
    }
    if !mako_knowledge_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: mako_knowledge_ctx,
            }],
        });
    }
    if !project_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: project_ctx }],
        });
    }
    for text in mako_ctx_sections {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text }],
        });
    }
    if let Some(ref append) = project_settings.system_prompt_append {
        if !append.is_empty() {
            injected.push(ModelMessage {
                role: Role::System,
                content: vec![Content::Text {
                    text: format!("[PROJECT SETTINGS]\n{}", append),
                }],
            });
        }
    }
    if !plan_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: plan_ctx }],
        });
    }
    if !delegated_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: delegated_ctx,
            }],
        });
    }
    if !task_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: task_ctx }],
        });
    }
    if !report_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: report_ctx }],
        });
    }
    if !skills_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text { text: skills_ctx }],
        });
    }

    injected.extend_from_slice(conversation);
    injected
}

/// Truncate a string to at most `max_chars` characters on a valid UTF-8
/// boundary, appending "..." when truncation occurs.
pub(super) fn truncate_utf8(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{}...", truncated)
}

pub(super) fn open_context_database(db_path: &Path, context: &'static str) -> Option<Database> {
    match Database::new(db_path) {
        Ok(db) => Some(db),
        Err(error) => {
            warn!(context, db_path = %db_path.display(), error = %error, "Failed to open context database");
            None
        }
    }
}
