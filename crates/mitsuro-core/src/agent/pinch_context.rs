//! Pinch context for session transitions
//!
//! When context approaches limits, creates a structured context
//! to a new session with preserved context and user direction.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::summarizer::SummarizationResult;
use crate::storage::RankedFile;

const PROJECT_CONTEXT_MAX_BYTES: usize = 8000;
const PROJECT_CONTEXT_SECTION_MARKER: &str = "[PROJECT INSTRUCTIONS - ";
const PROJECT_CONTEXT_OMISSION_NOTICE: &str =
    "[...earlier project instructions omitted for context limits]\n\n";

/// Complete pinch context for injection into new session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinchContext {
    /// Source session UUID
    pub source_session_id: String,
    /// Source session title for reference
    pub source_session_title: String,
    /// High-level summary of work accomplished
    pub work_summary: String,
    /// Key architectural/design decisions made
    pub key_decisions: Vec<String>,
    /// Incomplete tasks or next steps
    pub pending_tasks: Vec<String>,
    /// Files ranked by importance
    pub ranked_files: Vec<RankedFileInfo>,
    /// User's hints about what to preserve (stage 1 input)
    pub preservation_hints: Option<String>,
    /// User's direction for next phase (stage 2 input)
    pub direction: Option<String>,
    /// When pinch was created
    pub created_at: DateTime<Utc>,
    /// CLAUDE.md / KRAB.md project context
    pub project_context: Option<String>,
    /// Key file contents (path, content) for immediate reference
    pub key_file_contents: Vec<(String, String)>,
    /// Active plan content (if any) - full markdown of the plan
    pub active_plan: Option<String>,
}

/// Serializable version of RankedFile for storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedFileInfo {
    pub path: String,
    pub score: f64,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PinchContextInput {
    pub source_session_id: String,
    pub source_session_title: String,
    pub summary: SummarizationResult,
    pub ranked_files: Vec<RankedFile>,
    pub preservation_hints: Option<String>,
    pub direction: Option<String>,
    pub project_context: Option<String>,
    pub key_file_contents: Vec<(String, String)>,
    pub active_plan: Option<String>,
}

impl From<RankedFile> for RankedFileInfo {
    fn from(rf: RankedFile) -> Self {
        Self {
            path: rf.path,
            score: rf.score,
            reasons: rf.reasons,
        }
    }
}

/// Safely truncate a string to at most `max_bytes` bytes on a valid UTF-8 char boundary.
fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn split_project_context_sections(ctx: &str) -> Vec<&str> {
    let starts: Vec<_> = ctx
        .match_indices(PROJECT_CONTEXT_SECTION_MARKER)
        .map(|(idx, _)| idx)
        .collect();
    if starts.len() <= 1 {
        return vec![ctx];
    }

    starts
        .iter()
        .enumerate()
        .map(|(i, start)| {
            let end = starts.get(i + 1).copied().unwrap_or(ctx.len());
            ctx[*start..end].trim()
        })
        .collect()
}

fn truncate_project_context_for_pinch(ctx: &str) -> (String, bool) {
    if ctx.len() <= PROJECT_CONTEXT_MAX_BYTES {
        return (ctx.to_string(), false);
    }

    let sections = split_project_context_sections(ctx);
    if sections.len() <= 1 {
        return (
            truncate_utf8(ctx, PROJECT_CONTEXT_MAX_BYTES).to_string(),
            true,
        );
    }

    let budget = PROJECT_CONTEXT_MAX_BYTES.saturating_sub(PROJECT_CONTEXT_OMISSION_NOTICE.len());
    let mut selected = Vec::new();
    let mut used = 0usize;

    for section in sections.iter().rev() {
        let separator_len = if selected.is_empty() { 0 } else { 2 };
        let section_len = section.len();
        if used + separator_len + section_len > budget {
            continue;
        }
        selected.push(*section);
        used += separator_len + section_len;
    }

    if selected.is_empty() {
        selected.push(truncate_utf8(
            sections.last().copied().unwrap_or(ctx),
            budget.max(1),
        ));
    }

    selected.reverse();
    (
        format!(
            "{}{}",
            PROJECT_CONTEXT_OMISSION_NOTICE,
            selected.join("\n\n")
        ),
        true,
    )
}

impl PinchContext {
    pub fn from_input(input: PinchContextInput) -> Self {
        Self {
            source_session_id: input.source_session_id,
            source_session_title: input.source_session_title,
            work_summary: input.summary.work_summary,
            key_decisions: input.summary.key_decisions,
            pending_tasks: input.summary.pending_tasks,
            ranked_files: input.ranked_files.into_iter().map(Into::into).collect(),
            preservation_hints: input.preservation_hints,
            direction: input.direction,
            created_at: Utc::now(),
            project_context: input.project_context,
            key_file_contents: input.key_file_contents,
            active_plan: input.active_plan,
        }
    }

