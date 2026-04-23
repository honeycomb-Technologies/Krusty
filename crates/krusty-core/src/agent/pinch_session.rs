use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde_json::json;

use crate::ai::client::AiClient;
use crate::ai::types::{Content, ModelMessage};
use crate::plan::{PlanFile, PlanManager};
use crate::storage::{Database, FileActivityTracker, RankedFile, SessionManager};

use super::build_project_context;
use super::pinch_context::{PinchContext, PinchContextInput};
use super::summarizer::{generate_summary, SummarizationResult};

pub const PINCH_RANKED_FILE_LIMIT: usize = 20;
pub const PINCH_SUMMARY_FILE_CONTENT_LIMIT: usize = 10;
pub const PINCH_CONTEXT_FILE_CONTENT_LIMIT: usize = 5;

pub struct CreatePinchedSessionRequest<'a> {
    pub db_path: &'a Path,
    pub ai_client: Option<&'a AiClient>,
    pub session_id: &'a str,
    pub source_session_title: &'a str,
    pub conversation: &'a [ModelMessage],
    pub working_dir: &'a Path,
    pub model: Option<&'a str>,
    pub target_branch: Option<&'a str>,
    pub preservation_hints: Option<String>,
    pub direction: Option<String>,
    pub initial_user_message: Option<String>,
}

pub struct CreatePinchedSessionResult {
    pub new_session_id: String,
    pub summary: SummarizationResult,
    pub pinch_context: PinchContext,
}

pub async fn create_pinched_session(
    request: CreatePinchedSessionRequest<'_>,
) -> Result<CreatePinchedSessionResult> {
    if request.conversation.is_empty() {
        bail!("Cannot pinch session with no messages");
    }

    let db = Database::new(request.db_path)?;
    let session_manager = SessionManager::new(db);
    let ranked_files = ranked_files_for_pinch(&session_manager, request.session_id);
    let file_contents = load_key_file_contents(
        request.session_id,
        request.working_dir,
        &ranked_files,
        PINCH_SUMMARY_FILE_CONTENT_LIMIT,
    );
    let project_context = load_project_context(request.working_dir);
    let active_plan = load_active_plan(request.db_path, request.session_id);
    let summary = summarize_pinch(
        request.ai_client,
        request.conversation,
        request.preservation_hints.as_deref(),
        &ranked_files,
        &file_contents,
        project_context.as_deref(),
        request.model,
    )
    .await;
    let active_plan_markdown = active_plan.as_ref().map(PlanFile::to_markdown);
    let key_file_contents = file_contents
        .iter()
        .take(PINCH_CONTEXT_FILE_CONTENT_LIMIT)
        .cloned()
        .collect();
    let pinch_context = PinchContext::from_input(PinchContextInput {
        source_session_id: request.session_id.to_string(),
        source_session_title: request.source_session_title.to_string(),
        summary: summary.clone(),
        ranked_files,
        preservation_hints: request.preservation_hints,
        direction: request.direction,
        project_context,
        key_file_contents,
        active_plan: active_plan_markdown,
    });

    let new_title = format!("{} (continued)", request.source_session_title);
    let working_dir_for_child = request.working_dir.to_string_lossy().to_string();
    let new_session_id = session_manager.create_linked_session(
        &new_title,
        request.session_id,
        &pinch_context,
        request.model,
        Some(working_dir_for_child.as_str()),
        request.target_branch,
    )?;

    let system_msg_json =
        json!([{ "type": "text", "text": pinch_context.to_system_message() }]).to_string();
    session_manager.save_message(&new_session_id, "system", &system_msg_json)?;

    if let Some(plan) = active_plan.as_ref() {
        if let Err(error) = PlanManager::new(request.db_path.to_path_buf())
            .and_then(|pm| pm.save_plan_for_session(&new_session_id, plan))
        {
            tracing::warn!(
                source_session_id = %request.session_id,
                new_session_id = %new_session_id,
                error = %error,
                "Failed to carry active plan into pinched session"
            );
        }
    }

    if let Some(message) = request
        .initial_user_message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty())
    {
        let content_json = serde_json::to_string(&vec![Content::Text {
            text: message.to_string(),
        }])?;
        session_manager.save_message(&new_session_id, "user", &content_json)?;
    }

    Ok(CreatePinchedSessionResult {
        new_session_id,
        summary,
        pinch_context,
    })
}

