use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::report::{parse_explore_report, summary_looks_non_substantive};

const HANDOFF_OPEN: &str = "<delegated_handoff>";
const HANDOFF_CLOSE: &str = "</delegated_handoff>";
const ACCEPTANCE_CONTRACT_OPEN: &str = "<delegated_acceptance_contract>";
const ACCEPTANCE_CONTRACT_CLOSE: &str = "</delegated_acceptance_contract>";
const MAX_HANDOFF_SUMMARY_BYTES: usize = 1_200;
const MAX_HANDOFF_ACCEPTANCE_CHECKS: usize = 8;
const MAX_HANDOFF_CHECK_ID_BYTES: usize = 128;
const MAX_HANDOFF_CHECK_STATUS_BYTES: usize = 32;
const MAX_HANDOFF_CHECK_EVIDENCE_BYTES: usize = 1_200;
const MAX_HANDOFF_REMAINING_ITEMS: usize = 8;
const MAX_HANDOFF_REMAINING_ITEM_BYTES: usize = 600;
const MAX_HANDOFF_BLOCKERS: usize = 8;
const MAX_HANDOFF_BLOCKER_BYTES: usize = 600;
const MAX_HANDOFF_GENERATED_ARTIFACTS: usize = 16;
const MAX_HANDOFF_ARTIFACT_PATH_BYTES: usize = 512;
const MAX_HANDOFF_ARTIFACT_PURPOSE_BYTES: usize = 600;

pub(crate) fn parse_delegated_handoff(output: &str) -> Option<DelegatedTaskHandoff> {
    let output = output.trim_end();
    if !output.ends_with(HANDOFF_CLOSE)
        || output.matches(HANDOFF_OPEN).count() != 1
        || output.matches(HANDOFF_CLOSE).count() != 1
    {
        return None;
    }
    let start = output.find(HANDOFF_OPEN)? + HANDOFF_OPEN.len();
    let end = output[start..].find(HANDOFF_CLOSE)? + start;
    serde_json::from_str(output[start..end].trim()).ok()
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskObjectiveStatus {
    Complete,
    #[default]
    Degraded,
    Blocked,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedAcceptanceCheck {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub evidence: String,
}

/// A generated deliverable that must cross an isolated build boundary.
///
/// Paths are provider-authored claims until the isolation integrator validates
/// them against the task worktree. Cache and dependency directories remain
/// prohibited even when a provider declares them here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedGeneratedArtifact {
    pub path: String,
    #[serde(default)]
    pub purpose: String,
}

/// Provider-authored handoff interpreted under canonical runtime evidence.
/// The provider may truthfully declare incomplete work, but it cannot create
/// tool evidence or promote a failed validation by prose alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedTaskHandoff {
    pub status: TaskObjectiveStatus,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub acceptance_checks: Vec<DelegatedAcceptanceCheck>,
    #[serde(default)]
    pub remaining_work: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub generated_artifacts: Vec<DelegatedGeneratedArtifact>,
}

impl DelegatedTaskHandoff {
    fn is_complete(&self) -> bool {
        self.status == TaskObjectiveStatus::Complete
            && self.remaining_work.is_empty()
            && self.blockers.is_empty()
            && !self.acceptance_checks.is_empty()
            && self.acceptance_checks.iter().all(|check| {
                matches!(
                    check.status.trim().to_ascii_lowercase().as_str(),
                    "passed" | "pass" | "not_applicable" | "not-applicable"
                ) && !check.evidence.trim().is_empty()
            })
    }

