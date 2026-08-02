use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::fs;
use tokio::process::Command;

const DIAGNOSTIC_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DIAGNOSTIC_CHARS: usize = 2_000;

pub(super) async fn collect_mutation_warnings(
    paths: &[PathBuf],
    working_dir: &Path,
) -> Vec<String> {
    let mut warnings = Vec::new();

    for path in paths.iter().filter(|path| path.is_file()) {
        let Some(extension) = path.extension().and_then(|value| value.to_str()) else {
            continue;
        };
        let extension = extension.to_ascii_lowercase();
        if !matches!(extension.as_str(), "json" | "toml" | "yaml" | "yml") {
            continue;
        }

        let Ok(content) = fs::read_to_string(path).await else {
            continue;
        };
        let parse_error = match extension.as_str() {
            "json" => serde_json::from_str::<serde_json::Value>(&content)
                .err()
                .map(|error| error.to_string()),
            "toml" => toml::from_str::<toml::Value>(&content)
                .err()
                .map(|error| error.to_string()),
            "yaml" | "yml" => serde_yaml::from_str::<serde_yaml::Value>(&content)
                .err()
                .map(|error| error.to_string()),
            _ => None,
        };
        if let Some(error) = parse_error {
            warnings.push(format!(
                "Syntax diagnostic for {}: {}",
                path.display(),
                truncate(&error)
            ));
        }
    }

    if paths.is_empty() {
        return warnings;
    }

    let mut command = Command::new("git");
    command
        .arg("diff")
        .arg("--check")
        .arg("--")
        .args(paths)
        .current_dir(working_dir)
        .kill_on_drop(true);

    if let Ok(Ok(output)) = tokio::time::timeout(DIAGNOSTIC_TIMEOUT, command.output()).await {
        if !output.status.success() {
            let diagnostic = String::from_utf8_lossy(&output.stdout);
            let fallback = String::from_utf8_lossy(&output.stderr);
            let diagnostic = if diagnostic.trim().is_empty() {
                fallback.trim()
            } else {
                diagnostic.trim()
            };
            if !diagnostic.is_empty()
                && !diagnostic
                    .to_ascii_lowercase()
                    .contains("not a git repository")
            {
                warnings.push(format!("git diff --check: {}", truncate(diagnostic)));
            }
        }
    }

    warnings
}

fn truncate(value: &str) -> String {
    if value.chars().count() <= MAX_DIAGNOSTIC_CHARS {
        return value.to_string();
    }
    let prefix = value.chars().take(MAX_DIAGNOSTIC_CHARS).collect::<String>();
    format!("{prefix}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_structured_file_returns_actionable_warning() {
        let temp = tempfile::TempDir::new().expect("temp dir should create");
        let path = temp.path().join("broken.json");
        fs::write(&path, "{broken")
            .await
            .expect("fixture should write");

        let warnings = collect_mutation_warnings(&[path], temp.path()).await;

        assert!(warnings
            .iter()
            .any(|warning| warning.contains("Syntax diagnostic")));
    }

    #[tokio::test]
    async fn non_git_workspace_does_not_emit_git_usage_noise() {
        let temp = tempfile::TempDir::new().expect("temp dir should create");
        let path = temp.path().join("valid.json");
        fs::write(&path, r#"{"status":"ok"}"#)
            .await
            .expect("fixture should write");

        let warnings = collect_mutation_warnings(&[path], temp.path()).await;

        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }
}
