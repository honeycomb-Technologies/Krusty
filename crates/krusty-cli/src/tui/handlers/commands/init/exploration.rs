use super::template::generate_krab_template;
use crate::tui::app::App;

impl App {
    /// Generate a basic KRAB.md template without AI (fallback).
    pub(super) fn generate_basic_krab_template(&mut self) {
        let krab_path = self.runtime.working_dir.join("KRAB.md");
        let is_regenerate = krab_path.exists();

        let project_name = self.runtime.working_dir.file_name().map_or_else(
            || "Project".to_string(),
            |n| n.to_string_lossy().into_owned(),
        );

        let languages = self.detect_project_languages();
        let structure = self.detect_project_structure();
        let content = generate_krab_template(&project_name, &languages, &structure);

        match std::fs::write(&krab_path, &content) {
            Ok(_) => {
                let action = if is_regenerate {
                    "Regenerated"
                } else {
                    "Created"
                };
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!(
                        "{} KRAB.md ({} bytes) - basic template\n\n\
                        Note: Authenticate with /auth for AI-powered analysis.",
                        action,
                        content.len()
                    ),
                ));
            }
            Err(e) => {
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    format!("Failed to write KRAB.md: {}", e),
                ));
            }
        }
    }

    /// Start async codebase exploration for /init.
    pub(super) fn start_init_exploration(&mut self) {
        use crate::agent::subagent::{SubAgentPool, SubAgentTask};
        use crate::tui::utils::InitExplorationResult;
        use std::sync::Arc;

        let client = match self.create_ai_client() {
            Some(c) => Arc::new(c),
            None => {
                self.runtime.chat.messages.push((
                    "system".to_string(),
                    "Failed to create AI client for exploration".to_string(),
                ));
                return;
            }
        };

        let working_dir = self.runtime.working_dir.clone();
        let cancellation = self.runtime.cancellation.clone();
        let current_model = self.runtime.current_model.clone();

        self.runtime.cached_init_languages = Some(self.detect_project_languages());

        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        self.runtime.channels.init_exploration = Some(result_rx);

        let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
        self.runtime.channels.init_progress = Some(progress_rx);

        tokio::spawn(async move {
            let pool = SubAgentPool::new(client, cancellation)
                .with_concurrency(4)
                .with_override_model(Some(current_model));

            let tasks = vec![
                SubAgentTask::new(
                    "architecture",
                    "OUTPUT ONLY RAW FINDINGS. NO commentary, NO 'I will', NO 'Let me', NO summaries.\n\n\
                     List the main modules/crates with one-line descriptions:\n\
                     - module_name: what it does\n\n\
                     Then list key design patterns used (if any).",
                )
                .with_name("architecture")
                .with_working_dir(working_dir.clone()),
                SubAgentTask::new(
                    "conventions",
                    "OUTPUT ONLY RAW FINDINGS. NO commentary, NO 'I will', NO 'Let me'.\n\n\
                     List conventions found:\n\
                     - Error handling: (anyhow/thiserror/custom)\n\
                     - Logging: (tracing/log/println)\n\
                     - Async: (tokio/async-std/none)\n\
                     - Testing: (location, framework)\n\
                     - Naming: (any patterns observed)",
                )
                .with_name("conventions")
                .with_working_dir(working_dir.clone()),
                SubAgentTask::new(
                    "key_files",
                    "OUTPUT ONLY RAW FINDINGS. NO commentary, NO 'I will', NO 'Let me'.\n\n\
                     List 5-10 important files with one-line descriptions:\n\
                     - `path/to/file.rs` - what it does",
                )
                .with_name("key_files")
                .with_working_dir(working_dir.clone()),
                SubAgentTask::new(
                    "build_system",
                    "OUTPUT ONLY RAW FINDINGS. NO commentary, NO 'I will', NO 'Let me'.\n\n\
                     List build commands:\n\
                     ```bash\n\
                     command  # description\n\
                     ```\n\n\
                     List key dependencies (just names, no versions unless critical).",
                )
                .with_name("build")
                .with_working_dir(working_dir.clone()),
            ];

            let results = pool.execute_with_progress(tasks, progress_tx).await;

            let mut architecture = String::new();
            let mut conventions = String::new();
            let mut key_files = String::new();
            let mut build_system = String::new();
            let mut any_success = false;
            let mut errors: Vec<String> = Vec::new();

            for result in results {
                if result.success {
                    any_success = true;
                    match result.task_id.as_str() {
                        "architecture" => architecture = result.output,
                        "conventions" => conventions = result.output,
                        "key_files" => key_files = result.output,
                        "build_system" => build_system = result.output,
                        _ => {}
                    }
                } else if let Some(err) = result.error {
                    errors.push(format!("{}: {}", result.task_id, err));
                }
            }

            let error_msg = if any_success {
                None
            } else if errors.is_empty() {
                Some("All exploration agents failed (no details)".to_string())
            } else {
                Some(format!("Exploration failed:\n{}", errors.join("\n")))
            };

            let exploration_result = InitExplorationResult {
                architecture,
                conventions,
                key_files,
                build_system,
                success: any_success,
                error: error_msg,
            };

            let _ = result_tx.send(exploration_result);
        });
    }

    /// Detect programming languages used in the project.
    pub fn detect_project_languages(&self) -> Vec<String> {
        let mut languages = Vec::new();

        if self.runtime.working_dir.join("Cargo.toml").exists() {
            languages.push("Rust".to_string());
        }
        if self.runtime.working_dir.join("package.json").exists() {
            languages.push("JavaScript/TypeScript".to_string());
        }
        if self.runtime.working_dir.join("pyproject.toml").exists()
            || self.runtime.working_dir.join("setup.py").exists()
        {
            languages.push("Python".to_string());
        }
        if self.runtime.working_dir.join("go.mod").exists() {
            languages.push("Go".to_string());
        }
        if self.runtime.working_dir.join("pom.xml").exists()
            || self.runtime.working_dir.join("build.gradle").exists()
        {
            languages.push("Java".to_string());
        }
        if self.runtime.working_dir.join("Gemfile").exists() {
            languages.push("Ruby".to_string());
        }
        if self.runtime.working_dir.join("mix.exs").exists() {
            languages.push("Elixir".to_string());
        }

        if languages.is_empty() {
            languages.push("Unknown".to_string());
        }

        languages
    }

    /// Detect basic project structure.
    fn detect_project_structure(&self) -> Vec<(String, String)> {
        let mut structure = Vec::new();

        let common_dirs = [
            ("src", "Source code"),
            ("lib", "Library code"),
            ("tests", "Test files"),
            ("test", "Test files"),
            ("docs", "Documentation"),
            ("examples", "Example code"),
            ("scripts", "Build/utility scripts"),
            ("config", "Configuration files"),
            ("migrations", "Database migrations"),
        ];

        for (dir, desc) in common_dirs {
            if self.runtime.working_dir.join(dir).is_dir() {
                structure.push((dir.to_string(), desc.to_string()));
            }
        }

        structure
    }
}