    /// Keep the provider-authored handoff useful to the parent without
    /// retaining an unbounded copy of delegated model output.
    fn bounded(&self) -> Self {
        Self {
            status: self.status,
            summary: truncate_preview(&self.summary, MAX_HANDOFF_SUMMARY_BYTES),
            acceptance_checks: self
                .acceptance_checks
                .iter()
                .take(MAX_HANDOFF_ACCEPTANCE_CHECKS)
                .map(|check| DelegatedAcceptanceCheck {
                    id: truncate_preview(&check.id, MAX_HANDOFF_CHECK_ID_BYTES),
                    status: truncate_preview(&check.status, MAX_HANDOFF_CHECK_STATUS_BYTES),
                    evidence: truncate_preview(&check.evidence, MAX_HANDOFF_CHECK_EVIDENCE_BYTES),
                })
                .collect(),
            remaining_work: bounded_strings(
                &self.remaining_work,
                MAX_HANDOFF_REMAINING_ITEMS,
                MAX_HANDOFF_REMAINING_ITEM_BYTES,
            ),
            blockers: bounded_strings(
                &self.blockers,
                MAX_HANDOFF_BLOCKERS,
                MAX_HANDOFF_BLOCKER_BYTES,
            ),
            generated_artifacts: self
                .generated_artifacts
                .iter()
                .take(MAX_HANDOFF_GENERATED_ARTIFACTS)
                .map(|artifact| DelegatedGeneratedArtifact {
                    path: truncate_preview(&artifact.path, MAX_HANDOFF_ARTIFACT_PATH_BYTES),
                    purpose: truncate_preview(
                        &artifact.purpose,
                        MAX_HANDOFF_ARTIFACT_PURPOSE_BYTES,
                    ),
                })
                .collect(),
        }
    }
}

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
    /// Canonical acceptance capabilities proven by successful governed tools.
    /// Provider-authored handoff prose cannot populate this field.
    #[serde(default)]
    pub acceptance_proofs: Vec<String>,
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

    pub fn record_acceptance_proof(&mut self, proof: &str) {
        if !self.acceptance_proofs.iter().any(|item| item == proof) {
            self.acceptance_proofs.push(proof.to_string());
            self.acceptance_proofs.sort();
        }
    }

    pub(crate) fn has_acceptance_proof(&self, proof: &str) -> bool {
        self.acceptance_proofs.iter().any(|item| item == proof)
    }
}

