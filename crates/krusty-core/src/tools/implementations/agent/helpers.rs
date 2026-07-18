use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::agent::subagent::{AgentProgress, AgentProgressStatus, SubAgentResult};
use crate::agent::DelegatedRunStage;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::{Database, DelegatedRunRecord, DelegatedRunScope, DelegatedRunStore};
use crate::tools::registry::DelegationPolicy;
use crate::tools::{ToolContext, ToolResult};

/// Build the immediate response for a background agent launch.
pub(super) fn background_started_result(
    delegated_run_id: &str,
    agent_type: &str,
    name: Option<&str>,
) -> ToolResult {
    let mut result = json!({
        "status": "background_started",
        "delegated_run_id": delegated_run_id,
        "agent_type": agent_type,
        "message": format!(
            "{} agent started in background. Continue other work; after it completes, its result will appear in delegated context on a later turn. delegated_run_id: '{}'.",
            agent_type, delegated_run_id
        ),
    });
    if let Some(name) = name {
        result["name"] = json!(name);
        result["message"] = json!(format!(
            "Named agent '{}' ({}) started in background. Continue other work; after it completes, its result will appear in delegated context on a later turn. delegated_run_id: '{}'.",
            name, agent_type, delegated_run_id
        ));
    }
    ToolResult::success_data(result)
}
// ---------------------------------------------------------------------------
// Helper functions (ported from explore.rs and build.rs)
// ---------------------------------------------------------------------------

pub(super) struct SingleAgentArtifact {
    pub(super) payload: Value,
    pub(super) review_summary: String,
    pub(super) final_stage: DelegatedRunStage,
}

pub(super) fn build_single_agent_artifact(
    delegated_run_id: &str,
    result: &SubAgentResult,
    delegation_policy: &DelegationPolicy,
) -> SingleAgentArtifact {
    let agent = result.evidence_json();
    let review_summary = truncate_utf8(&result.output, 500).to_string();
    let payload = json!({
        "delegated_run_id": delegated_run_id,
        "findings": result.output,
        "files_examined": result.files_examined,
        "files_examined_count": result.files_examined.len(),
        "paths_examined": result.files_examined,
        "paths_examined_count": result.files_examined.len(),
        "duration_ms": result.duration_ms,
        "turns_used": result.turns_used,
        "success": result.success,
        "outcome": if result.success { "success" } else { "failed" },
        "outcome_reason": if result.success { "usable_evidence" } else { "api_or_tool_error" },
        "agent_count": 1,
        "successful_agents": if result.success { 1 } else { 0 },
        "usable_agents": if result.success { 1 } else { 0 },
        "degraded_agents": 0,
        "failed_agents": if result.success { 0 } else { 1 },
        "agents": [agent],
        "background_processes": result.background_processes,
        "delegation_policy": delegation_policy.audit_json(),
    });

    SingleAgentArtifact {
        payload,
        review_summary,
        final_stage: single_agent_final_stage(result.success),
    }
}

pub(super) fn single_agent_final_stage(success: bool) -> DelegatedRunStage {
    if success {
        DelegatedRunStage::Complete
    } else {
        DelegatedRunStage::Failed
    }
}

pub(super) fn persist_single_agent_artifact(
    store: &DelegatedRunStore,
    delegated_run_id: &str,
    artifact: &SingleAgentArtifact,
    resumable: bool,
    error_message: &str,
) {
    if let Err(err) = store.finalize_run(
        delegated_run_id,
        artifact.final_stage,
        &artifact.payload,
        Some(&artifact.review_summary),
        resumable,
    ) {
        warn!(delegated_run_id = %delegated_run_id, error = %err, "{}", error_message);
    }
}

pub(super) fn persist_single_agent_artifact_from_db_path(
    db_path: &Path,
    delegated_run_id: &str,
    artifact: &SingleAgentArtifact,
    resumable: bool,
    agent_type: &str,
) {
    match Database::new(db_path) {
        Ok(db) => {
            let store = DelegatedRunStore::new(db);
            let error_message = format!(
                "Background {}: failed to persist final artifact",
                agent_type
            );
            persist_single_agent_artifact(
                &store,
                delegated_run_id,
                artifact,
                resumable,
                &error_message,
            );
        }
        Err(err) => {
            tracing::error!(
                delegated_run_id = %delegated_run_id,
                error = %err,
                "Failed to open database for background {} finalization",
                agent_type
            );
        }
    }
}

