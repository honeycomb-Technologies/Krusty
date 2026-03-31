//! Sub-agent types and data structures
//!
//! Core types for sub-agent configuration, progress tracking, and results.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Duration;

use crate::ai::retry::is_retryable_status;
use crate::ai::retry::IsRetryable;
use crate::tools::registry::DelegationPolicy;

/// Error type for subagent API calls that supports retry logic
#[derive(Debug)]
pub struct SubAgentApiError {
    pub message: String,
    pub status: Option<u16>,
    pub retry_after: Option<Duration>,
}

impl std::fmt::Display for SubAgentApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(status) = self.status {
            write!(f, "HTTP {}: {}", status, self.message)
        } else {
            write!(f, "{}", self.message)
        }
    }
}

impl std::error::Error for SubAgentApiError {}

impl IsRetryable for SubAgentApiError {
    fn is_retryable(&self) -> bool {
        match self.status {
            Some(status) => is_retryable_status(status),
            // Network errors without status codes are typically retryable
            None => {
                self.message.contains("timeout")
                    || self.message.contains("connection")
                    || self.message.contains("network")
            }
        }
    }

    fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }
}

impl From<anyhow::Error> for SubAgentApiError {
    fn from(err: anyhow::Error) -> Self {
        let message = err.to_string();
        // Try to extract HTTP status from error message
        let status = extract_status_from_error(&message);
        Self {
            message,
            status,
            retry_after: None,
        }
    }
}

