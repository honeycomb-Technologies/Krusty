use serde_json::Value;
use std::path::Path;

use super::super::types::{
    parse_explore_report, render_explore_report, summary_looks_non_substantive,
    synthesize_explore_report, synthesize_explore_report_from_paths, SubAgentResult, SubAgentTask,
};
use super::governance::delegated_is_explore;

pub(super) fn completion_summary_preview(text: &str) -> Option<String> {
    let lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(4)
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return None;
    }

    let summary = lines.join(" ");
    if summary.len() <= 600 {
        Some(summary)
    } else {
        let mut end = 600;
        while end > 0 && !summary.is_char_boundary(end) {
            end -= 1;
        }
        Some(format!("{}...", &summary[..end]))
    }
}

pub(super) fn should_replace_forced_summary(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return true;
    }

    if let Some(report) = parse_explore_report(trimmed) {
        return summary_looks_non_substantive(&report.summary)
            || report
                .key_findings
                .iter()
                .all(|finding| summary_looks_non_substantive(finding));
    }

    summary_looks_non_substantive(trimmed)
}

pub(super) fn synthesized_explorer_output(
    task_name: &str,
    final_output: &str,
    files_examined: &[String],
) -> String {
    if final_output.trim().is_empty() || should_replace_forced_summary(final_output) {
        synthesize_explore_report_from_paths(task_name, files_examined)
            .map(|report| render_explore_report(&report))
            .unwrap_or_else(|| fallback_explorer_summary(files_examined))
    } else {
        final_output.to_string()
    }
}

pub(super) fn normalize_explorer_result(
    mut result: SubAgentResult,
    task: &SubAgentTask,
) -> SubAgentResult {
    if result.delegated_run_id.is_none() {
        result.delegated_run_id = task.delegated_run_id.clone();
    }
    if delegated_is_explore(task) {
        if let Some(report) = parse_explore_report(&result.output) {
            for path in report.paths_examined {
                if !path.trim().is_empty()
                    && !result
                        .files_examined
                        .iter()
                        .any(|existing| existing == &path)
                {
                    result.files_examined.push(path);
                }
            }
            for file in report.files_examined {
                if !file.trim().is_empty()
                    && !result
                        .files_examined
                        .iter()
                        .any(|existing| existing == &file)
                {
                    result.files_examined.push(file);
                }
            }
            if !result.has_usable_evidence() {
                if let Some(report) =
                    synthesize_explore_report_from_paths(&task.name, &result.files_examined)
                {
                    let synthesized_paths = report.paths_examined.clone();
                    result.output = render_explore_report(&report);
                    result.files_examined = synthesized_paths;
                }
            }
        } else if result.success {
            if let Some(report) = synthesize_explore_report(&result.output, &result.files_examined)
            {
                let synthesized_paths = report.paths_examined.clone();
                result.output = render_explore_report(&report);
                result.files_examined = synthesized_paths;
            } else if let Some(report) =
                synthesize_explore_report_from_paths(&task.name, &result.files_examined)
            {
                let synthesized_paths = report.paths_examined.clone();
                result.output = render_explore_report(&report);
                result.files_examined = synthesized_paths;
            } else {
                result.success = false;
                result.error = Some(
                    "Delegated exploration finished without the required structured explore report"
                        .to_string(),
                );
            }
        }
    }

    if delegated_is_explore(task) && result.success && !result.has_usable_evidence() {
        result.success = false;
        result.error = Some("Delegated exploration completed without usable evidence".to_string());
    }
    result
}

pub(super) fn relative_or_display(path: &str, working_dir: &Path) -> String {
    let candidate = std::path::Path::new(path);
    let display = candidate
        .strip_prefix(working_dir)
        .unwrap_or(candidate)
        .display()
        .to_string();
    if display.is_empty() {
        ".".to_string()
    } else {
        display
    }
}

pub(super) fn collect_paths_from_tool_result(
    name: &str,
    output: &str,
    working_dir: &Path,
) -> Vec<String> {
    let parsed =
        serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()));
    let payload = parsed
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&parsed);

    let mut paths = Vec::new();
    match name {
        "read" => {
            if let Some(path) = payload.get("file_path").and_then(|value| value.as_str()) {
                paths.push(relative_or_display(path, working_dir));
            }
        }
        "glob" => {
            // A successful empty glob is still canonical negative evidence for
            // the directory that was searched. Retain the base path so an
            // explorer can synthesize a truthful report even when there are no
            // matching files to add below.
            if let Some(search_path) = payload.get("search_path").and_then(|value| value.as_str()) {
                paths.push(relative_or_display(search_path, working_dir));
            }
            if let Some(matches) = payload.get("matches").and_then(|value| value.as_array()) {
                for entry in matches.iter().take(12) {
                    if let Some(path) = entry.as_str() {
                        paths.push(relative_or_display(path, working_dir));
                    }
                }
            }
        }
        "grep" => {
            if let Some(matches) = payload.get("matches").and_then(|value| value.as_array()) {
                for entry in matches.iter().take(12) {
                    if let Some(path) = entry.get("file").and_then(|value| value.as_str()) {
                        paths.push(relative_or_display(path, working_dir));
                    }
                }
            }
        }
        "list" => {
            if let Some(output) = payload.get("output").and_then(|value| value.as_str()) {
                for line in output.lines().take(12) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        paths.push(trimmed.to_string());
                    }
                }
            }
        }
        _ => {}
    }

    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    unique
}

pub(super) fn text_claims_tool_empty(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    let markers = [
        "no output",
        "nothing is working",
        "nothing worked",
        "returned no results",
        "returning no results",
        "returning empty results",
        "tools are returning empty",
        "every tool is returning empty",
        "unable to locate any files",
        "no files found",
    ];

    markers.iter().any(|marker| normalized.contains(marker))
}

pub(super) fn tool_result_has_positive_evidence(name: &str, output: &str, is_error: bool) -> bool {
    if is_error {
        return false;
    }

    let parsed =
        serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()));
    let payload = parsed
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&parsed);

    match name {
        "read" => payload
            .get("content")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty()),
        "glob" => payload
            .get("count")
            .and_then(|value| value.as_u64())
            .is_some_and(|count| count > 0),
        "grep" => {
            payload
                .get("count")
                .and_then(|value| value.as_u64())
                .is_some_and(|count| count > 0)
                || payload
                    .get("total_matches")
                    .and_then(|value| value.as_u64())
                    .is_some_and(|count| count > 0)
        }
        "list" => payload
            .get("total_entries")
            .and_then(|value| value.as_u64())
            .is_some_and(|count| count > 0),
        "bash" => payload
            .get("output_preview")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.trim().is_empty()),
        _ => false,
    }
}

pub(super) fn timeout_partial_output(final_output: &str, files_examined: &[String]) -> String {
    if final_output.trim().is_empty() && !files_examined.is_empty() {
        format!(
            "Sub-agent timed out before producing final output. {} files were examined: {}",
            files_examined.len(),
            files_examined
                .iter()
                .take(20)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        )
    } else {
        final_output.to_string()
    }
}

fn fallback_explorer_summary(files_examined: &[String]) -> String {
    let files = files_examined
        .iter()
        .filter(|path| !path.trim().is_empty())
        .take(8)
        .cloned()
        .collect::<Vec<_>>();

    if files.is_empty() {
        "Exploration converged without a final synthesized answer. Summarize the evidence gathered so far before requesting a narrower follow-up.".to_string()
    } else {
        format!(
            "Exploration converged after repeated low-yield read-only cycles. Use the gathered evidence to summarize the codebase based on these key files: {}.",
            files.join(", ")
        )
    }
}
