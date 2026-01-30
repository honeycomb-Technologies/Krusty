//! Slash command handler
//!
//! Handles /command parsing and execution.

use crate::tui::app::{App, Popup, View};

impl App {
    /// Handle slash commands
    pub fn handle_slash_command(&mut self, cmd: &str) {
        let parts: Vec<&str> = cmd.split_whitespace().collect();
        let command = parts.first().map(|s| s.to_lowercase()).unwrap_or_default();

        match command.as_str() {
            "/home" => {
                self.current_session_id = None;
                self.chat.messages.clear();
                self.chat.streaming_assistant_idx = None;
                self.chat.conversation.clear();
                self.clear_plan();
                self.ui.view = View::StartMenu;
            }
            "/load" => {
                // Set current directory for the popup title
                let current_dir = self.working_dir.to_string_lossy().into_owned();
                self.popups.session.set_current_directory(&current_dir);

                // Get sessions for current directory only
                let sessions: Vec<_> = self
                    .list_sessions_for_directory(&current_dir)
                    .into_iter()
                    .map(|s| crate::tui::popups::session_list::SessionInfo {
                        id: s.id,
                        title: s.title,
                        updated_at: s.updated_at.format("%Y-%m-%d %H:%M").to_string(),
                    })
                    .collect();

                self.popups.session.set_sessions(sessions);
                self.ui.popup = Popup::SessionList;
            }
            "/model" => {
                // Populate model list from registry (non-blocking)
                let configured = self.configured_providers();

                // Get organized models from registry without blocking
                if let Some((recent_models, models_by_provider)) = self
                    .services
                    .model_registry
                    .try_get_organized_models(&configured)
                {
                    // Convert HashMap to Vec sorted by provider display order
                    let models_vec: Vec<_> = crate::ai::providers::ProviderId::all()
                        .iter()
                        .filter_map(|id| {
                            models_by_provider
                                .get(id)
                                .map(|models| (*id, models.clone()))
                        })
                        .collect();

                    self.popups.model.set_models(recent_models, models_vec);
                }

                self.ui.popup = Popup::ModelSelect;

                // If OpenRouter is configured but has no models, trigger fetch
                if configured.contains(&crate::ai::providers::ProviderId::OpenRouter) {
                    if let Some(false) = self
                        .services
                        .model_registry
                        .try_has_models(crate::ai::providers::ProviderId::OpenRouter)
                    {
                        self.start_openrouter_fetch();
                    }
                }

                // If OpenCode Zen is configured but has no models, trigger fetch
                if configured.contains(&crate::ai::providers::ProviderId::OpenCodeZen) {
                    if let Some(false) = self
                        .services
                        .model_registry
                        .try_has_models(crate::ai::providers::ProviderId::OpenCodeZen)
                    {
                        self.start_opencodezen_fetch();
                    }
                }
            }
            "/auth" => {
                self.popups.auth.reset();
                // Set configured providers to show checkmarks
                let configured = self.configured_providers();
                self.popups.auth.set_configured_providers(configured);
                self.ui.popup = Popup::Auth;
            }
            "/lsp" => {
                self.ui.popup = Popup::LspBrowser;
                if self.popups.lsp.needs_fetch() {
                    self.start_extensions_fetch();
                }
            }
            "/ps" | "/processes" => {
                self.refresh_process_popup();
                self.ui.popup = Popup::ProcessList;
            }
            "/theme" => {
                self.popups.theme.open(&self.ui.theme_name);
                self.ui.popup = Popup::ThemeSelect;
            }
            "/clear" => {
                self.chat.messages.clear();
                self.chat.streaming_assistant_idx = None;
                self.blocks = crate::tui::state::BlockManager::new();
            }
            "/cmd" => self.ui.popup = Popup::Help,
            "/init" => {
                self.handle_init_command();
            }
            "/pinch" => {
                self.handle_pinch_command();
            }
            "/terminal" | "/term" | "/shell" => {
                self.handle_terminal_command(parts.get(1).copied());
            }
            "/plan" => {
                self.handle_plan_command(parts.get(1).copied());
            }
            "/skills" => {
                self.open_skills_browser();
            }
            "/mcp" => {
                self.open_mcp_browser();
            }
            "/hooks" => {
                self.open_hooks_popup();
            }
            "/update" => {
                self.start_update_check();
            }
            _ => {
                self.chat
                    .messages
                    .push(("system".to_string(), format!("Unknown command: {}", cmd)));
            }
        }
    }