/// Try to extract HTTP status code from error message
pub fn extract_status_from_error(message: &str) -> Option<u16> {
    // Common patterns: "HTTP 429", "status: 429", "status code: 429"
    for pattern in &["HTTP ", "status: ", "status code: "] {
        if let Some(pos) = message.find(pattern) {
            let start = pos + pattern.len();
            let code_str: String = message[start..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(code) = code_str.parse() {
                return Some(code);
            }
        }
    }
    None
}

/// Real-time progress update from a sub-agent
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentProgress {
    /// First-class delegated runtime unit identifier.
    pub delegated_run_id: Option<String>,
    /// Agent task ID
    pub task_id: String,
    /// Display name (derived from task context)
    pub name: String,
    /// Current status
    pub status: AgentProgressStatus,
    /// Number of tool calls made
    pub tool_count: usize,
    /// Approximate token usage
    pub tokens: usize,
    /// Current action description (e.g., "reading app.rs")
    pub current_action: Option<String>,
    /// Short completion summary when the sub-agent finishes a delegated task.
    pub completion_summary: Option<String>,
    /// Lines added (for build agents)
    pub lines_added: usize,
    /// Lines removed (for build agents)
    pub lines_removed: usize,
    /// Plan task ID completed (for auto-marking tasks)
    pub completed_plan_task: Option<String>,
}

/// Status of a sub-agent
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AgentProgressStatus {
    /// Agent is running
    #[default]
    Running,
    /// Agent completed successfully
    Complete,
    /// Agent failed
    Failed,
}

/// Configuration for a sub-agent task
///
/// The model to use is determined by `SubAgentPool.override_model`, not by the task.
/// This provides a provider-agnostic experience where all sub-agents use the user's
/// current model.
#[derive(Debug, Clone)]
pub struct SubAgentTask {
    pub id: String,
    /// Display name for the agent (e.g., "tui", "agent", "main")
    pub name: String,
    pub prompt: String,
    pub working_dir: PathBuf,
    /// Stable delegated runtime unit identifier shared by all tasks in one parent invocation.
    pub delegated_run_id: Option<String>,
    /// Plan task ID this agent completes (for auto-marking)
    pub plan_task_id: Option<String>,
    /// Whether thinking/reasoning is enabled for this agent
    pub thinking_enabled: bool,
    /// Inherited delegated execution policy from parent tool context.
    pub delegation_policy: Option<DelegationPolicy>,
    /// Optional per-task turn budget inherited from parent.
    pub max_turns_override: Option<usize>,
}

impl SubAgentTask {
    pub fn new(id: impl Into<String>, prompt: impl Into<String>) -> Self {
        let id = id.into();
        let name = id.clone(); // Default name is same as id
        Self {
            id,
            name,
            prompt: prompt.into(),
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            delegated_run_id: None,
            plan_task_id: None,
            thinking_enabled: false, // Default off for sub-agents
            delegation_policy: None,
            max_turns_override: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = dir;
        self
    }

    pub fn with_delegated_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.delegated_run_id = Some(run_id.into());
        self
    }

    pub fn with_plan_task_id(mut self, task_id: impl Into<String>) -> Self {
        self.plan_task_id = Some(task_id.into());
        self
    }

    pub fn with_thinking(mut self, enabled: bool) -> Self {
        self.thinking_enabled = enabled;
        self
    }

    pub fn with_delegation_policy(mut self, policy: DelegationPolicy) -> Self {
        self.delegation_policy = Some(policy);
        self
    }

    pub fn with_max_turns(mut self, max_turns: usize) -> Self {
        self.max_turns_override = Some(max_turns);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreReport {
    pub summary: String,
    #[serde(default)]
    pub module_structure: Option<String>,
    #[serde(default)]
    pub structural_coverage: Option<String>,
    #[serde(default)]
    pub semantic_coverage: Option<String>,
    #[serde(default)]
    pub responsibilities: Vec<String>,
    #[serde(default)]
    pub key_apis: Vec<String>,
    #[serde(default)]
    pub integration_points: Vec<String>,
    #[serde(default)]
    pub strengths: Vec<String>,
    #[serde(default)]
    pub gaps: Vec<String>,
    #[serde(default)]
    pub paths_examined: Vec<String>,
    #[serde(default)]
    pub files_examined: Vec<String>,
    #[serde(default)]
    pub key_findings: Vec<String>,
    #[serde(default)]
    pub design_patterns: Vec<String>,
    #[serde(default)]
    pub concerns: Vec<String>,
    #[serde(default)]
    pub confidence: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreEvidenceArtifact {
    pub agent: String,
    pub delegated_run_id: Option<String>,
    pub success: bool,
    pub usable_evidence: bool,
    pub degraded_success: bool,
    pub outcome_reason: String,
    pub summary: String,
    pub module_structure: Option<String>,
    pub structural_coverage: Option<String>,
    pub semantic_coverage: Option<String>,
    pub responsibilities: Vec<String>,
    pub key_apis: Vec<String>,
    pub integration_points: Vec<String>,
    pub strengths: Vec<String>,
    pub gaps: Vec<String>,
    pub paths_examined: Vec<String>,
    pub files_examined: Vec<String>,
    pub directories_examined: Vec<String>,
    pub key_findings: Vec<String>,
    pub design_patterns: Vec<String>,
    pub concerns: Vec<String>,
    pub confidence: Option<String>,
    pub turns_used: usize,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub policy_violations: Vec<String>,
}

fn strip_think_blocks(text: &str) -> String {
    let mut cleaned = text.to_string();
    loop {
        let Some(start) = cleaned.find("<think>") else {
            break;
        };
        let Some(end_rel) = cleaned[start..].find("</think>") else {
            cleaned.replace_range(start.., "");
            break;
        };
        let end = start + end_rel + "</think>".len();
        cleaned.replace_range(start..end, "");
    }
    cleaned.trim().to_string()
}

fn normalize_coverage_level(value: Option<String>) -> Option<String> {
    value.and_then(|coverage| {
        let normalized = strip_think_blocks(&coverage).to_ascii_lowercase();
        match normalized.trim() {
            "high" => Some("high".to_string()),
            "medium" => Some("medium".to_string()),
            "low" => Some("low".to_string()),
            _ => None,
        }
    })
}

fn infer_structural_coverage(report: &ExploreReport) -> Option<String> {
    if !report.paths_examined.is_empty() && report.module_structure.is_some() {
        Some("high".to_string())
    } else if !report.paths_examined.is_empty() {
        Some("medium".to_string())
    } else {
        None
    }
}

fn infer_semantic_coverage(report: &ExploreReport) -> String {
    let semantic_signals = [
        !report.responsibilities.is_empty(),
        !report.key_apis.is_empty(),
        !report.integration_points.is_empty(),
        !report.strengths.is_empty(),
        !report.gaps.is_empty(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();

    match semantic_signals {
        0 => "low".to_string(),
        1..=2 => "medium".to_string(),
        _ => "high".to_string(),
    }
}

fn normalize_explore_report(mut report: ExploreReport) -> ExploreReport {
    report.summary = strip_think_blocks(&report.summary);
    report.module_structure = report
        .module_structure
        .take()
        .map(|structure| strip_think_blocks(&structure))
        .filter(|structure| !structure.trim().is_empty());
    report.structural_coverage = normalize_coverage_level(report.structural_coverage.take());
    report.semantic_coverage = normalize_coverage_level(report.semantic_coverage.take());
    report.responsibilities = report
        .responsibilities
        .into_iter()
        .map(|item| strip_think_blocks(&item))
        .filter(|item| !item.trim().is_empty())
        .collect();
    report.key_apis = report
        .key_apis
        .into_iter()
        .map(|item| strip_think_blocks(&item))
        .filter(|item| !item.trim().is_empty())
        .collect();
    report.integration_points = report
        .integration_points
        .into_iter()
        .map(|item| strip_think_blocks(&item))
        .filter(|item| !item.trim().is_empty())
        .collect();
    report.strengths = report
        .strengths
        .into_iter()
        .map(|item| strip_think_blocks(&item))
        .filter(|item| !item.trim().is_empty())
        .collect();
    report.gaps = report
        .gaps
        .into_iter()
        .map(|item| strip_think_blocks(&item))
        .filter(|item| !item.trim().is_empty())
        .collect();
    report.key_findings = report
        .key_findings
        .into_iter()
        .map(|finding| strip_think_blocks(&finding))
        .filter(|finding| !finding.trim().is_empty())
        .collect();
    report.design_patterns = report
        .design_patterns
        .into_iter()
        .map(|pattern| strip_think_blocks(&pattern))
        .filter(|pattern| !pattern.trim().is_empty())
        .collect();
    report.concerns = report
        .concerns
        .into_iter()
        .map(|concern| strip_think_blocks(&concern))
        .filter(|concern| !concern.trim().is_empty())
        .collect();
    if report.structural_coverage.is_none() {
        report.structural_coverage = infer_structural_coverage(&report);
    }
    if report.semantic_coverage.is_none() {
        report.semantic_coverage = Some(infer_semantic_coverage(&report));
    }
    report
}

pub(crate) fn parse_explore_report(text: &str) -> Option<ExploreReport> {
    let trimmed = text.trim();
    let tagged = extract_tagged_json(trimmed, "explore_report").unwrap_or(trimmed);
    serde_json::from_str::<ExploreReport>(tagged)
        .ok()
        .map(normalize_explore_report)
}

pub(crate) fn render_explore_report(report: &ExploreReport) -> String {
    let body = serde_json::to_string_pretty(report).unwrap_or_else(|_| {
        json!({
            "summary": report.summary,
            "module_structure": report.module_structure,
            "structural_coverage": report.structural_coverage,
            "semantic_coverage": report.semantic_coverage,
            "responsibilities": report.responsibilities,
            "key_apis": report.key_apis,
            "integration_points": report.integration_points,
            "strengths": report.strengths,
            "gaps": report.gaps,
            "paths_examined": report.paths_examined,
            "files_examined": report.files_examined,
            "key_findings": report.key_findings,
            "design_patterns": report.design_patterns,
            "concerns": report.concerns,
            "confidence": report.confidence,
        })
        .to_string()
    });

    format!("<explore_report>\n{}\n</explore_report>", body)
}

pub(crate) fn synthesize_explore_report(
    summary: &str,
    paths_examined: &[String],
) -> Option<ExploreReport> {
    let cleaned_summary = summary.trim();
    let paths = dedup_files(paths_examined, 12);
    let (_, files) = split_examined_paths(&paths);

    if cleaned_summary.is_empty() || paths.is_empty() {
        return None;
    }

    let findings = cleaned_summary
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .map(ToString::to_string)
        .collect::<Vec<_>>();

    Some(ExploreReport {
        summary: cleaned_summary.to_string(),
        module_structure: None,
        structural_coverage: Some("medium".to_string()),
        semantic_coverage: Some("low".to_string()),
        responsibilities: Vec::new(),
        key_apis: Vec::new(),
        integration_points: Vec::new(),
        strengths: Vec::new(),
        gaps: Vec::new(),
        paths_examined: paths,
        files_examined: files,
        key_findings: if findings.is_empty() {
            vec![cleaned_summary.to_string()]
        } else {
            findings
        },
        design_patterns: Vec::new(),
        concerns: Vec::new(),
        confidence: Some("medium".to_string()),
    })
}

pub(crate) fn synthesize_explore_report_from_paths(
    agent_label: &str,
    paths_examined: &[String],
) -> Option<ExploreReport> {
    let paths = dedup_files(paths_examined, 12);
    if paths.is_empty() {
        return None;
    }

    let (directories, files) = split_examined_paths(&paths);
    let mut findings = Vec::new();
    if !directories.is_empty() {
        findings.push(format!(
            "Top-level structure includes: {}",
            directories
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !files.is_empty() {
        findings.push(format!(
            "Representative files include: {}",
            files.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if findings.is_empty() {
        findings.push(format!(
            "Collected {} supporting paths for {}",
            paths.len(),
            agent_label
        ));
    }

    let summary = if !directories.is_empty() && !files.is_empty() {
        format!(
            "Explored {} and identified a directory-led module layout with representative files for the main implementation areas.",
            agent_label
        )
    } else if !directories.is_empty() {
        format!(
            "Explored {} and identified the main directory structure and module boundaries from the available paths.",
            agent_label
        )
    } else {
        format!(
            "Explored {} and identified representative source files for the main implementation areas.",
            agent_label
        )
    };

    Some(ExploreReport {
        summary,
        module_structure: Some(if !directories.is_empty() {
            format!(
                "{} is organized around {} with representative files {}.",
                agent_label,
                directories
                    .iter()
                    .take(8)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", "),
                if files.is_empty() {
                    "not yet read deeply".to_string()
                } else {
                    files.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
                }
            )
        } else {
            format!(
                "{} is represented primarily by files {}.",
                agent_label,
                files.iter().take(6).cloned().collect::<Vec<_>>().join(", ")
            )
        }),
        structural_coverage: Some(if !directories.is_empty() {
            "high".to_string()
        } else {
            "medium".to_string()
        }),
        semantic_coverage: Some("low".to_string()),
        responsibilities: Vec::new(),
        key_apis: Vec::new(),
        integration_points: Vec::new(),
        strengths: Vec::new(),
        gaps: Vec::new(),
        paths_examined: paths,
        files_examined: files,
        key_findings: findings,
        design_patterns: Vec::new(),
        concerns: Vec::new(),
        confidence: Some("medium".to_string()),
    })
}

fn extract_tagged_json<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = text.find(&open)?;
    let end = text[start + open.len()..].find(&close)?;
    Some(text[start + open.len()..start + open.len() + end].trim())
}

/// Result from a sub-agent execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentResult {
    pub task_id: String,
    pub agent_name: String,
    pub delegated_run_id: Option<String>,
    pub success: bool,
    pub output: String,
    pub files_examined: Vec<String>,
    pub duration_ms: u64,
    pub turns_used: usize,
    pub error: Option<String>,
    pub policy_violations: Vec<String>,
}

impl SubAgentResult {
    pub fn brief_summary(&self) -> String {
        if let Some(report) = parse_explore_report(&self.output) {
            let mut parts = vec![report.summary.trim().to_string()];
            if !report.key_findings.is_empty() {
                parts.push(
                    report
                        .key_findings
                        .iter()
                        .take(2)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            let combined = parts
                .into_iter()
                .filter(|part| !part.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !combined.trim().is_empty() {
                return truncate_preview(combined.trim(), 600);
            }
        }

        let lines = self
            .output
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .take(4)
            .collect::<Vec<_>>();

        let summary = if lines.is_empty() {
            self.error
                .clone()
                .unwrap_or_else(|| "No summary produced".to_string())
        } else {
            lines.join(" ")
        };

        truncate_preview(&summary, 600)
    }

    pub fn evidence_artifact(&self) -> ExploreEvidenceArtifact {
        let report = parse_explore_report(&self.output);
        let report_paths = report
            .as_ref()
            .map(|parsed| {
                let paths = if parsed.paths_examined.is_empty() {
                    parsed.files_examined.clone()
                } else {
                    parsed.paths_examined.clone()
                };
                dedup_files(&paths, 12)
            })
            .unwrap_or_default();
        let paths_examined = if report_paths.is_empty() {
            dedup_files(&self.files_examined, 12)
        } else {
            report_paths
        };
        let (directories_examined, concrete_files_examined) = split_examined_paths(&paths_examined);
        let report_confidence = report.as_ref().and_then(|parsed| parsed.confidence.clone());

        ExploreEvidenceArtifact {
            agent: if self.agent_name.trim().is_empty() {
                self.task_id.clone()
            } else {
                self.agent_name.clone()
            },
            delegated_run_id: self.delegated_run_id.clone(),
            success: self.success,
            usable_evidence: self.has_usable_evidence(),
            degraded_success: self.is_degraded_success(),
            outcome_reason: self.outcome_reason().to_string(),
            summary: self.brief_summary(),
            module_structure: report
                .as_ref()
                .and_then(|parsed| parsed.module_structure.clone()),
            structural_coverage: report
                .as_ref()
                .and_then(|parsed| parsed.structural_coverage.clone()),
            semantic_coverage: report
                .as_ref()
                .and_then(|parsed| parsed.semantic_coverage.clone()),
            responsibilities: report
                .as_ref()
                .map(|parsed| parsed.responsibilities.clone())
                .unwrap_or_default(),
            key_apis: report
                .as_ref()
                .map(|parsed| parsed.key_apis.clone())
                .unwrap_or_default(),
            integration_points: report
                .as_ref()
                .map(|parsed| parsed.integration_points.clone())
                .unwrap_or_default(),
            strengths: report
                .as_ref()
                .map(|parsed| parsed.strengths.clone())
                .unwrap_or_default(),
            gaps: report
                .as_ref()
                .map(|parsed| parsed.gaps.clone())
                .unwrap_or_default(),
            paths_examined,
            files_examined: concrete_files_examined,
            directories_examined,
            key_findings: report
                .as_ref()
                .map(|parsed| parsed.key_findings.clone())
                .unwrap_or_default(),
            design_patterns: report
                .as_ref()
                .map(|parsed| parsed.design_patterns.clone())
                .unwrap_or_default(),
            concerns: report
                .as_ref()
                .map(|parsed| parsed.concerns.clone())
                .unwrap_or_default(),
            confidence: report_confidence,
            turns_used: self.turns_used,
            duration_ms: self.duration_ms,
            error: self.error.clone(),
            policy_violations: self.policy_violations.clone(),
        }
    }

    pub fn evidence_json(&self) -> Value {
        serde_json::to_value(self.evidence_artifact()).unwrap_or_else(|_| {
            json!({
                "agent": if self.agent_name.trim().is_empty() {
                    self.task_id.clone()
                } else {
                    self.agent_name.clone()
                },
                "delegated_run_id": self.delegated_run_id,
                "success": self.success,
                "usable_evidence": self.has_usable_evidence(),
                "degraded_success": self.is_degraded_success(),
                "outcome_reason": self.outcome_reason(),
                "summary": self.brief_summary(),
                "module_structure": Option::<String>::None,
                "structural_coverage": Some("low"),
                "semantic_coverage": Some("low"),
                "responsibilities": Vec::<String>::new(),
                "key_apis": Vec::<String>::new(),
                "integration_points": Vec::<String>::new(),
                "strengths": Vec::<String>::new(),
                "gaps": Vec::<String>::new(),
                "paths_examined": dedup_files(&self.files_examined, 12),
                "files_examined": dedup_files(&self.files_examined, 12),
                "directories_examined": Vec::<String>::new(),
                "key_findings": Vec::<String>::new(),
                "design_patterns": Vec::<String>::new(),
                "concerns": Vec::<String>::new(),
                "confidence": Option::<String>::None,
                "turns_used": self.turns_used,
                "duration_ms": self.duration_ms,
                "error": self.error,
                "policy_violations": self.policy_violations,
            })
        })
    }

    pub fn has_usable_evidence(&self) -> bool {
        if !self.success || self.error.is_some() {
            return false;
        }

        if let Some(report) = parse_explore_report(&self.output) {
            return !report.summary.trim().is_empty()
                && !summary_looks_non_substantive(&report.summary)
                && (!report.paths_examined.is_empty() || !report.files_examined.is_empty())
                && report.key_findings.iter().any(|finding| {
                    let trimmed = finding.trim();
                    !trimmed.is_empty() && !summary_looks_non_substantive(trimmed)
                });
        }

        !self.files_examined.is_empty()
    }

    pub fn is_degraded_success(&self) -> bool {
        self.success && !self.has_usable_evidence()
    }

    pub fn outcome_reason(&self) -> &'static str {
        let error = self
            .error
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();

        if error.contains("misread successful tool output") {
            return "misread_tool_output";
        }
        if error.contains("required structured explore report") {
            return "missing_structured_report";
        }
        if error.contains("invalid explore target")
            || error.contains("missing explore target")
            || error.contains("outside project")
        {
            return "invalid_target";
        }
        if error.contains("http ")
            || error.contains("timeout")
            || error.contains("connection")
            || error.contains("network")
            || error.contains("provider")
        {
            return "provider_failure";
        }
        if self.success && !self.has_usable_evidence() {
            return "no_usable_evidence";
        }
        if self.success {
            return "usable_evidence";
        }
        if self.error.is_some() {
            return "execution_failure";
        }
        "unknown"
    }
}

fn dedup_files(files: &[String], limit: usize) -> Vec<String> {
    let mut unique = Vec::new();
    for file in files {
        if !unique.iter().any(|existing| existing == file) {
            unique.push(file.clone());
        }
        if unique.len() >= limit {
            break;
        }
    }
    unique
}

fn split_examined_paths(paths: &[String]) -> (Vec<String>, Vec<String>) {
    let mut directories = Vec::new();
    let mut files = Vec::new();

    for path in paths {
        if path.ends_with('/') {
            directories.push(path.clone());
        } else {
            files.push(path.clone());
        }
    }

    (directories, files)
}

fn truncate_preview(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }

    let mut boundary = max_chars.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }

    text[..boundary].trim_end().to_string()
}

pub(crate) fn summary_looks_non_substantive(summary: &str) -> bool {
    let normalized = summary.trim().to_ascii_lowercase();
    let markers = [
        "<think>",
        "unable to explore",
        "could not be accessed",
        "does not exist",
        "inaccessible",
        "returned empty results",
        "returned no results",
        "no results",
        "let me check",
        "let me explore",
        "let me inspect",
        "let me read",
        "let me try",
        "let me use",
        "i'll start by",
        "i will start by",
        "i'm going to",
        "first i'll",
        "first, i'll",
    ];

    markers.iter().any(|marker| normalized.contains(marker))
}

/// Parsed tool call from API response
#[derive(Debug)]
pub(crate) struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[cfg(test)]
mod tests {
    use super::{
        parse_explore_report, synthesize_explore_report, synthesize_explore_report_from_paths,
        SubAgentResult,
    };

    fn base_result() -> SubAgentResult {
        SubAgentResult {
            task_id: "agent-1".to_string(),
            agent_name: "agent-1".to_string(),
            delegated_run_id: Some("run-1".to_string()),
            success: true,
            output: String::new(),
            files_examined: Vec::new(),
            duration_ms: 0,
            turns_used: 0,
            error: None,
            policy_violations: Vec::new(),
        }
    }

    #[test]
    fn usable_evidence_requires_more_than_empty_fallback_summary() {
        let mut result = base_result();
        result.output =
            "Exploration converged without a final synthesized answer. No files examined."
                .to_string();

        assert!(!result.has_usable_evidence());
        assert!(result.is_degraded_success());
    }

    #[test]
    fn usable_evidence_accepts_examined_files() {
        let mut result = base_result();
        result.output = r#"<explore_report>
{
  "summary": "Investigated the orchestrator and tool registry.",
  "paths_examined": ["src/agent/", "src/agent/orchestrator.rs"],
  "files_examined": ["src/agent/orchestrator.rs"],
  "key_findings": ["The orchestrator owns the main loop."],
  "design_patterns": ["event bus"],
  "concerns": [],
  "confidence": "high"
}
</explore_report>"#
            .to_string();

        assert!(result.has_usable_evidence());
        assert!(!result.is_degraded_success());
        assert_eq!(result.outcome_reason(), "usable_evidence");
    }

    #[test]
    fn parse_explore_report_reads_tagged_json() {
        let report = parse_explore_report(
            r#"
before
<explore_report>
{"summary":"ok","files_examined":["src/lib.rs"],"key_findings":["x"],"design_patterns":[],"concerns":[],"confidence":"medium"}
</explore_report>
after
"#,
        )
        .expect("report");

        assert_eq!(report.summary, "ok");
        assert_eq!(report.files_examined, vec!["src/lib.rs"]);
    }

    #[test]
    fn parse_explore_report_strips_think_blocks() {
        let report = parse_explore_report(
            r#"<explore_report>
{"summary":"<think>hidden</think>Visible summary","paths_examined":["src/"],"files_examined":[],"key_findings":["<think>nope</think>Real finding"],"design_patterns":[],"concerns":[],"confidence":"medium"}
</explore_report>"#,
        )
        .expect("report");

        assert_eq!(report.summary, "Visible summary");
        assert_eq!(report.key_findings, vec!["Real finding"]);
    }

    #[test]
    fn synthesize_explore_report_promotes_real_evidence() {
        let report = synthesize_explore_report(
            "The orchestrator owns the canonical loop.\nThe tool registry centralizes tools.",
            &[
                "src/agent/".to_string(),
                "src/agent/orchestrator.rs".to_string(),
                "src/tools/registry.rs".to_string(),
            ],
        )
        .expect("report");

        assert_eq!(report.paths_examined.len(), 3);
        assert_eq!(report.files_examined.len(), 2);
        assert!(!report.key_findings.is_empty());
        assert_eq!(report.confidence.as_deref(), Some("medium"));
    }

    #[test]
    fn synthesize_explore_report_from_paths_builds_deterministic_summary() {
        let report = synthesize_explore_report_from_paths(
            "krusty-core/src",
            &[
                "agent/".to_string(),
                "storage/".to_string(),
                "agent/orchestrator.rs".to_string(),
            ],
        )
        .expect("report");

        assert!(report.summary.contains("krusty-core/src"));
        assert_eq!(report.paths_examined.len(), 3);
        assert!(!report.key_findings.is_empty());
    }

    #[test]
    fn usable_evidence_accepts_directory_backed_explore_report() {
        let mut result = base_result();
        result.output = r#"<explore_report>
{
  "summary": "The crate is organized into agent, ai, and storage subsystems.",
  "paths_examined": ["src/agent/", "src/ai/", "src/storage/"],
  "files_examined": [],
  "key_findings": ["The top-level architecture is split by subsystem directory."],
  "design_patterns": ["modular crate layout"],
  "concerns": [],
  "confidence": "medium"
}
</explore_report>"#
            .to_string();

        assert!(result.has_usable_evidence());
        let artifact = result.evidence_artifact();
        assert_eq!(artifact.directories_examined.len(), 3);
        assert!(artifact.files_examined.is_empty());
    }

    #[test]
    fn usable_evidence_rejects_non_substantive_explore_reports() {
        let mut result = base_result();
        result.output = r#"<explore_report>
{
  "summary": "Unable to explore the target directory because the tools returned empty results.",
  "paths_examined": ["/tmp/demo/src"],
  "files_examined": [],
  "key_findings": ["The directory could not be accessed"],
  "design_patterns": [],
  "concerns": ["No usable source evidence was gathered"],
  "confidence": "low"
}
</explore_report>"#
            .to_string();

        assert!(!result.has_usable_evidence());

        result.output = r#"<explore_report>
{
  "summary": "Let me check the parent directory structure:",
  "paths_examined": ["src/", "main.rs"],
  "files_examined": ["main.rs"],
  "key_findings": ["Let me check the parent directory structure:"],
  "design_patterns": [],
  "concerns": [],
  "confidence": "medium"
}
</explore_report>"#
            .to_string();

        assert!(!result.has_usable_evidence());
    }

    #[test]
    fn evidence_artifact_separates_directories_from_files() {
        let mut result = base_result();
        result.files_examined = vec![
            "agent/".to_string(),
            "storage/".to_string(),
            "agent/orchestrator.rs".to_string(),
        ];
        result.output = "Investigated runtime and storage layout".to_string();

        let artifact = result.evidence_artifact();
        assert_eq!(artifact.delegated_run_id.as_deref(), Some("run-1"));
        assert_eq!(artifact.directories_examined.len(), 2);
        assert_eq!(
            artifact.files_examined,
            vec!["agent/orchestrator.rs".to_string()]
        );
        assert_eq!(artifact.paths_examined.len(), 3);
    }

    #[test]
    fn outcome_reason_detects_misread_tool_output() {
        let mut result = base_result();
        result.success = false;
        result.error = Some("Misread successful tool output after correction".to_string());

        assert_eq!(result.outcome_reason(), "misread_tool_output");
    }
}
