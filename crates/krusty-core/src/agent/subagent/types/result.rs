use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::report::{parse_explore_report, summary_looks_non_substantive};

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

/// Result from a sub-agent execution.
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

#[cfg(test)]
mod tests {
    use super::SubAgentResult;

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
