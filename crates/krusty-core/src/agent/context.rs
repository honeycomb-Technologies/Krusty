//! Context injection for the agentic loop.
//!
//! Builds plan, skills, and project context strings that get injected as
//! system messages at the head of the conversation before each AI call.
//! This ensures the AI is always aware of the active plan, available skills,
//! and project-specific instructions.

mod mako;
mod memory;
mod episodes;
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
use crate::storage::{Database, MakoProfileSnapshot, ProjectSettings, WorkMode};

pub use plan::build_plan_context;
pub use project::build_project_context;
pub use skills::build_skills_context;
pub use workspace::build_subagent_project_context;

const MAX_PROJECT_SETTINGS_APPEND_BYTES: usize = 8 * 1024;
/// Aggregate ceiling for request-time system context assembled by this module.
/// Individual sources also have smaller limits, but the aggregate guard keeps
/// several simultaneously-large sources from bypassing those local budgets.
pub(super) const MAX_DYNAMIC_CONTEXT_BYTES: usize = 64 * 1024;

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
    inject_context_with_mako_profile(
        conversation,
        db_path,
        session_id,
        working_dir,
        project_dir,
        work_mode,
        skills_manager,
        model_id,
        session_type,
        mako_crew_slug,
        user_id,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn inject_context_with_mako_profile(
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
    mako_profile: Option<&MakoProfileSnapshot>,
) -> Vec<ModelMessage> {
    let is_chat = session_type == Some("chat");

    if is_chat {
        // The caller owns the Chat capability prompt because it also owns the
        // request-time tool filter (for example, research mode). Injecting a
        // second capability policy here can contradict the tools that are
        // actually present on the request or an explicit custom prompt.
        let mut injected = Vec::with_capacity(conversation.len() + 1);
        let memory_ctx = memory::build_memory_context(db_path, None, user_id, conversation);
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
        memory::build_memory_context(
            db_path,
            context_project_dir.as_deref(),
            user_id,
            conversation,
        )
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
    let mako_episode_ctx = if is_mako {
        episodes::build_episode_context(
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
    let mut mako_ctx_sections = if is_mako {
        if let Some(profile) = mako_profile {
            mako::build_mako_context_sections_with_profile(
                project_dir.unwrap_or(working_dir),
                profile,
                mako_crew_slug,
            )
        } else {
            mako::build_mako_context_sections(project_dir.unwrap_or(working_dir), mako_crew_slug)
        }
    } else {
        Vec::new()
    };
    if is_mako {
        mako_ctx_sections.insert(
            0,
            crate::agent::autonomy::coordinator_prompt::mako_coordinator_system_prompt(),
        );
    }
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
    if !mako_episode_ctx.is_empty() {
        injected.push(ModelMessage {
            role: Role::System,
            content: vec![Content::Text {
                text: mako_episode_ctx,
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
                    text: format!(
                        "[PROJECT SETTINGS]\n{}",
                        truncate_utf8_bytes(append, MAX_PROJECT_SETTINGS_APPEND_BYTES)
                    ),
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

    bound_dynamic_context_messages(&mut injected);
    injected.extend_from_slice(conversation);
    injected
}

fn bound_dynamic_context_messages(messages: &mut Vec<ModelMessage>) {
    let total_bytes = messages
        .iter()
        .filter_map(context_message_text)
        .map(str::len)
        .sum::<usize>();
    if total_bytes <= MAX_DYNAMIC_CONTEXT_BYTES {
        return;
    }

    let mut allocation_order = (0..messages.len()).collect::<Vec<_>>();
    allocation_order.sort_by(|left, right| {
        let left_text = context_message_text(&messages[*left]).unwrap_or_default();
        let right_text = context_message_text(&messages[*right]).unwrap_or_default();
        dynamic_context_priority(right_text)
            .cmp(&dynamic_context_priority(left_text))
            .then_with(|| left.cmp(right))
    });

    let mut remaining = MAX_DYNAMIC_CONTEXT_BYTES;
    let mut retained = vec![None; messages.len()];
    let mut omitted = Vec::new();
    let mut truncated = Vec::new();
    for index in allocation_order {
        let Some(text) = context_message_text(&messages[index]) else {
            continue;
        };
        let label = context_section_label(text);
        if text.len() <= remaining {
            retained[index] = Some(text.to_string());
            remaining -= text.len();
            continue;
        }

        let marker = format!("\n[{} TRUNCATED AT AGGREGATE CONTEXT BUDGET]", label);
        if remaining > marker.len() + 256 {
            let mut bounded = truncate_utf8_bytes(text, remaining - marker.len());
            bounded.push_str(&marker);
            retained[index] = Some(bounded);
            truncated.push(label);
            remaining = 0;
        } else {
            omitted.push(label);
        }
    }

    let mut bounded_messages = Vec::with_capacity(messages.len());
    for (index, mut message) in std::mem::take(messages).into_iter().enumerate() {
        if context_message_text(&message).is_none() {
            bounded_messages.push(message);
            continue;
        }
        let Some(text) = retained[index].take() else {
            continue;
        };
        message.content = vec![Content::Text { text }];
        bounded_messages.push(message);
    }
    *messages = bounded_messages;

    warn!(
        total_bytes,
        retained_bytes = MAX_DYNAMIC_CONTEXT_BYTES - remaining,
        max_bytes = MAX_DYNAMIC_CONTEXT_BYTES,
        truncated_sections = ?truncated,
        omitted_sections = ?omitted,
        "Dynamic request context exceeded its aggregate budget"
    );
}

fn context_message_text(message: &ModelMessage) -> Option<&str> {
    if message.role != Role::System || message.content.len() != 1 {
        return None;
    }
    match &message.content[0] {
        Content::Text { text } => Some(text),
        _ => None,
    }
}

fn dynamic_context_priority(text: &str) -> u8 {
    if text.starts_with("[PLAN MODE ACTIVE")
        || text.starts_with("[ACTIVE PLAN")
        || text.starts_with("[AUTONOMOUS TASKS]")
        || text.starts_with("[WORKSPACE MODE:")
        || text.starts_with("[ENVIRONMENT]")
    {
        120
    } else if text.starts_with("[PROJECT INSTRUCTIONS")
        || text.starts_with("[PROJECT SETTINGS]")
        || text.starts_with("[MAKO ")
    {
        100
    } else if text.starts_with("[AVAILABLE SKILLS]") || text.starts_with("[RECENT DELEGATED RUNS]")
    {
        80
    } else {
        60
    }
}

fn context_section_label(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("CONTEXT SECTION")
        .trim_matches(['[', ']'])
        .chars()
        .take(80)
        .collect()
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

/// Truncate a string to at most `max_bytes` without splitting a UTF-8 code
/// point. This is used for request-context budgets, which are byte based so a
/// multi-byte document cannot silently exceed the configured cap.
pub(super) fn truncate_utf8_bytes(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }

    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
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
