use std::path::Path;
use std::process::Command;

use tracing::warn;

use crate::storage::ProjectSettings;

use super::project::build_project_context;

/// Build combined workspace + project context for a subagent system prompt.
pub fn build_subagent_project_context(working_dir: &Path, project_dir: Option<&Path>) -> String {
    let workspace = build_workspace_context(working_dir, project_dir);
    let project = project_dir.map(build_project_context).unwrap_or_default();
    let project_settings = project_dir.map(ProjectSettings::load).unwrap_or_default();

    let mut ctx = String::new();
    if !workspace.is_empty() {
        ctx.push_str(&workspace);
    }
    if !project.is_empty() {
        if !ctx.is_empty() {
            ctx.push_str("\n\n");
        }
        ctx.push_str(&project);
    }
    if let Some(ref append) = project_settings.system_prompt_append {
        if !append.is_empty() {
            if !ctx.is_empty() {
                ctx.push_str("\n\n");
            }
            ctx.push_str(&format!("[PROJECT SETTINGS]\n{}", append));
        }
    }
    ctx
}

pub(super) fn build_workspace_context(working_dir: &Path, project_dir: Option<&Path>) -> String {
    let execution_dir = working_dir.display();

    if let Some(project_dir) = project_dir {
        return format!(
            "[WORKSPACE MODE: PROJECT]\n\n\
             Execution directory: {}\n\
             Project directory: {}\n\
             - Treat the project directory above as the canonical repository root for this session\n\
             - Prefer absolute paths rooted in that project directory when referring to files\n\
             - Do not invent alternate workspace roots or mirror paths if tools already revealed the real filesystem layout",
            execution_dir,
            project_dir.display()
        );
    }

    format!(
        "[WORKSPACE MODE: NEUTRAL]\n\n\
     Execution directory: {}\n\
     Project directory: none selected\n\n\
     No project directory is currently selected for this session.\n\
     - Do not assume the user wants help with any repository just because files are accessible\n\
     - Do not describe yourself as working on a named project unless the user explicitly selects or creates one\n\
     - Use the execution directory above when running tools, and treat paths returned by tools as canonical\n\
     - Do not invent alternate workspace roots, mount points, or mirrored directories when tools already returned absolute paths\n\
     - Read-only system inspection, brainstorming, and machine-level tasks are valid in this mode\n\
     - If the user chooses or creates a project directory, call `set_workspace_context` so future turns pivot into project-aware behavior",
        execution_dir
    )
}

fn run_git_context_command(working_dir: &Path, args: &[&str]) -> Option<std::process::Output> {
    match Command::new("git")
        .args(args)
        .current_dir(working_dir)
        .output()
    {
        Ok(output) if output.status.success() => Some(output),
        Ok(output) => {
            warn!(
                working_dir = %working_dir.display(),
                ?args,
                status = ?output.status,
                "Git probe failed while building environment context"
            );
            None
        }
        Err(error) => {
            warn!(
                working_dir = %working_dir.display(),
                ?args,
                error = %error,
                "Failed to run git probe while building environment context"
            );
            None
        }
    }
}

pub(super) fn summarize_git_status(status_text: &str) -> Option<String> {
    let mut modified = 0usize;
    let mut staged = 0usize;
    let mut untracked = 0usize;

    for line in status_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("??") {
            untracked += 1;
        } else if trimmed.starts_with('A') {
            staged += 1;
        } else if trimmed.starts_with('M') || trimmed.contains('M') {
            modified += 1;
        }
    }

    let mut parts = Vec::new();
    if modified > 0 {
        parts.push(format!("{} modified", modified));
    }
    if staged > 0 {
        parts.push(format!("{} staged", staged));
    }
    if untracked > 0 {
        parts.push(format!("{} untracked", untracked));
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(", "))
    }
}

/// Build environment context with runtime information.
///
/// Gathers working directory, git status, platform, shell, date, and model
/// information. Git commands that fail are skipped with warnings.
pub(super) fn build_environment_context(working_dir: &Path, model_id: Option<&str>) -> String {
    let mut lines = vec![
        "[ENVIRONMENT]".to_string(),
        format!("Working directory: {}", working_dir.display()),
    ];

    let is_git_repo = working_dir.join(".git").exists();
    lines.push(format!(
        "Git repository: {}",
        if is_git_repo { "yes" } else { "no" }
    ));

    if is_git_repo {
        if let Some(output) =
            run_git_context_command(working_dir, &["rev-parse", "--abbrev-ref", "HEAD"])
        {
            let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !branch.is_empty() {
                lines.push(format!("Git branch: {}", branch));
            }
        }

        if let Some(output) = run_git_context_command(working_dir, &["status", "--short"]) {
            let status_text = String::from_utf8_lossy(&output.stdout);
            if let Some(summary) = summarize_git_status(&status_text) {
                lines.push(format!("Git status: {}", summary));
            }
        }
    }

    lines.push(format!("Platform: {}", std::env::consts::OS));

    if let Ok(shell) = std::env::var("SHELL") {
        let shell_name = shell.rsplit('/').next().unwrap_or(&shell);
        lines.push(format!("Shell: {}", shell_name));
    }

    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    lines.push(format!("Date: {}", date));

    if let Some(model) = model_id {
        lines.push(format!("Model: {}", model));
    }

    lines.join("\n")
}