    /// Handle /init command - intelligently analyze codebase and generate KRAB.md
    fn handle_init_command(&mut self) {
        use crate::tui::app::View;

        // Check if authenticated (need AI for exploration)
        if !self.is_authenticated() {
            // Fallback: generate basic template without AI
            self.generate_basic_krab_template();
            return;
        }

        // Check if already exploring
        if self.channels.init_exploration.is_some() {
            self.chat.messages.push((
                "system".to_string(),
                "Exploration already in progress...".to_string(),
            ));
            return;
        }

        // If on start menu, switch to chat view
        if self.ui.view == View::StartMenu {
            self.ui.view = View::Chat;
        }

        // Create session if none exists
        if self.current_session_id.is_none() {
            self.create_session("/init - Codebase Analysis");
        }

        // Add /init as user message (like a natural conversation)
        self.chat
            .messages
            .push(("user".to_string(), "/init".to_string()));

        // Generate unique ID for this exploration
        let explore_id = format!("init-{}", uuid::Uuid::new_v4());

        // Create ExploreBlock to show the consortium of crabs
        let explore_block = crate::tui::blocks::ExploreBlock::with_tool_id(
            "Analyzing codebase for KRAB.md...".to_string(),
            explore_id.clone(),
        );
        self.blocks.explore.push(explore_block);

        // Add to message timeline so it renders in chat
        self.chat
            .messages
            .push(("explore".to_string(), explore_id.clone()));

        // Store the explore ID for completion
        self.init_explore_id = Some(explore_id);

        // Start async exploration
        self.start_init_exploration();
    }

