//! Workspace context builder for ACP
//!
//! Provides cached workspace context to avoid repeated blocking I/O operations.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// Cached workspace context
#[derive(Clone, Debug)]
pub struct WorkspaceContext {
    pub context: String,
    pub workspace_path: PathBuf,
    pub cached_at: chrono::DateTime<chrono::Utc>,
}

/// Workspace context builder with caching
pub struct WorkspaceContextBuilder {
    /// Cached context to avoid repeated I/O
    cache: Arc<RwLock<Option<WorkspaceContext>>>,
    /// Cache TTL in seconds (default: 5 minutes)
    cache_ttl_seconds: u64,
}

impl WorkspaceContextBuilder {
    /// Create a new workspace context builder
    pub fn new() -> Self {
        Self {
            cache: Arc::new(RwLock::new(None)),
            cache_ttl_seconds: 300, // 5 minutes
        }
    }

    /// Set cache TTL in seconds
    pub fn with_cache_ttl(mut self, ttl_seconds: u64) -> Self {
        self.cache_ttl_seconds = ttl_seconds;
        self
    }

    /// Get workspace context, using cache if fresh
    pub async fn get_context(&self, cwd: &Path) -> String {
        // Check cache first
        {
            let cache_read = self.cache.read().await;
            if let Some(cached) = cache_read.as_ref() {
                // Check if cache is still valid
                let age = chrono::Utc::now() - cached.cached_at;
                let age_seconds = age.num_seconds().unsigned_abs();
                if cached.workspace_path == cwd && age_seconds < self.cache_ttl_seconds {
                    debug!("Using cached workspace context (age: {}s)", age_seconds);
                    return cached.context.clone();
                }
            }
        }

        // Cache miss or expired - build fresh context
        debug!("Building fresh workspace context for {:?}", cwd);
        let context = self.build_context_blocking(cwd).await;

        // Update cache
        {
            let mut cache_write = self.cache.write().await;
            *cache_write = Some(WorkspaceContext {
                context: context.clone(),
                workspace_path: cwd.to_path_buf(),
                cached_at: chrono::Utc::now(),
            });
        }

        context
    }

    /// Invalidate the cache (e.g., when directory structure changes)
    pub async fn invalidate(&self) {
        let mut cache_write = self.cache.write().await;
        *cache_write = None;
        debug!("Workspace context cache invalidated");
    }

    /// Build workspace context in blocking thread to avoid blocking async runtime
    async fn build_context_blocking(&self, cwd: &Path) -> String {
        let cwd = cwd.to_path_buf();
        let cwd_display = cwd.display().to_string();

        tokio::task::spawn_blocking(move || build_workspace_context(&cwd))
            .await
            .unwrap_or_else(|e| {
                tracing::error!("Failed to build workspace context: {}", e);
                format!(
                    "## Workspace Context\n\nWorking directory: {}\n\n(context unavailable)",
                    cwd_display
                )
            })
    }
}

impl Default for WorkspaceContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Build workspace context for the AI
///
/// Provides information about:
/// - Working directory path
/// - Project type (based on config files)
/// - Directory structure (top-level)
/// - Key files present
fn build_workspace_context(cwd: &Path) -> String {
    use std::fs;

    let mut context = String::new();
    context.push_str(&format!(
        "## Workspace Context\n\nWorking directory: {}\n\n",
        cwd.display()
    ));

    // Check for project type indicators
    let mut project_indicators = Vec::new();

    let config_files = [
        ("Cargo.toml", "Rust (Cargo)"),
        ("package.json", "Node.js/JavaScript"),
        ("pyproject.toml", "Python (pyproject)"),
        ("setup.py", "Python (setup.py)"),
        ("go.mod", "Go"),
        ("pom.xml", "Java (Maven)"),
        ("build.gradle", "Java/Kotlin (Gradle)"),
        ("Makefile", "Make"),
        ("CMakeLists.txt", "C/C++ (CMake)"),
        ("Gemfile", "Ruby"),
        ("composer.json", "PHP"),
        ("pubspec.yaml", "Dart/Flutter"),
        (".git", "Git repository"),
    ];

    for (file, project_type) in config_files {
        if cwd.join(file).exists() {
            project_indicators.push(project_type);
        }
    }

    if !project_indicators.is_empty() {
        context.push_str("Project type: ");
        context.push_str(&project_indicators.join(", "));
        context.push_str("\n\n");
    }

    // List top-level directory contents
    context.push_str("### Directory contents:\n```\n");

    if let Ok(entries) = fs::read_dir(cwd) {
        let mut items: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                // Skip hidden files except .git
                if name.starts_with('.') && name != ".git" {
                    return None;
                }
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                Some(if is_dir { format!("{}/", name) } else { name })
            })
            .collect();

        items.sort();

        // Limit to first 50 items to avoid overwhelming context
        let display_count = items.len().min(50);
        for item in items.iter().take(display_count) {
            context.push_str(item);
            context.push('\n');
        }
        if items.len() > 50 {
            context.push_str(&format!("... and {} more items\n", items.len() - 50));
        }
    } else {
        context.push_str("(unable to read directory)\n");
    }

    context.push_str("```\n");

    // Add git branch info if available
    if cwd.join(".git").exists() {
        if let Ok(output) = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(cwd)
            .output()
        {
            if output.status.success() {
                let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !branch.is_empty() {
                    context.push_str(&format!("\nGit branch: {}\n", branch));
                }
            }
        }
    }

    context
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_workspace_context() {
        let temp_dir = tempfile::tempdir().unwrap();
        let context = build_workspace_context(temp_dir.path());

        assert!(context.contains("Workspace Context"));
        assert!(context.contains(&temp_dir.path().display().to_string()));
    }

    #[test]
    fn test_workspace_context_caching() {
        let builder = WorkspaceContextBuilder::new().with_cache_ttl(10); // 10 seconds for testing

        let temp_dir = tempfile::tempdir().unwrap();

        // First call should build context
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ctx1 = rt.block_on(async { builder.get_context(temp_dir.path()).await });

        // Second call should use cache
        let ctx2 = rt.block_on(async { builder.get_context(temp_dir.path()).await });

        assert_eq!(ctx1, ctx2);
    }
}