/// Canonical background process handoff produced by a delegated agent tool
/// call. This is collected from the successful Bash result itself rather than
/// trusting the delegated model to repeat the handle in its prose summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedProcessArtifact {
    pub process_id: String,
    /// Exact process-registry owner. New delegated tasks use a task-scoped
    /// owner; older persisted artifacts deserialize without one.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub owner_id: String,
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
    pub objective_status: TaskObjectiveStatus,
    /// Provider-authored claims remain distinct from canonical tool evidence.
    /// This bounded projection exists so parent synthesis does not depend on a
    /// model repeating acceptance evidence in its prose summary.
    #[serde(default)]
    pub handoff: Option<DelegatedTaskHandoff>,
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
    pub fn delegated_handoff(&self) -> Option<DelegatedTaskHandoff> {
        parse_delegated_handoff(&self.output)
    }

    /// Downgrade provider-authored completion when it omits any immutable
    /// acceptance requirement embedded in the durable task objective.
    pub(crate) fn enforce_acceptance_contract(&mut self, task_prompt: &str) {
        let Some(required) = parse_acceptance_contract(task_prompt) else {
            return;
        };
        let Some(mut handoff) = self.delegated_handoff() else {
            return;
        };
        let missing = required
            .required
            .iter()
            .filter(|requirement| {
                let required_id = normalize_check_id(&requirement.id);
                let provider_passed = handoff.acceptance_checks.iter().any(|check| {
                    normalize_check_id(&check.id) == required_id
                        && matches!(
                            check.status.trim().to_ascii_lowercase().as_str(),
                            "passed" | "pass" | "not_applicable" | "not-applicable"
                        )
                        && !check.evidence.trim().is_empty()
                });
                let required_proofs = acceptance_requirement_proofs(requirement);
                !provider_passed
                    || required_proofs
                        .iter()
                        .any(|proof| !self.evidence.has_acceptance_proof(proof))
            })
            .collect::<Vec<_>>();
        if missing.is_empty() {
            return;
        }

        handoff.status = TaskObjectiveStatus::Degraded;
        for requirement in &missing {
            handoff.remaining_work.push(format!(
                "Acceptance check '{}' was not passed with evidence: {}",
                requirement.id, requirement.criterion
            ));
        }
        handoff.remaining_work = bounded_strings(
            &handoff.remaining_work,
            MAX_HANDOFF_REMAINING_ITEMS,
            MAX_HANDOFF_REMAINING_ITEM_BYTES,
        );
        let Some(start) = self.output.rfind(HANDOFF_OPEN) else {
            return;
        };
        if let Ok(serialized) = serde_json::to_string(&handoff) {
            self.output.truncate(start);
            self.output.push_str(HANDOFF_OPEN);
            self.output.push_str(&serialized);
            self.output.push_str(HANDOFF_CLOSE);
        }
    }

    /// Return the provider-authored handoff under the runtime's fixed bounds.
    /// Durable replay and parent synthesis use this instead of raw child output.
    pub fn bounded_delegated_handoff(&self) -> Option<DelegatedTaskHandoff> {
        self.delegated_handoff().map(|handoff| handoff.bounded())
    }

    pub fn objective_status(&self) -> TaskObjectiveStatus {
        if self.termination == SubAgentTermination::Cancelled {
            return TaskObjectiveStatus::Blocked;
        }
        if self.termination == SubAgentTermination::Failed || self.error.is_some() {
            return TaskObjectiveStatus::Failed;
        }
        let Some(handoff) = self.delegated_handoff() else {
            return TaskObjectiveStatus::Degraded;
        };
        if handoff.is_complete() {
            TaskObjectiveStatus::Complete
        } else {
            match handoff.status {
                TaskObjectiveStatus::Complete | TaskObjectiveStatus::Degraded => {
                    TaskObjectiveStatus::Degraded
                }
                other => other,
            }
        }
    }

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
            objective_status: self.objective_status(),
            handoff: self.bounded_delegated_handoff(),
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
                "objective_status": self.objective_status(),
                "handoff": self.bounded_delegated_handoff(),
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
        self.success
            && self.error.is_none()
            && self.has_retained_evidence()
            && self.objective_status() != TaskObjectiveStatus::Failed
    }

    /// Whether an isolated workspace is allowed to publish this result back to
    /// the authoritative project. Clean completions remain publishable even
    /// when they are legitimate no-ops. Interrupted results must prove an
    /// actual mutation; read-only observations are useful to the parent but
    /// cannot satisfy a downstream file dependency.
    pub fn is_eligible_for_isolated_integration(&self) -> bool {
        self.success
            || (self.termination.is_degraded_interruption()
                && self.has_partial_evidence()
                && self.evidence.mutations > 0)
    }

    pub fn is_degraded_success(&self) -> bool {
        (self.termination.is_degraded_interruption() && self.has_partial_evidence())
            || (self.success && self.objective_status() != TaskObjectiveStatus::Complete)
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

#[derive(Debug, Deserialize)]
struct DelegatedAcceptanceContract {
    required: Vec<DelegatedAcceptanceRequirement>,
}

#[derive(Debug, Deserialize)]
struct DelegatedAcceptanceRequirement {
    id: String,
    criterion: String,
    #[serde(default)]
    evidence_kind: Option<String>,
}

fn parse_acceptance_contract(prompt: &str) -> Option<DelegatedAcceptanceContract> {
    if prompt.matches(ACCEPTANCE_CONTRACT_OPEN).count() != 1
        || prompt.matches(ACCEPTANCE_CONTRACT_CLOSE).count() != 1
    {
        return None;
    }
    let start = prompt.find(ACCEPTANCE_CONTRACT_OPEN)? + ACCEPTANCE_CONTRACT_OPEN.len();
    let end = prompt[start..].find(ACCEPTANCE_CONTRACT_CLOSE)? + start;
    serde_json::from_str(prompt[start..end].trim()).ok()
}

fn acceptance_requirement_proofs(
    requirement: &DelegatedAcceptanceRequirement,
) -> Vec<&'static str> {
    let required_id = normalize_check_id(&requirement.id);
    let criterion = requirement.criterion.to_ascii_lowercase();
    let inferred_browser_required = required_id.contains("browser")
        || criterion.contains("browser")
        || criterion.contains("page-check")
        || criterion.contains("page check")
        || criterion.contains("live-page")
        || criterion.contains("live page");
    let requirement_text = format!("{required_id} {criterion}");
    let mut proofs = Vec::new();
    match requirement.evidence_kind.as_deref() {
        Some("browser_keyboard") => proofs.push("browser_keyboard"),
        Some("browser_touch") => proofs.push("browser_touch"),
        Some("browser_runtime") => proofs.push("browser_runtime"),
        Some("handoff") => {}
        _ if inferred_browser_required => {
            if requirement_text.contains("keyboard") {
                proofs.push("browser_keyboard");
            }
            if requirement_text.contains("touch") {
                proofs.push("browser_touch");
            }
            if proofs.is_empty() {
                proofs.push("browser_runtime");
            }
        }
        _ => {}
    }
    proofs
}

pub(crate) fn missing_required_browser_acceptance_proofs(
    prompt: &str,
    evidence: &DelegatedEvidenceSummary,
) -> Vec<&'static str> {
    let Some(contract) = parse_acceptance_contract(prompt) else {
        return Vec::new();
    };
    let mut missing = contract
        .required
        .iter()
        .flat_map(acceptance_requirement_proofs)
        .filter(|proof| !evidence.has_acceptance_proof(proof))
        .collect::<Vec<_>>();
    missing.sort_unstable();
    missing.dedup();
    missing
}