    /// Format as system message for new session
    ///
    /// Creates a structured markdown document that provides
    /// context for the continued conversation.
    pub fn to_system_message(&self) -> String {
        let mut msg = String::new();

        // Directive header - tell Claude to USE this context
        msg.push_str("# Pinch - CONTINUATION SESSION\n\n");
        msg.push_str("> **IMPORTANT**: This is a continuation of previous work. ");
        msg.push_str("The context below represents completed analysis and decisions. ");
        msg.push_str("Do NOT re-search or re-discover what is already documented here. ");
        msg.push_str("Start from this context and continue the work.\n\n");

        msg.push_str(&format!(
            "Continuing from session: **{}**\n\n",
            self.source_session_title
        ));

        // User direction (highest priority - put first)
        if let Some(direction) = &self.direction {
            msg.push_str("## Priority Direction\n\n");
            msg.push_str(&format!("**User requested focus**: {}\n\n", direction));
        }

        // Work summary
        msg.push_str("## Summary of Previous Work\n\n");
        msg.push_str(&self.work_summary);
        msg.push_str("\n\n");

        // Key decisions
        if !self.key_decisions.is_empty() {
            msg.push_str("## Key Decisions Made\n\n");
            for decision in &self.key_decisions {
                msg.push_str(&format!("- {}\n", decision));
            }
            msg.push('\n');
        }

        // Pending tasks - make these actionable
        if !self.pending_tasks.is_empty() {
            msg.push_str("## Pending/Incomplete Work (Continue From Here)\n\n");
            msg.push_str("These tasks were identified as next steps:\n\n");
            for (i, task) in self.pending_tasks.iter().enumerate() {
                msg.push_str(&format!("{}. {}\n", i + 1, task));
            }
            msg.push('\n');
        }

        // Key files list (top 10)
        if !self.ranked_files.is_empty() {
            msg.push_str("## Key Files (by importance)\n\n");
            for (i, file) in self.ranked_files.iter().take(10).enumerate() {
                let reasons = if file.reasons.is_empty() {
                    String::new()
                } else {
                    format!(" - {}", file.reasons.join(", "))
                };
                msg.push_str(&format!("{}. `{}`{}\n", i + 1, file.path, reasons));
            }
            msg.push('\n');
        }

        // Preservation hints (if any)
        if let Some(hints) = &self.preservation_hints {
            msg.push_str("## Preservation Notes\n\n");
            msg.push_str(&format!("User emphasized: {}\n\n", hints));
        }

        msg.push_str("---\n\n");

        // PROJECT CONTEXT - Critical for continuation!
        if let Some(ctx) = &self.project_context {
            msg.push_str("## Project Instructions\n\n");
            msg.push_str("Follow these project rules and guidelines:\n\n");
            let (project_context, truncated) = truncate_project_context_for_pinch(ctx);
            msg.push_str(&project_context);
            if truncated {
                msg.push_str("\n\n...[truncated for context limits]\n");
            }
            msg.push_str("\n\n---\n\n");
        }

        // KEY FILE CONTENTS - So Claude doesn't start blind
        if !self.key_file_contents.is_empty() {
            msg.push_str("## Key File Contents (Pre-loaded)\n\n");
            msg.push_str("These files are critical for continuing the work:\n\n");
            for (path, content) in self.key_file_contents.iter().take(5) {
                msg.push_str(&format!("### `{}`\n\n```\n", path));
                // Truncate very long files
                if content.len() > 4000 {
                    msg.push_str(truncate_utf8(content, 4000));
                    msg.push_str("\n...[truncated]\n");
                } else {
                    msg.push_str(content);
                }
                msg.push_str("\n```\n\n");
            }
            msg.push_str("---\n\n");
        }

        // ACTIVE PLAN - If user has a plan in progress
        if let Some(plan) = &self.active_plan {
            msg.push_str("## Active Plan\n\n");
            msg.push_str(
                "There is an active plan in progress. Continue from where you left off:\n\n",
            );
            // Truncate if very long
            if plan.len() > 6000 {
                msg.push_str(truncate_utf8(plan, 6000));
                msg.push_str("\n\n...[plan truncated]\n");
            } else {
                msg.push_str(plan);
            }
            msg.push_str("\n\n---\n\n");
        }

        // Action instruction
        msg.push_str("## How to Proceed\n\n");
        if self.direction.is_some() {
            msg.push_str("1. Focus on the **Priority Direction** above\n");
            msg.push_str("2. Reference the pre-loaded file contents above\n");
            msg.push_str("3. Build on the Key Decisions already made\n");
            msg.push_str("4. Read additional files as needed using the Key Files list\n");
        } else if !self.pending_tasks.is_empty() {
            msg.push_str("1. Start with the first **Pending Task** above\n");
            msg.push_str("2. Reference the pre-loaded file contents above\n");
            msg.push_str("3. Build on the Key Decisions already made\n");
            msg.push_str("4. Read additional files as needed using the Key Files list\n");
        } else {
            msg.push_str("Ask the user what they'd like to work on next.\n");
        }

        msg.push_str(&format!(
            "\n*Pinch created: {}*\n",
            self.created_at.format("%Y-%m-%d %H:%M UTC")
        ));

        msg
    }
}

#[cfg(test)]
mod tests {
    use super::{PinchContext, PinchContextInput};
    use crate::agent::summarizer::SummarizationResult;

    #[test]
    fn to_system_message_keeps_closest_project_instructions_when_truncated() {
        let root_section = format!(
            "[PROJECT INSTRUCTIONS - /repo/KRAB.md]\n\n{}\n\n[END PROJECT INSTRUCTIONS]",
            "root instructions\n".repeat(700)
        );
        let local_section = "[PROJECT INSTRUCTIONS - app/AGENTS.md]\n\nlocal instructions\n\n[END PROJECT INSTRUCTIONS]";
        let pinch_ctx = PinchContext::from_input(PinchContextInput {
            source_session_id: "source-session".to_string(),
            source_session_title: "Source Session".to_string(),
            summary: SummarizationResult::default(),
            ranked_files: vec![],
            preservation_hints: None,
            direction: None,
            project_context: Some(format!("{root_section}\n\n{local_section}")),
            key_file_contents: vec![],
            active_plan: None,
        });

        let message = pinch_ctx.to_system_message();

        assert!(message.contains("## Project Instructions"));
        assert!(message.contains("local instructions"));
        assert!(message.contains("app/AGENTS.md"));
        assert!(message.contains("omitted for context limits"));
    }
}
