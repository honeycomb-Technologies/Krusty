use serde::{Deserialize, Serialize};
use serde_json::json;

/// Structured delegated explore output.
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

fn strip_think_blocks(text: &str) -> String {
    let mut cleaned = text.to_string();
    while let Some(start) = cleaned.find("<think>") {
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

fn extract_tagged_json<'a>(text: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = text.find(&open)?;
    let end = text[start + open.len()..].find(&close)?;
    Some(text[start + open.len()..start + open.len() + end].trim())
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

#[cfg(test)]
mod tests {
    use super::{
        parse_explore_report, synthesize_explore_report, synthesize_explore_report_from_paths,
    };

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
            "mitsuro-core/src",
            &[
                "agent/".to_string(),
                "storage/".to_string(),
                "agent/orchestrator.rs".to_string(),
            ],
        )
        .expect("report");

        assert!(report.summary.contains("mitsuro-core/src"));
        assert_eq!(report.paths_examined.len(), 3);
        assert!(!report.key_findings.is_empty());
    }
}