pub(super) fn emit_single_agent_completion(
    progress_tx: &Option<mpsc::UnboundedSender<AgentProgress>>,
    delegated_run_id: &str,
    agent_type: &str,
    result: &SubAgentResult,
    review_summary: &str,
) {
    if let Some(tx) = progress_tx {
        let status = if result.success {
            AgentProgressStatus::Complete
        } else {
            AgentProgressStatus::Failed
        };
        if tx
            .send(AgentProgress {
                delegated_run_id: Some(delegated_run_id.to_string()),
                task_id: result.task_id.clone(),
                name: agent_type.to_string(),
                identity: None,
                status,
                tool_count: 0,
                tokens: 0,
                current_action: None,
                completion_summary: Some(review_summary.to_string()),
                lines_added: 0,
                lines_removed: 0,
                completed_plan_task: None,
            })
            .is_err()
        {
            debug!(
                "Background {} progress channel disconnected (parent returned)",
                agent_type
            );
        }
    }
}

pub(super) fn build_single_agent_warnings(result: &SubAgentResult, action: &str) -> Vec<String> {
    if result.success {
        return Vec::new();
    }

    match result.error.as_deref() {
        Some(err) => vec![format!("{} failed: {}", action, err)],
        None => vec![format!("{} completed without usable results.", action)],
    }
}

pub(super) fn concise_target_label(target: &str, index: usize) -> String {
    let trimmed = target.trim_matches('/');
    if trimmed.is_empty() {
        return format!("target-{}", index + 1);
    }

    let parts = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    if parts.len() >= 2 {
        return format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]);
    }

    parts[0].to_string()
}

pub(super) fn resolve_explore_target(
    target: &str,
    project_dir: &Path,
    kind: &str,
) -> Result<PathBuf, String> {
    let candidate = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        project_dir.join(target)
    };

    let canonical = candidate.canonicalize().map_err(|_| {
        format!(
            "Missing explore target: {} '{}' does not exist under project root",
            kind, target
        )
    })?;

    let project_root = project_dir
        .canonicalize()
        .map_err(|err| format!("Invalid project root '{}': {}", project_dir.display(), err))?;

    if !canonical.starts_with(&project_root) {
        return Err(format!(
            "Invalid explore target: {} '{}' is outside project root",
            kind, target
        ));
    }

    match kind {
        "directory" if !canonical.is_dir() => Err(format!(
            "Invalid explore target: directory '{}' resolved to a non-directory path",
            target
        )),
        "file" if !canonical.is_file() => Err(format!(
            "Invalid explore target: file '{}' resolved to a non-file path",
            target
        )),
        _ => Ok(canonical),
    }
}

pub(super) fn relative_label(path: &Path, project_dir: &Path) -> String {
    path.strip_prefix(project_dir)
        .unwrap_or(path)
        .display()
        .to_string()
}

pub(super) fn delegated_scope(
    label: &str,
    path: &Path,
    kind: &str,
    project_dir: &Path,
) -> DelegatedRunScope {
    DelegatedRunScope {
        label: label.to_string(),
        path: relative_label(path, project_dir),
        kind: kind.to_string(),
    }
}

pub(super) fn open_delegated_run_store(ctx: &ToolContext) -> Option<DelegatedRunStore> {
    let db_path = ctx.db_path.as_ref()?;
    match Database::new(db_path) {
        Ok(db) => Some(DelegatedRunStore::new(db)),
        Err(error) => {
            warn!(
                session_id = ?ctx.session_id,
                tool_use_id = ?ctx.tool_use_id,
                db_path = %db_path.display(),
                error = %error,
                "Failed to open delegated run store"
            );
            None
        }
    }
}

