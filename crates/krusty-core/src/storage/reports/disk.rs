use anyhow::{Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};

use super::model::Report;
use crate::paths;

pub fn promote_report_content(report: &Report) -> String {
    let summary = report.summary.trim();
    if !summary.is_empty() {
        return summary.to_string();
    }

    let mut collapsed = report
        .content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.len() > 600 {
        collapsed.truncate(600);
        collapsed.push_str("...");
    }
    collapsed
}

pub(super) fn slugify(title: &str) -> String {
    let slug = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");

    if slug.is_empty() {
        "report".to_string()
    } else {
        slug
    }
}

#[derive(Serialize)]
struct ReportFrontmatter<'a> {
    title: &'a str,
    created: &'a str,
    session_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_dir: Option<&'a str>,
    #[serde(skip_serializing_if = "slice_is_empty")]
    tags: &'a [String],
    #[serde(skip_serializing_if = "slice_is_empty")]
    sources: &'a [String],
}

fn slice_is_empty(values: &[String]) -> bool {
    values.is_empty()
}

pub(super) fn write_report_to_disk(report: &Report, report_root: Option<&Path>) -> Result<PathBuf> {
    let reports_dir = report_root
        .map(paths::project_reports_dir)
        .unwrap_or_else(|| paths::config_dir().join("reports"));
    std::fs::create_dir_all(&reports_dir).context("creating reports directory")?;

    let path = next_report_path(report, &reports_dir);
    let markdown = render_report_markdown(report)?;

    std::fs::write(&path, markdown).context("writing report file")?;
    Ok(path)
}

fn report_date_prefix(created_at: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| chrono::Utc::now().format("%Y-%m-%d").to_string())
}

fn next_report_path(report: &Report, reports_dir: &Path) -> PathBuf {
    let date = report_date_prefix(&report.created_at);
    let slug = slugify(&report.title);
    let base = reports_dir.join(format!("{date}-{slug}.md"));
    if !base.exists() {
        return base;
    }

    let short_id: String = report.id.chars().filter(|c| *c != '-').take(8).collect();
    let fallback = reports_dir.join(format!("{date}-{slug}-{short_id}.md"));
    if !fallback.exists() {
        return fallback;
    }

    for index in 2.. {
        let candidate = reports_dir.join(format!("{date}-{slug}-{short_id}-{index}.md"));
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("report path search should always find an available file name");
}

fn render_report_markdown(report: &Report) -> Result<String> {
    let frontmatter = ReportFrontmatter {
        title: &report.title,
        created: &report.created_at,
        session_id: &report.session_id,
        project_dir: report.project_dir.as_deref(),
        tags: &report.tags,
        sources: &report.sources,
    };
    let yaml = serde_yaml::to_string(&frontmatter).context("serializing report frontmatter")?;
    let yaml = yaml.strip_prefix("---\n").unwrap_or(yaml.as_str());
    let body = report.content.trim_start_matches('\n');
    Ok(format!("---\n{}---\n\n{}", yaml, body))
}