fn normalize_check_id(id: &str) -> String {
    id.trim().to_ascii_lowercase().replace([' ', '-'], "_")
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

fn bounded_strings(values: &[String], limit: usize, max_bytes: usize) -> Vec<String> {
    values
        .iter()
        .take(limit)
        .map(|value| truncate_preview(value, max_bytes))
        .collect()
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
        missing_required_browser_acceptance_proofs, parse_delegated_handoff, DelegatedEvidenceKind,
        DelegatedEvidenceSummary, DelegatedProcessArtifact, SubAgentResult, SubAgentTermination,
        TaskObjectiveStatus,
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
    fn structured_handoff_distinguishes_objective_completion_from_loop_termination() {
        let mut complete = base_result();
        complete.output = r#"Implemented and verified the task.
<delegated_handoff>{"status":"complete","summary":"done","acceptance_checks":[{"id":"tests","status":"passed","evidence":"focused tests passed"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();
        complete
            .evidence
            .record_success(DelegatedEvidenceKind::Mutation);
        assert_eq!(complete.objective_status(), TaskObjectiveStatus::Complete);
        assert_eq!(complete.evidence_json()["objective_status"], "complete");
        assert_eq!(
            complete.evidence_json()["handoff"]["acceptance_checks"][0]["evidence"],
            "focused tests passed"
        );

        let mut incomplete = complete.clone();
        incomplete.output = r#"Work remains.
<delegated_handoff>{"status":"degraded","summary":"partial","acceptance_checks":[{"id":"build","status":"failed","evidence":"compile error"}],"remaining_work":["fix the build"],"blockers":[]}</delegated_handoff>"#.to_string();
        assert_eq!(incomplete.objective_status(), TaskObjectiveStatus::Degraded);
        assert!(incomplete.is_degraded_success());
    }

    #[test]
    fn empty_acceptance_evidence_cannot_claim_complete() {
        let mut result = base_result();
        result.output = r#"<delegated_handoff>{"status":"complete","summary":"claimed done","acceptance_checks":[],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();

        assert_eq!(result.objective_status(), TaskObjectiveStatus::Degraded);
    }

    #[test]
    fn required_acceptance_contract_downgrades_missing_runtime_proof() {
        let mut result = base_result();
        result.output = r#"<delegated_handoff>{"status":"complete","summary":"tests and build passed","acceptance_checks":[{"id":"tests","status":"passed","evidence":"7 tests"},{"id":"build","status":"passed","evidence":"dist emitted"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();
        let prompt = r#"Verify the product.
<delegated_acceptance_contract>{"required":[{"id":"tests","criterion":"tests pass"},{"id":"browser_runtime","criterion":"primary interaction works in a browser without console errors"}]}</delegated_acceptance_contract>"#;

        result.enforce_acceptance_contract(prompt);

        assert_eq!(result.objective_status(), TaskObjectiveStatus::Degraded);
        let handoff = result.delegated_handoff().expect("rewritten handoff");
        assert!(handoff
            .remaining_work
            .iter()
            .any(|item| item.contains("browser_runtime")));
    }

    #[test]
    fn required_acceptance_contract_accepts_normalized_ids_with_evidence() {
        let mut result = base_result();
        result.output = r#"<delegated_handoff>{"status":"complete","summary":"runtime proved","acceptance_checks":[{"id":"Browser Runtime","status":"passed","evidence":"phone and desktop interaction passed with zero console errors"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();
        let prompt = r#"<delegated_acceptance_contract>{"required":[{"id":"browser-runtime","criterion":"runtime works"}]}</delegated_acceptance_contract>"#;

        result.evidence.record_acceptance_proof("browser_runtime");
        result.enforce_acceptance_contract(prompt);

        assert_eq!(result.objective_status(), TaskObjectiveStatus::Complete);
    }

    #[test]
    fn browser_acceptance_claim_requires_canonical_browser_tool_proof() {
        let mut result = base_result();
        result.output = r#"<delegated_handoff>{"status":"complete","summary":"claimed browser pass","acceptance_checks":[{"id":"browser_runtime","status":"passed","evidence":"looked good"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();
        let prompt = r#"<delegated_acceptance_contract>{"required":[{"id":"browser_runtime","criterion":"browser interaction works without errors"}]}</delegated_acceptance_contract>"#;

        result.enforce_acceptance_contract(prompt);
        assert_eq!(result.objective_status(), TaskObjectiveStatus::Degraded);

        let mut proved = base_result();
        proved.output = r#"<delegated_handoff>{"status":"complete","summary":"proved browser pass","acceptance_checks":[{"id":"browser_runtime","status":"passed","evidence":"browser_check passed phone and desktop with zero errors"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();
        proved.evidence.record_acceptance_proof("browser_runtime");
        proved.enforce_acceptance_contract(prompt);
        assert_eq!(proved.objective_status(), TaskObjectiveStatus::Complete);
    }

    #[test]
    fn browser_modality_claims_require_matching_canonical_actions() {
        let prompt = r#"<delegated_acceptance_contract>{"required":[{"id":"interactive","criterion":"desktop keyboard and phone touch interactions work in a browser"}]}</delegated_acceptance_contract>"#;
        let output = r#"<delegated_handoff>{"status":"complete","summary":"claimed interactions","acceptance_checks":[{"id":"interactive","status":"passed","evidence":"browser run passed"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#;

        let mut click_only = base_result();
        click_only.output = output.to_string();
        click_only
            .evidence
            .record_acceptance_proof("browser_runtime");
        click_only.evidence.record_acceptance_proof("browser_touch");
        click_only.enforce_acceptance_contract(prompt);
        assert_eq!(click_only.objective_status(), TaskObjectiveStatus::Degraded);

        let mut complete = base_result();
        complete.output = output.to_string();
        complete.evidence.record_acceptance_proof("browser_runtime");
        complete.evidence.record_acceptance_proof("browser_touch");
        complete
            .evidence
            .record_acceptance_proof("browser_keyboard");
        complete.enforce_acceptance_contract(prompt);
        assert_eq!(complete.objective_status(), TaskObjectiveStatus::Complete);
    }

    #[test]
    fn implementation_controls_do_not_imply_browser_execution() {
        let mut result = base_result();
        result.output = r#"<delegated_handoff>{"status":"complete","summary":"UI implemented","acceptance_checks":[{"id":"interface-playable","status":"passed","evidence":"keyboard and touch controls implemented in interface-owned files"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();
        let prompt = r#"<delegated_acceptance_contract>{"required":[{"id":"interface-playable","criterion":"Keyboard and touch controls are implemented in interface-owned files."}]}</delegated_acceptance_contract>"#;

        result.enforce_acceptance_contract(prompt);

        assert_eq!(result.objective_status(), TaskObjectiveStatus::Complete);
    }

    #[test]
    fn explicit_browser_evidence_kind_does_not_depend_on_wording() {
        let prompt = r#"<delegated_acceptance_contract>{"required":[{"id":"primary-journey","criterion":"The primary journey is proven.","evidence_kind":"browser_runtime"}]}</delegated_acceptance_contract>"#;
        let output = r#"<delegated_handoff>{"status":"complete","summary":"journey passed","acceptance_checks":[{"id":"primary-journey","status":"passed","evidence":"governed check passed"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#;
        let mut result = base_result();
        result.output = output.to_string();
        result.enforce_acceptance_contract(prompt);
        assert_eq!(result.objective_status(), TaskObjectiveStatus::Degraded);

        let mut proved = base_result();
        proved.output = output.to_string();
        proved.evidence.record_acceptance_proof("browser_runtime");
        proved.enforce_acceptance_contract(prompt);
        assert_eq!(proved.objective_status(), TaskObjectiveStatus::Complete);
    }

    #[test]
    fn missing_browser_proofs_are_deduplicated_for_runtime_correction() {
        let prompt = r#"<delegated_acceptance_contract>{"required":[{"id":"journey","criterion":"Works","evidence_kind":"browser_runtime"},{"id":"page","criterion":"Browser works"}]}</delegated_acceptance_contract>"#;
        let evidence = DelegatedEvidenceSummary::default();

        assert_eq!(
            missing_required_browser_acceptance_proofs(prompt, &evidence),
            vec!["browser_runtime"]
        );

        let mut proved = evidence;
        proved.record_acceptance_proof("browser_runtime");
        assert!(missing_required_browser_acceptance_proofs(prompt, &proved).is_empty());
    }

    #[test]
    fn luna_shaped_acceptance_evidence_survives_without_summary_duplication() {
        let mut result = base_result();
        result.output = r#"<explore_report>
{
  "summary": "Marker read and environment status confirmed.",
  "paths_examined": ["proof/alpha.txt"],
  "files_examined": ["proof/alpha.txt"],
  "key_findings": ["Environment status confirmed as ENV_ABSENT."],
  "design_patterns": [],
  "concerns": [],
  "confidence": "medium"
}
</explore_report>
<delegated_handoff>{"status":"complete","summary":"proof complete","acceptance_checks":[{"id":"marker","status":"passed","evidence":"alpha-live-proof"},{"id":"environment","status":"passed","evidence":"ENV_ABSENT"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();

        let artifact = result.evidence_artifact();
        assert!(!artifact.summary.contains("alpha-live-proof"));
        let handoff = artifact.handoff.expect("bounded handoff");
        assert_eq!(handoff.acceptance_checks[0].evidence, "alpha-live-proof");
        assert_eq!(handoff.acceptance_checks[1].evidence, "ENV_ABSENT");
    }

    #[test]
    fn provider_handoff_projection_is_utf8_safe_and_bounded() {
        let mut result = base_result();
        let checks = (0..12)
            .map(|index| {
                json!({
                    "id": format!("check-{index}-{}", "i".repeat(500)),
                    "status": "passed".repeat(100),
                    "evidence": "🐝".repeat(1_000),
                })
            })
            .collect::<Vec<_>>();
        let handoff = json!({
            "status": "complete",
            "summary": "s".repeat(4_000),
            "acceptance_checks": checks,
            "remaining_work": (0..20).map(|_| "r".repeat(2_000)).collect::<Vec<_>>(),
            "blockers": (0..20).map(|_| "b".repeat(2_000)).collect::<Vec<_>>(),
            "generated_artifacts": (0..20).map(|index| json!({
                "path": format!("artifact-{index}-{}", "p".repeat(2_000)),
                "purpose": "purpose".repeat(1_000),
            })).collect::<Vec<_>>(),
        });
        result.output = format!("<delegated_handoff>{handoff}</delegated_handoff>");

        let bounded = result.bounded_delegated_handoff().expect("bounded handoff");
        assert_eq!(bounded.acceptance_checks.len(), 8);
        assert_eq!(bounded.remaining_work.len(), 8);
        assert_eq!(bounded.blockers.len(), 8);
        assert_eq!(bounded.generated_artifacts.len(), 16);
        assert!(bounded.summary.len() <= 1_200);
        assert!(bounded
            .acceptance_checks
            .iter()
            .all(|check| check.id.len() <= 128
                && check.status.len() <= 32
                && check.evidence.len() <= 1_200
                && std::str::from_utf8(check.evidence.as_bytes()).is_ok()));
    }

    #[test]
    fn complete_claim_with_failed_check_is_canonically_degraded() {
        let mut result = base_result();
        result.output = r#"<delegated_handoff>{"status":"complete","summary":"claimed done","acceptance_checks":[{"id":"live","status":"failed","evidence":"no listener"}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#.to_string();
        result
            .evidence
            .record_success(DelegatedEvidenceKind::Execution);

        assert_eq!(result.objective_status(), TaskObjectiveStatus::Degraded);
    }

    #[test]
    fn handoff_must_be_one_final_machine_readable_block() {
        let valid = r#"summary
<delegated_handoff>{"status":"complete","summary":"done","acceptance_checks":[],"remaining_work":[],"blockers":[]}</delegated_handoff>"#;
        assert!(parse_delegated_handoff(valid).is_some());
        assert!(parse_delegated_handoff(&format!("{valid}\ntrailing claim")).is_none());
        assert!(parse_delegated_handoff(&format!("{valid}\n{valid}")).is_none());
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
        assert!(
            !result.is_eligible_for_isolated_integration(),
            "read-only partial evidence cannot release a file dependency"
        );
    }

    #[test]
    fn interrupted_mutation_can_publish_its_partial_isolated_patch() {
        let mut result = base_result();
        result.success = false;
        result.error = Some("Provider call timed out".to_string());
        result.termination = SubAgentTermination::ProviderTimeout;
        result
            .evidence
            .record_success(DelegatedEvidenceKind::Mutation);

        assert!(result.has_partial_evidence());
        assert!(result.is_eligible_for_isolated_integration());
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
</explore_report>
<delegated_handoff>{"status":"complete","summary":"Investigation complete.","acceptance_checks":[{"id":"report","status":"passed","evidence":"Structured explore report includes examined files and findings."}],"remaining_work":[],"blockers":[]}</delegated_handoff>"#
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
            owner_id: "owner-a:hive:task".to_string(),
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
