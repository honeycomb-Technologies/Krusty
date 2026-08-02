use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::report::{parse_explore_report, summary_looks_non_substantive};

/// Compact, provider-neutral proof that a delegated child actually exercised
/// its governed capability surface. Raw tool output is intentionally excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedEvidenceSummary {
    #[serde(default)]
    pub attempted: usize,
    #[serde(default)]
    pub succeeded: usize,
    #[serde(default)]
    pub observations: usize,
    #[serde(default)]
    pub mutations: usize,
    #[serde(default)]
    pub executions: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegatedEvidenceKind {
    Observation,
    Mutation,
    Execution,
}

/// Provider-neutral reason the delegated loop stopped.
///
/// `Completed` is the serde default so results serialized before this field was
/// introduced remain readable. Callers must still combine this value with
/// `success` and retained evidence before publishing a durable terminal stage.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubAgentTermination {
    #[default]
    Completed,
    ProviderMaxTokens,
    ProviderTimeout,
    /// The semantic loop guard stopped repeated work after one bounded,
    /// tool-free synthesis turn. Canonical evidence remains usable as a
    /// degraded result; a guard with no evidence remains a failure.
    LoopGuard,
    Failed,
    Cancelled,
}

impl SubAgentTermination {
    pub fn is_provider_interruption(self) -> bool {
        matches!(self, Self::ProviderMaxTokens | Self::ProviderTimeout)
    }

    pub fn is_degraded_interruption(self) -> bool {
        self.is_provider_interruption() || self == Self::LoopGuard
    }
}

impl DelegatedEvidenceSummary {
    pub fn record_attempt(&mut self) {
        self.attempted = self.attempted.saturating_add(1);
    }

    pub fn record_success(&mut self, kind: DelegatedEvidenceKind) {
        self.succeeded = self.succeeded.saturating_add(1);
        let counter = match kind {
            DelegatedEvidenceKind::Observation => &mut self.observations,
            DelegatedEvidenceKind::Mutation => &mut self.mutations,
            DelegatedEvidenceKind::Execution => &mut self.executions,
        };
        *counter = counter.saturating_add(1);
    }

    pub fn has_canonical_evidence(&self) -> bool {
        self.succeeded > 0
    }
}