pub(super) fn build_resume_seed(
    previous: &DelegatedRunRecord,
    target_label: &str,
) -> Option<String> {
    let artifact = previous.artifact.as_ref()?;
    let agent_artifact = artifact
        .get("agents")
        .and_then(|value| value.as_array())
        .and_then(|agents| {
            agents.iter().find(|entry| {
                entry
                    .get("agent")
                    .and_then(|value| value.as_str())
                    .is_some_and(|agent| agent == target_label)
            })
        });

    let summary = agent_artifact
        .and_then(|entry| entry.get("summary").and_then(|value| value.as_str()))
        .or(previous.human_review.as_deref())
        .or_else(|| {
            artifact
                .get("investigation_summary")
                .and_then(|value| value.as_str())
        })?;

    let paths = agent_artifact
        .and_then(|entry| {
            entry
                .get("paths_examined")
                .and_then(|value| value.as_array())
        })
        .map(|paths| {
            paths
                .iter()
                .filter_map(|value| value.as_str())
                .take(8)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|value| !value.trim().is_empty());

    let gaps = agent_artifact
        .and_then(|entry| entry.get("gaps").and_then(|value| value.as_array()))
        .map(|gaps| {
            gaps.iter()
                .filter_map(|value| value.as_str())
                .take(4)
                .collect::<Vec<_>>()
                .join("; ")
        })
        .filter(|value| !value.trim().is_empty());

    let mut lines = vec![format!(
        "Previous delegated run {} already covered this target.",
        previous.delegated_run_id
    )];
    lines.push(format!(
        "Reuse and extend this prior evidence: {}",
        summary.trim()
    ));
    if let Some(paths) = paths {
        lines.push(format!("Previously examined paths: {}", paths));
    }
    if let Some(gaps) = gaps {
        lines.push(format!("Previously identified gaps: {}", gaps));
    }
    lines.push(
        "Do not remap the same directory structure from scratch unless it is required to close a concrete gap."
            .to_string(),
    );
    Some(lines.join("\n"))
}

/// Build a brief summary of the parent conversation for context injection.
///
/// Extracts the last `max_turns` user/assistant messages and formats them as a
/// `[PARENT CONTEXT]` block that the sub-agent can use for upstream awareness.
pub(super) fn build_parent_context_brief(
    conversation: &[ModelMessage],
    max_turns: usize,
) -> String {
    let relevant: Vec<_> = conversation
        .iter()
        .filter(|msg| msg.role == Role::User || msg.role == Role::Assistant)
        .collect();

    let start = relevant.len().saturating_sub(max_turns);
    let window = &relevant[start..];

    if window.is_empty() {
        return String::new();
    }

    let mut lines = vec!["[PARENT CONTEXT]".to_string()];
    for msg in window {
        let role_label = match msg.role {
            Role::User => "User",
            Role::Assistant => "Assistant",
            _ => continue,
        };

        let text = msg
            .content
            .iter()
            .filter_map(|c| match c {
                Content::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");

        if text.is_empty() {
            continue;
        }

        let truncated = if text.len() > 200 {
            let mut end = 200;
            while end > 0 && !text.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}...", &text[..end])
        } else {
            text
        };

        lines.push(format!("{}: {}", role_label, truncated));
    }
    lines.push("[/PARENT CONTEXT]".to_string());

    lines.join("\n")
}

// Build-specific helper functions

pub(super) fn classify_build_outcome(
    results_len: usize,
    failed_agents: usize,
    modified_files: usize,
) -> &'static str {
    if results_len == 0 || modified_files == 0 {
        return "failed";
    }
    if failed_agents == 0 {
        return "success";
    }
    "partial"
}

pub(super) fn build_confidence(failed_agents: usize, modified_files: usize) -> &'static str {
    if modified_files == 0 {
        return "low";
    }
    if failed_agents == 0 {
        return "high";
    }
    "medium"
}

pub(super) fn build_investigation_summary(
    successful_builders: usize,
    failed_builders: usize,
    modified_files: usize,
    lines_added: usize,
    lines_removed: usize,
) -> String {
    let mut parts = vec![format!(
        "Delegated build completed with {} successful builders across {} modified files.",
        successful_builders, modified_files
    )];
    parts.push(format!(
        "Net code movement: +{} / -{} lines.",
        lines_added, lines_removed
    ));
    if failed_builders > 0 {
        parts.push(format!(
            "{} builders reported errors and should be reviewed before accepting the build output.",
            failed_builders
        ));
    }
    parts.join(" ")
}

pub(super) fn build_coverage_gap_notice(errors: &[String]) -> Option<String> {
    if errors.is_empty() {
        None
    } else {
        Some(format!(
            "These builders failed or reported issues: {}",
            errors.join(" | ")
        ))
    }
}

/// Truncate a string to at most `max_bytes` bytes without splitting a UTF-8
/// character. Returns the longest prefix that fits within the byte budget.
pub(super) fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