async fn summarize_pinch(
    ai_client: Option<&AiClient>,
    conversation: &[ModelMessage],
    preservation_hints: Option<&str>,
    ranked_files: &[RankedFile],
    file_contents: &[(String, String)],
    project_context: Option<&str>,
    model: Option<&str>,
) -> SummarizationResult {
    if let Some(ai_client) = ai_client {
        generate_summary(
            ai_client,
            conversation,
            preservation_hints,
            ranked_files,
            file_contents,
            project_context,
            model,
        )
        .await
        .unwrap_or_else(|error| {
            tracing::warn!("Summarization failed, using defaults: {}", error);
            SummarizationResult::default()
        })
    } else {
        SummarizationResult::default()
    }
}

fn ranked_files_for_pinch(session_manager: &SessionManager, session_id: &str) -> Vec<RankedFile> {
    match FileActivityTracker::new(session_manager.db(), session_id.to_string())
        .get_ranked_files(PINCH_RANKED_FILE_LIMIT)
    {
        Ok(ranked_files) => ranked_files,
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "Failed to load ranked files for pinch context"
            );
            Vec::new()
        }
    }
}

fn load_project_context(working_dir: &Path) -> Option<String> {
    let context = build_project_context(working_dir);
    (!context.trim().is_empty()).then_some(context)
}

fn load_active_plan(db_path: &Path, session_id: &str) -> Option<PlanFile> {
    match PlanManager::new(db_path.to_path_buf()).and_then(|pm| pm.get_active_plan(session_id)) {
        Ok(plan) => plan,
        Err(error) => {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "Failed to load active plan for pinch context"
            );
            None
        }
    }
}

fn load_key_file_contents(
    session_id: &str,
    working_dir: &Path,
    ranked_files: &[RankedFile],
    limit: usize,
) -> Vec<(String, String)> {
    ranked_files
        .iter()
        .take(limit)
        .filter_map(|file| {
            let path = if Path::new(&file.path).is_absolute() {
                PathBuf::from(&file.path)
            } else {
                working_dir.join(&file.path)
            };

            match std::fs::read_to_string(&path) {
                Ok(content) => Some((file.path.clone(), content)),
                Err(error) => {
                    tracing::warn!(
                        session_id = %session_id,
                        path = %path.display(),
                        error = %error,
                        "Failed to load key file content for pinch context"
                    );
                    None
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::{create_pinched_session, CreatePinchedSessionRequest};
    use crate::ai::types::{Content, ModelMessage, Role};
    use crate::plan::PlanManager;
    use crate::storage::{Database, SessionManager};

    #[tokio::test]
    async fn create_pinched_session_preserves_system_context_and_plan() {
        let temp = TempDir::new().expect("temp dir");
        let db_path = temp.path().join("pinch.db");
        let session_manager = SessionManager::new(Database::new(&db_path).expect("db"));
        let session_id = session_manager
            .create_session(
                "Pinch Source",
                Some("test-model"),
                Some(temp.path().to_str().expect("path string")),
            )
            .expect("source session");
        std::fs::write(temp.path().join("main.rs"), "fn main() {}\n").expect("file");
        session_manager
            .save_message(
                &session_id,
                "user",
                &serde_json::to_string(&vec![Content::Text {
                    text: "Keep working on the refactor.".to_string(),
                }])
                .expect("user json"),
            )
            .expect("save source message");

        let plan_manager = PlanManager::new(db_path.clone()).expect("plan manager");
        let plan = plan_manager
            .create_plan(
                "Refactor",
                &session_id,
                Some(temp.path().to_str().expect("path string")),
            )
            .expect("plan");

        let result = create_pinched_session(CreatePinchedSessionRequest {
            db_path: &db_path,
            ai_client: None,
            session_id: &session_id,
            source_session_title: "Pinch Source",
            conversation: &[ModelMessage {
                role: Role::User,
                content: vec![Content::Text {
                    text: "Keep working on the refactor.".to_string(),
                }],
            }],
            working_dir: temp.path(),
            model: Some("test-model"),
            target_branch: Some("feature/refactor"),
            preservation_hints: Some("Preserve the route split.".to_string()),
            direction: Some("Continue modularity work.".to_string()),
            initial_user_message: Some("Continue.".to_string()),
        })
        .await
        .expect("pinch should succeed");

        let child = session_manager
            .get_session(&result.new_session_id)
            .expect("load child")
            .expect("child exists");
        assert_eq!(
            child.parent_session_id.as_deref(),
            Some(session_id.as_str())
        );

        let messages = session_manager
            .load_session_messages(&result.new_session_id)
            .expect("load child messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].0, "system");
        assert_eq!(messages[1].0, "user");
        assert!(messages[0].1.contains("Pinch - CONTINUATION SESSION"));

        let child_plan = plan_manager
            .get_active_plan(&result.new_session_id)
            .expect("load child plan")
            .expect("child plan");
        assert_eq!(child_plan.title, plan.title);
        assert_eq!(result.summary.work_summary, "No summary available.");
    }
}