/// Canonical background process handoff produced by a delegated agent tool
/// call. This is collected from the successful Bash result itself rather than
/// trusting the delegated model to repeat the handle in its prose summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedProcessArtifact {
    pub process_id: String,
    pub status: String,
    pub command: String,
    pub working_dir: String,
    #[serde(default)]
    pub endpoint_hints: Vec<String>,
    #[serde(default)]
    pub reused_existing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExploreEvidenceArtifact {
    pub agent: String,
    pub delegated_run_id: Option<String>,
    pub success: bool,
    pub usable_evidence: bool,
    pub degraded_success: bool,
    pub outcome_reason: String,
    #[serde(default)]
    pub termination: SubAgentTermination,
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
    #[serde(default)]
    pub evidence: DelegatedEvidenceSummary,
    #[serde(default)]
    pub background_processes: Vec<DelegatedProcessArtifact>,
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
    #[serde(default)]
    pub termination: SubAgentTermination,
    pub policy_violations: Vec<String>,
    #[serde(default)]
    pub evidence: DelegatedEvidenceSummary,
    #[serde(default)]
    pub background_processes: Vec<DelegatedProcessArtifact>,
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
            termination: self.termination,
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
            evidence: self.evidence.clone(),
            background_processes: self.background_processes.clone(),
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
                "termination": self.termination,
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
                "evidence": self.evidence,
                "background_processes": self.background_processes,
            })
        })
    }

    /// Whether the result retains evidence that is useful to a parent or a
    /// resumed run, independently of whether the child reached a clean end.
    pub fn has_retained_evidence(&self) -> bool {
        if !self.background_processes.is_empty() {
            return true;
        }

        if self.evidence.has_canonical_evidence() {
            return true;
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

    /// Canonical evidence strong enough to publish an interrupted provider
    /// response as a durable partial result. Merely attempting a path does not
    /// qualify because failed reads may still populate `files_examined`.
    pub fn has_partial_evidence(&self) -> bool {
        self.evidence.has_canonical_evidence() || !self.background_processes.is_empty()
    }

    pub fn has_usable_evidence(&self) -> bool {
        if self.termination.is_degraded_interruption() {
            return self.has_partial_evidence();
        }
        self.success && self.error.is_none() && self.has_retained_evidence()
    }

    pub fn is_degraded_success(&self) -> bool {
        (self.termination.is_degraded_interruption() && self.has_partial_evidence())
            || (self.success && !self.has_usable_evidence())
    }

    pub fn outcome_reason(&self) -> &'static str {
        match self.termination {
            SubAgentTermination::ProviderMaxTokens => return "provider_max_tokens",
            SubAgentTermination::ProviderTimeout => return "provider_timeout",
            SubAgentTermination::LoopGuard => return "loop_guard",
            SubAgentTermination::Cancelled => return "cancelled",
            SubAgentTermination::Completed | SubAgentTermination::Failed => {}
        }

        let error = self
            .error
            .as_deref()
            .unwrap_or_default()
            .to_ascii_lowercase();

        if error.contains("misread successful tool output") {
            return "misread_tool_output";
        }
        if error.contains("canonical tool evidence") {
            return "no_canonical_tool_evidence";
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
    use serde_json::json;

    use super::{
        DelegatedEvidenceKind, DelegatedEvidenceSummary, DelegatedProcessArtifact, SubAgentResult,
        SubAgentTermination,
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
            termination: SubAgentTermination::Completed,
            policy_violations: Vec::new(),
            evidence: DelegatedEvidenceSummary::default(),
            background_processes: Vec::new(),
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
    fn canonical_tool_evidence_is_compact_and_makes_a_success_usable() {
        let mut result = base_result();
        result.output = "Updated the requested source file.".to_string();
        result.evidence.record_attempt();
        result
            .evidence
            .record_success(DelegatedEvidenceKind::Mutation);

        assert!(result.has_usable_evidence());
        assert_eq!(result.outcome_reason(), "usable_evidence");
        let artifact = result.evidence_json();
        assert_eq!(artifact["evidence"]["attempted"], 1);
        assert_eq!(artifact["evidence"]["succeeded"], 1);
        assert_eq!(artifact["evidence"]["mutations"], 1);
        assert!(artifact["evidence"].get("raw_output").is_none());
    }

    #[test]
    fn legacy_serialized_result_defaults_missing_evidence_ledger() {
        let result: SubAgentResult = serde_json::from_value(json!({
            "task_id": "legacy",
            "agent_name": "legacy",
            "delegated_run_id": null,
            "success": true,
            "output": "legacy prose",
            "files_examined": [],
            "duration_ms": 1,
            "turns_used": 1,
            "error": null,
            "policy_violations": [],
            "background_processes": []
        }))
        .expect("legacy result should remain deserializable");

        assert_eq!(result.evidence, DelegatedEvidenceSummary::default());
        assert_eq!(result.termination, SubAgentTermination::Completed);
        assert!(!result.has_usable_evidence());
    }

    #[test]
    fn provider_interruption_retains_evidence_without_becoming_success() {
        let mut result = base_result();
        result.success = false;
        result.error = Some("Provider output reached its token limit".to_string());
        result.termination = SubAgentTermination::ProviderMaxTokens;
        result
            .evidence
            .record_success(DelegatedEvidenceKind::Observation);

        assert!(result.has_retained_evidence());
        assert!(result.has_partial_evidence());
        assert!(result.has_usable_evidence());
        assert!(result.is_degraded_success());
        assert_eq!(result.outcome_reason(), "provider_max_tokens");
        assert_eq!(result.evidence_json()["termination"], "provider_max_tokens");
    }

    #[test]
    fn provider_interruption_without_evidence_is_not_usable() {
        let mut result = base_result();
        result.success = false;
        result.error = Some("Provider call timed out".to_string());
        result.termination = SubAgentTermination::ProviderTimeout;

        assert!(!result.has_retained_evidence());
        assert!(!result.has_partial_evidence());
        assert!(!result.has_usable_evidence());
        assert!(!result.is_degraded_success());
        assert_eq!(result.outcome_reason(), "provider_timeout");
    }

    #[test]
    fn loop_guard_evidence_is_degraded_but_never_clean_completion() {
        let mut result = base_result();
        result.success = false;
        result.error = Some("semantic loop guard".to_string());
        result.termination = SubAgentTermination::LoopGuard;
        result
            .evidence
            .record_success(DelegatedEvidenceKind::Observation);

        assert!(result.has_usable_evidence());
        assert!(result.is_degraded_success());
        assert_eq!(result.outcome_reason(), "loop_guard");
        assert_eq!(result.evidence_json()["termination"], "loop_guard");
    }

    #[test]
    fn interrupted_provider_does_not_treat_attempted_paths_as_partial_evidence() {
        let mut result = base_result();
        result.success = false;
        result.error = Some("Provider call timed out".to_string());
        result.termination = SubAgentTermination::ProviderTimeout;
        result.files_examined.push("missing.rs".to_string());

        assert!(result.has_retained_evidence());
        assert!(!result.has_partial_evidence());
        assert!(!result.has_usable_evidence());
    }

    #[test]
    fn missing_canonical_evidence_has_a_specific_outcome_reason() {
        let mut result = base_result();
        result.success = false;
        result.error =
            Some("Delegated child completed without canonical tool evidence".to_string());

        assert_eq!(result.outcome_reason(), "no_canonical_tool_evidence");
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

    #[test]
    fn evidence_artifact_preserves_canonical_background_process_handoff() {
        let mut result = base_result();
        result.output = "Server started successfully.".to_string();
        result.background_processes = vec![DelegatedProcessArtifact {
            process_id: "process-123".to_string(),
            status: "running".to_string(),
            command: "python3 server.py --host 127.0.0.1 --port 5940".to_string(),
            working_dir: "/workspace/demo".to_string(),
            endpoint_hints: vec!["127.0.0.1:5940".to_string()],
            reused_existing: false,
        }];

        let artifact = result.evidence_json();
        assert_eq!(
            artifact["background_processes"][0]["process_id"],
            "process-123"
        );
        assert_eq!(
            artifact["background_processes"][0]["endpoint_hints"],
            json!(["127.0.0.1:5940"])
        );
        assert_eq!(artifact["usable_evidence"], true);
        assert_eq!(artifact["outcome_reason"], "usable_evidence");
    }
}