    /// Generate a basic KRAB.md template without AI (fallback)
    fn generate_basic_krab_template(&mut self) {
        let krab_path = self.working_dir.join("KRAB.md");
        let is_regenerate = krab_path.exists();

        let project_name = self.working_dir.file_name().map_or_else(
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
                self.chat.messages.push((
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
                self.chat.messages.push((
                    "system".to_string(),
                    format!("Failed to write KRAB.md: {}", e),
                ));
            }
        }
    }

    /// Start async codebase exploration for /init
    ///
    /// Flow:
    /// 1. Create indexing progress channel and start background indexing
    /// 2. Poll indexing progress and show in UI
    /// 3. When indexing completes, start AI exploration agents
    /// 4. Poll exploration progress and show in UI
    fn start_init_exploration(&mut self) {
        use crate::agent::subagent::{SubAgentPool, SubAgentTask};
        use crate::paths;
        use crate::tui::utils::InitExplorationResult;
        use krusty_core::index::Indexer;
        use krusty_core::storage::Database;
        use std::sync::Arc;

        let client = match self.create_ai_client() {
            Some(c) => Arc::new(c),
            None => {
                self.chat.messages.push((
                    "system".to_string(),
                    "Failed to create AI client for exploration".to_string(),
                ));
                return;
            }
        };

        let working_dir = self.working_dir.clone();
        let cancellation = self.cancellation.clone();
        let current_model = self.current_model.clone();

        // Cache languages once at /init start (used during polling)
        self.cached_init_languages = Some(self.detect_project_languages());

        // Create indexing progress channel
        let (indexing_tx, indexing_rx) = tokio::sync::mpsc::unbounded_channel();
        self.channels.indexing_progress = Some(indexing_rx);

        // Create indexing completion channel (exploration waits on this)
        let (indexing_done_tx, indexing_done_rx) = tokio::sync::oneshot::channel();
        // Note: we don't store indexing_done_rx in channels - it's passed to the exploration task

        // Create exploration result channel
        let (result_tx, result_rx) = tokio::sync::oneshot::channel();
        self.channels.init_exploration = Some(result_rx);

        // Create exploration progress channel
        let (progress_tx, progress_rx) = tokio::sync::mpsc::unbounded_channel();
        self.channels.init_progress = Some(progress_rx);

        // Get database path for spawned thread
        let db_path = paths::config_dir().join("krusty.db");
        let indexing_working_dir = working_dir.clone();

        // Spawn indexing in blocking task (opens its own DB connection)
        // Attempts embeddings first; falls back to sync indexing without embeddings
        tokio::task::spawn_blocking(move || {
            let result = (|| -> Result<(), String> {
                let db = Database::new(&db_path).map_err(|e| e.to_string())?;

                // Try with embeddings first (uses async index_codebase via block_on)
                match Indexer::new().map_err(|e| e.to_string())?.with_embeddings() {
                    Ok(mut indexer) => {
                        tracing::info!("Indexing with embeddings enabled");
                        let handle = tokio::runtime::Handle::current();
                        handle
                            .block_on(indexer.index_codebase(
                                db.conn(),
                                &indexing_working_dir,
                                Some(indexing_tx),
                            ))
                            .map_err(|e| e.to_string())?;
                    }
                    Err(e) => {
                        tracing::info!("Embeddings unavailable ({e}), indexing without");
                        let mut indexer = Indexer::new().map_err(|e| e.to_string())?;
                        indexer
                            .index_codebase_sync(
                                db.conn(),
                                &indexing_working_dir,
                                Some(indexing_tx),
                            )
                            .map_err(|e| e.to_string())?;
                    }
                }
                Ok(())
            })();
            let _ = indexing_done_tx.send(result);
        });

        // Spawn async task that waits for indexing then runs AI exploration
        tokio::spawn(async move {
            // Wait for indexing to complete before starting AI exploration
            match indexing_done_rx.await {
                Ok(Ok(())) => tracing::info!("Codebase indexing complete, starting AI exploration"),
                Ok(Err(e)) => tracing::warn!("Indexing failed: {}, proceeding with exploration", e),
                Err(_) => tracing::warn!("Indexing task cancelled, proceeding with exploration"),
            }

            let pool = SubAgentPool::new(client, cancellation)
                .with_concurrency(4)
                .with_override_model(Some(current_model));

            // Create exploration tasks with strict output-only prompts
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

            // Execute with progress
            let results = pool.execute_with_progress(tasks, progress_tx).await;

            // Aggregate results
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

    /// Detect programming languages used in the project
    pub fn detect_project_languages(&self) -> Vec<String> {
        let mut languages = Vec::new();

        // Check for common project indicators
        if self.working_dir.join("Cargo.toml").exists() {
            languages.push("Rust".to_string());
        }
        if self.working_dir.join("package.json").exists() {
            languages.push("JavaScript/TypeScript".to_string());
        }
        if self.working_dir.join("pyproject.toml").exists()
            || self.working_dir.join("setup.py").exists()
        {
            languages.push("Python".to_string());
        }
        if self.working_dir.join("go.mod").exists() {
            languages.push("Go".to_string());
        }
        if self.working_dir.join("pom.xml").exists()
            || self.working_dir.join("build.gradle").exists()
        {
            languages.push("Java".to_string());
        }
        if self.working_dir.join("Gemfile").exists() {
            languages.push("Ruby".to_string());
        }
        if self.working_dir.join("mix.exs").exists() {
            languages.push("Elixir".to_string());
        }

        if languages.is_empty() {
            languages.push("Unknown".to_string());
        }

        languages
    }

    /// Detect basic project structure
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
            if self.working_dir.join(dir).is_dir() {
                structure.push((dir.to_string(), desc.to_string()));
            }
        }

        structure
    }

    /// Handle /pinch command - open pinch popup
    fn handle_pinch_command(&mut self) {
        if self.chat.messages.is_empty() {
            self.chat.messages.push((
                "system".to_string(),
                "No conversation to summarize. Start a chat first.".to_string(),
            ));
            return;
        }

        // Calculate context usage percentage
        let max_tokens = self.max_context_tokens();
        let usage_percent = if max_tokens > 0 {
            ((self.context_tokens_used as f64 / max_tokens as f64) * 100.0) as u8
        } else {
            0
        };

        // Get top files by activity (if we have a session)
        let top_files = self.get_top_files_preview(5);

        // Start the pinch popup
        self.popups.pinch.start(usage_percent, top_files);
        self.ui.popup = Popup::Pinch;
    }

    /// Get top N files by activity for preview
    pub(crate) fn get_top_files_preview(&self, n: usize) -> Vec<(String, f64)> {
        // Get file activity from database if we have a session
        if let (Some(sm), Some(session_id)) =
            (&self.services.session_manager, &self.current_session_id)
        {
            use crate::storage::FileActivityTracker;
            let db = sm.db();
            let tracker = FileActivityTracker::new(db, session_id.clone());
            return tracker.get_top_files_preview(n);
        }
        Vec::new()
    }

    /// Handle /terminal command - spawn an interactive PTY terminal
    fn handle_terminal_command(&mut self, shell: Option<&str>) {
        let shell_cmd = shell.unwrap_or("bash");

        match crate::tui::blocks::TerminalPane::spawn(shell_cmd, 24, 80) {
            Ok(mut pane) => {
                // Generate process ID and register with process registry
                let process_id = format!("terminal-{}", uuid::Uuid::new_v4());
                let process_id_clone = process_id.clone(); // Keep for message timeline
                let pid = pane.get_child_pid();
                pane.set_process_id(process_id.clone());

                // Register as Krusty process (async, spawn task)
                let registry = self.process_registry.clone();
                let working_dir = self.working_dir.clone();
                let cmd = shell_cmd.to_string();
                tokio::spawn(async move {
                    registry
                        .register_external(
                            process_id,
                            format!("terminal: {}", cmd),
                            Some("Interactive PTY terminal".to_string()),
                            pid,
                            working_dir,
                        )
                        .await;
                });

                self.blocks.terminal.push(pane);

                // Add to message timeline (store process_id for reliable lookup)
                self.chat
                    .messages
                    .push(("terminal".to_string(), process_id_clone));

                // Auto-scroll to show the new terminal
                self.scroll_system.scroll.request_scroll_to_bottom();
            }
            Err(e) => {
                self.chat.messages.push((
                    "system".to_string(),
                    format!("Failed to spawn terminal: {}", e),
                ));
            }
        }
    }

    /// Handle /plan command
    fn handle_plan_command(&mut self, subcommand: Option<&str>) {
        use crate::plan::PlanStatus;

        match subcommand {
            Some("clear") | Some("abandon") => {
                if let Some(ref mut plan) = self.active_plan {
                    // Mark as abandoned and save
                    plan.status = PlanStatus::Abandoned;
                    if let Err(e) = self.services.plan_manager.save_plan(plan) {
                        tracing::warn!("Failed to save abandoned plan: {}", e);
                    }
                    let title = plan.title.clone();
                    let file_path = plan
                        .file_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default();
                    self.clear_plan();
                    let msg = if file_path.is_empty() {
                        format!("Plan '{}' abandoned.", title)
                    } else {
                        format!("Plan '{}' abandoned. Saved at: {}", title, file_path)
                    };
                    self.chat.messages.push(("system".to_string(), msg));
                } else {
                    self.chat
                        .messages
                        .push(("system".to_string(), "No active plan to clear.".to_string()));
                }
            }
            Some("list") | Some("history") => {
                // Show completed plans for this working directory
                let working_dir_str = self.working_dir.to_string_lossy().into_owned();
                match self
                    .services
                    .plan_manager
                    .list_completed_for_dir(&working_dir_str)
                {
                    Ok(plans) if plans.is_empty() => {
                        self.chat.messages.push((
                            "system".to_string(),
                            "No completed plans for this directory.".to_string(),
                        ));
                    }
                    Ok(plans) => {
                        let mut msg = String::from("Completed plans:\n");
                        for plan in plans.iter().take(5) {
                            let date = plan.created_at.format("%Y-%m-%d");
                            msg.push_str(&format!(
                                "  • {} ({}) - {}/{} tasks\n",
                                plan.title, date, plan.progress.0, plan.progress.1,
                            ));
                        }
                        if plans.len() > 5 {
                            msg.push_str(&format!("  ... and {} more", plans.len() - 5));
                        }
                        self.chat.messages.push(("system".to_string(), msg));
                    }
                    Err(e) => {
                        self.chat
                            .messages
                            .push(("system".to_string(), format!("Failed to list plans: {}", e)));
                    }
                }
            }
            Some("show") | None => {
                if let Some(ref plan) = self.active_plan {
                    let (completed, total) = plan.progress();
                    let status_icon = if completed == total { "✓" } else { "◐" };
                    self.chat.messages.push((
                        "system".to_string(),
                        format!(
                            "{} '{}' ({}/{} tasks)\nUse Ctrl+T to toggle sidebar, /plan clear to abandon.",
                            status_icon, plan.title, completed, total
                        ),
                    ));
                    // Show sidebar if not visible
                    if !self.plan_sidebar.visible {
                        self.plan_sidebar.toggle();
                    }
                } else {
                    self.chat.messages.push((
                        "system".to_string(),
                        "No active plan.\n\
                        • Enter PLAN mode (Ctrl+B) and ask the AI to create a plan\n\
                        • Use /plan list to see completed plans"
                            .to_string(),
                    ));
                }
            }
            Some(unknown) => {
                self.chat.messages.push((
                    "system".to_string(),
                    format!(
                        "Unknown: /plan {}. Use: /plan, /plan list, /plan clear",
                        unknown
                    ),
                ));
            }
        }
    }

    /// Open skills browser popup
    fn open_skills_browser(&mut self) {
        // Load skills and populate popup
        let skills = match self.services.skills_manager.try_write() {
            Ok(mut guard) => guard.list_skills(),
            Err(_) => {
                self.chat.messages.push((
                    "system".to_string(),
                    "Skills manager is busy, try again.".to_string(),
                ));
                return;
            }
        };

        self.popups.skills.set_skills(skills);
        self.ui.popup = Popup::SkillsBrowser;
    }

    /// Refresh skills in the browser
    pub fn refresh_skills_browser(&mut self) {
        let skills = match self.services.skills_manager.try_write() {
            Ok(mut guard) => {
                guard.refresh();
                guard.list_skills()
            }
            Err(_) => return,
        };
        self.popups.skills.set_skills(skills);
    }

    /// Open MCP server browser popup
    fn open_mcp_browser(&mut self) {
        // Update the popup with current server state
        self.refresh_mcp_popup();
        self.ui.popup = Popup::McpBrowser;
    }

    /// Refresh MCP servers in the browser popup
    pub fn refresh_mcp_popup(&mut self) {
        let mcp = self.services.mcp_manager.clone();
        let servers = futures::executor::block_on(mcp.list_servers());
        self.popups.mcp.update(servers);
    }

    /// Open hooks configuration popup
    fn open_hooks_popup(&mut self) {
        let hooks: Vec<_> = futures::executor::block_on(async {
            self.services
                .user_hook_manager
                .read()
                .await
                .hooks()
                .to_vec()
        });
        self.popups.hooks.set_hooks(hooks);
        self.ui.popup = Popup::Hooks;
    }
}

/// Generate KRAB.md template content
fn generate_krab_template(
    project_name: &str,
    languages: &[String],
    structure: &[(String, String)],
) -> String {
    let mut content = String::new();

    content.push_str(&format!("# {}\n\n", project_name));
    content.push_str("<!-- KRAB.md - Project context for Krusty AI assistant -->\n");
    content.push_str(
        "<!-- This file is automatically read at the start of every AI conversation -->\n",
    );
    content.push_str(
        "<!-- Edit it to help the AI understand your project's rules and conventions -->\n\n",
    );

    content.push_str("## Overview\n\n");
    content.push_str("<!-- Describe what this project does and its main purpose -->\n\n");
    content.push_str("TODO: Add project description\n\n");

    content.push_str("## Tech Stack\n\n");
    for lang in languages {
        content.push_str(&format!("- {}\n", lang));
    }
    content.push('\n');

    if !structure.is_empty() {
        content.push_str("## Directory Structure\n\n");
        for (dir, desc) in structure {
            content.push_str(&format!("- `{}` - {}\n", dir, desc));
        }
        content.push('\n');
    }

    content.push_str("## Key Files\n\n");
    content.push_str("<!-- List important files the AI should know about -->\n\n");
    content.push_str("- `KRAB.md` - This file (project context)\n");
    content.push('\n');

    content.push_str("## Conventions\n\n");
    content.push_str("<!-- Describe coding style, naming conventions, etc. -->\n\n");
    content.push_str("TODO: Add coding conventions\n\n");

    content.push_str("## Build & Run\n\n");
    content.push_str("<!-- How to build, run, and test the project -->\n\n");
    content.push_str("```bash\n");
    content.push_str("# TODO: Add build commands\n");
    content.push_str("```\n\n");

    content.push_str("## Notes for AI\n\n");
    content.push_str("<!-- Any specific instructions or context for the AI assistant -->\n\n");
    content.push_str("TODO: Add any project-specific notes\n");

    content
}

/// Clean AI output - remove filler phrases and meta-commentary
fn clean_ai_output(text: &str) -> String {
    // Phrases that indicate meta-commentary, not findings
    const FILLER_STARTS: &[&str] = &[
        "Perfect!",
        "Great!",
        "Excellent!",
        "Now I",
        "Let me",
        "I will",
        "I'll",
        "Based on my",
        "After analyzing",
        "Here's what I found",
        "Here is",
        "Summary:",
        "## Summary",
        "### Summary",
        "Analysis:",
        "## Analysis",
    ];

    let mut lines: Vec<&str> = text.lines().collect();

    // Strip entire noise sections (header + content until next header or double blank)
    const NOISE_SECTIONS: &[&str] = &["### Files Examined", "### Sources"];
    let mut in_noise_section = false;
    lines.retain(|line| {
        let trimmed = line.trim();
        if NOISE_SECTIONS.iter().any(|s| trimmed.starts_with(s)) {
            in_noise_section = true;
            return false;
        }
        if in_noise_section {
            // End noise section at next header or empty line
            if trimmed.is_empty() || trimmed.starts_with('#') {
                in_noise_section = false;
            } else {
                return false;
            }
        }
        if trimmed.starts_with("## ") {
            return false;
        }
        !FILLER_STARTS.iter().any(|f| trimmed.starts_with(f))
    });

    // Remove excessive blank lines (more than 2 in a row)
    let mut result = String::new();
    let mut blank_count = 0;
    for line in lines {
        if line.trim().is_empty() {
            blank_count += 1;
            if blank_count <= 2 {
                result.push('\n');
            }
        } else {
            blank_count = 0;
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

/// Generate KRAB.md from AI exploration results
pub fn generate_krab_from_exploration(
    project_name: &str,
    languages: &[String],
    exploration: &crate::tui::utils::InitExplorationResult,
) -> String {
    let mut content = String::new();

    content.push_str(&format!("# {}\n\n", project_name));

    // Tech Stack
    content.push_str("## Tech Stack\n\n");
    for lang in languages {
        content.push_str(&format!("- {}\n", lang));
    }
    content.push('\n');

    // Architecture (from AI, cleaned)
    let arch = clean_ai_output(&exploration.architecture);
    if !arch.is_empty() {
        content.push_str("## Architecture\n\n");
        content.push_str(&arch);
        content.push_str("\n\n");
    }

    // Key Files (from AI, cleaned)
    let files = clean_ai_output(&exploration.key_files);
    if !files.is_empty() {
        content.push_str("## Key Files\n\n");
        content.push_str(&files);
        content.push_str("\n\n");
    }

    // Conventions (from AI, cleaned)
    let conv = clean_ai_output(&exploration.conventions);
    if !conv.is_empty() {
        content.push_str("## Conventions\n\n");
        content.push_str(&conv);
        content.push_str("\n\n");
    }

    // Build & Run (from AI, cleaned)
    let build = clean_ai_output(&exploration.build_system);
    if !build.is_empty() {
        content.push_str("## Build & Run\n\n");
        content.push_str(&build);
        content.push_str("\n\n");
    }

    // Notes for AI
    content.push_str("## Notes for AI\n\n");
    content.push_str("<!-- Add project-specific instructions here -->\n\n");

    content
}
