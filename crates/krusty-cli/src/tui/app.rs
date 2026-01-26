//! Main TUI application
//!
//! Core application state and event loop.
//! Handler implementations are in the handlers/ module.

use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, EventStream, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::RwLock;

use crate::agent::{
    dual_mind::DualMind, AgentCancellation, AgentConfig, AgentEventBus, AgentState,
    UserHookManager,
};
use crate::ai::client::AiClient;
use crate::ai::models::SharedModelRegistry;
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, AiToolCall, Content};
use crate::extensions::WasmHost;
use crate::lsp::LspManager;
use crate::plan::{PlanFile, PlanManager};
use crate::process::ProcessRegistry;
use crate::storage::{CredentialStore, Preferences, SessionManager};
use crate::tools::ToolRegistry;
use crate::tui::animation::MenuAnimator;
use crate::tui::input::{AutocompletePopup, MultiLineInput};
use crate::tui::markdown::MarkdownCache;
use crate::tui::polling::{
    poll_background_processes, poll_init_exploration, poll_mcp_status, poll_oauth_status,
};
use crate::tui::state::{
    BlockManager, BlockUiStates, ChatState, PopupState, ScrollSystem, ToolResultCache, UiState,
};
use crate::tui::streaming::StreamingManager;
use crate::tui::utils::{AsyncChannels, TitleEditor};
use krusty_core::skills::SkillsManager;

/// View types
#[derive(Debug, Clone, PartialEq)]
pub enum View {
    StartMenu,
    Chat,
}

/// Popup types
#[derive(Debug, Clone, PartialEq)]
pub enum Popup {
    None,
    Auth,
    ModelSelect,
    ThemeSelect,
    Help,
    SessionList,
    LspBrowser,
    LspInstall,
    McpBrowser,
    ProcessList,
    Pinch,
    FilePreview,
    SkillsBrowser,
    Hooks,
}

/// Work mode - BUILD (coding) or PLAN (planning)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WorkMode {
    Build,
    Plan,
}

impl WorkMode {
    pub fn toggle(&self) -> Self {
        match self {
            WorkMode::Build => WorkMode::Plan,
            WorkMode::Plan => WorkMode::Build,
        }
    }
}

/// Application services and external systems
pub struct AppServices {
    // Plan/session storage
    pub plan_manager: PlanManager,
    pub session_manager: Option<SessionManager>,
    pub preferences: Option<Preferences>,

    // Credentials/models
    pub credential_store: CredentialStore,
    pub model_registry: SharedModelRegistry,

    // Tool system
    pub tool_registry: Arc<ToolRegistry>,
    pub cached_ai_tools: Vec<AiTool>,
    pub user_hook_manager: Arc<RwLock<UserHookManager>>,

    // LSP/Extensions
    pub lsp_manager: Arc<LspManager>,
    pub lsp_skip_list: std::collections::HashSet<String>,
    pub pending_lsp_install: Option<crate::lsp::manager::MissingLspInfo>,
    pub wasm_host: Option<Arc<WasmHost>>,

    // Skills/MCP
    pub skills_manager: Arc<RwLock<SkillsManager>>,
    pub mcp_manager: Arc<krusty_core::mcp::McpManager>,
    pub mcp_status_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::utils::McpStatusUpdate>,
    pub oauth_status_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::utils::OAuthStatusUpdate>,
}

/// Application state
pub struct App {
    /// UI state (view, popup, theme, work_mode)
    pub ui: UiState,
    pub pending_view_change: Option<View>,
    pub active_plan: Option<PlanFile>,
    pub services: AppServices,

    pub plan_sidebar: crate::tui::components::PlanSidebarState,
    pub plugin_window: crate::tui::components::PluginWindowState,
    pub decision_prompt: crate::tui::components::DecisionPrompt,
    pub should_quit: bool,
    pub input: MultiLineInput,
    pub autocomplete: AutocompletePopup,
    pub file_search: crate::tui::input::FileSearchPopup,

    /// Chat state (messages, conversation, streaming flags)
    pub chat: ChatState,

    /// Scroll/layout system (scroll, layout, selection, hover, edge_scroll)
    pub scroll_system: ScrollSystem,

    pub current_model: String,

    // Token tracking
    pub context_tokens_used: usize,
    /// Flag to trigger auto-pinch after current response completes
    pub pending_auto_pinch: bool,

    // AI client
    pub ai_client: Option<AiClient>,
    pub api_key: Option<String>,

    // Dual-mind quality control (Big Claw / Little Claw)
    pub dual_mind: Option<Arc<RwLock<DualMind>>>,

    // Multi-provider support
    pub active_provider: ProviderId,

    // Process Registry for background processes
    pub process_registry: Arc<ProcessRegistry>,
    pub running_process_count: usize,
    pub running_process_elapsed: Option<std::time::Duration>,

    // Working directory
    pub working_dir: PathBuf,

    // Session management
    pub current_session_id: Option<String>,
    pub session_title: Option<String>,

    // Title editing state
    pub title_editor: TitleEditor,

    // Async channel receivers (LSP, tool results, bash output)
    pub channels: AsyncChannels,

    // /init exploration tracking
    pub init_explore_id: Option<String>,

    // Queued tool calls waiting for explore to complete
    pub queued_tools: Vec<AiToolCall>,

    // Pending tool results waiting to be combined with queued tool results
    pub pending_tool_results: Vec<Content>,

    // Popup states (grouped into component)
    pub popups: PopupState,

    // Menu animation
    pub menu_animator: MenuAnimator,

    // Agent system
    pub event_bus: AgentEventBus,
    pub agent_state: AgentState,
    pub agent_config: AgentConfig,
    pub cancellation: AgentCancellation,

    // Extended thinking mode (Tab to toggle)
    pub thinking_enabled: bool,

    // Streaming state machine (replaces flag-based state)
    pub streaming: StreamingManager,

    // Clipboard images pending resolution (id -> RGBA bytes)
    pub pending_clipboard_images: std::collections::HashMap<String, (usize, usize, Vec<u8>)>,

    // Extensions installed mid-session that need LSP registration
    pub pending_extension_paths: Vec<PathBuf>,

    // Block manager - owns all block types and their state
    // NOTE: Being phased out in favor of conversation-based rendering
    pub blocks: BlockManager,

    // NEW: ID-based UI state (replaces state in block structs)
    pub block_ui: BlockUiStates,

    // NEW: Tool result cache for rendering (keyed by tool_use_id)
    pub tool_results: ToolResultCache,

    // Markdown rendering cache
    pub markdown_cache: MarkdownCache,

    // Attached files mapping (display_name -> file_path)
    pub attached_files: std::collections::HashMap<String, PathBuf>,

    // Toast notification queue
    pub toasts: crate::tui::components::ToastQueue,

    // Auto-updater state
    pub update_status: Option<krusty_core::updater::UpdateStatus>,
    /// Path to the krusty repo (for self-update)
    #[allow(dead_code)] // Infrastructure for future auto-update feature
    pub update_repo_path: Option<PathBuf>,

    // Dirty-tracking for render optimization
    // When false, skip rendering to save CPU
    needs_redraw: bool,
}

impl App {
    /// Create new app instance
    pub async fn new() -> Self {
        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

        // Initialize all services via builder
        let (
            services,
            channels,
            process_registry,
            current_model,
            theme,
            theme_name,
            active_provider,
        ) = crate::tui::app_builder::init_services(&working_dir).await;

        Self {
            ui: UiState::new(theme, theme_name),
            pending_view_change: None,
            active_plan: None,
            services,
            plan_sidebar: crate::tui::components::PlanSidebarState::default(),
            plugin_window: crate::tui::components::PluginWindowState::default(),
            decision_prompt: crate::tui::components::DecisionPrompt::default(),
            should_quit: false,
            input: MultiLineInput::new(5),
            autocomplete: AutocompletePopup::new(),
            file_search: crate::tui::input::FileSearchPopup::new(working_dir.clone()),
            chat: ChatState::new(),
            scroll_system: ScrollSystem::new(),
            current_model,
            context_tokens_used: 0,
            pending_auto_pinch: false,
            ai_client: None,
            api_key: None,
            dual_mind: None,
            active_provider,
            process_registry,
            running_process_count: 0,
            running_process_elapsed: None,
            working_dir,
            current_session_id: None,
            session_title: None,
            title_editor: TitleEditor::new(),
            channels,
            init_explore_id: None,
            queued_tools: Vec::new(),
            pending_tool_results: Vec::new(),
            popups: PopupState::new(),
            menu_animator: MenuAnimator::new(),

            // Agent system
            event_bus: AgentEventBus::new(),
            agent_state: AgentState::new(),
            agent_config: AgentConfig::default(),
            cancellation: AgentCancellation::new(),

            // Extended thinking mode (Tab to toggle)
            thinking_enabled: false,

            // Streaming state machine
            streaming: StreamingManager::new(),

            // Clipboard images
            pending_clipboard_images: std::collections::HashMap::new(),

            // Extensions to register
            pending_extension_paths: Vec::new(),

            // Block manager (owns all block types)
            blocks: BlockManager::new(),

            // NEW: ID-based UI state
            block_ui: BlockUiStates::new(),

            // NEW: Tool result cache
            tool_results: ToolResultCache::new(),

            // Markdown rendering cache
            markdown_cache: MarkdownCache::new(),

            // Attached files for preview lookup
            attached_files: std::collections::HashMap::new(),

            // Toast notifications
            toasts: crate::tui::components::ToastQueue::new(),

            // Auto-updater
            update_status: None,
            update_repo_path: krusty_core::updater::detect_repo_path(),

            // Start with dirty to ensure first frame renders
            needs_redraw: true,
        }
    }

    /// Get max context window size for current model
    pub fn max_context_tokens(&self) -> usize {
        // First check dynamic ModelRegistry (OpenRouter models live here)
        // Use try_get_model() to avoid blocking during rendering
        if let Some(metadata) = self
            .services
            .model_registry
            .try_get_model(&self.current_model)
        {
            return metadata.context_window;
        }

        // Fall back to static provider config (Anthropic, Z.ai, etc.)
        if let Some(provider) = crate::ai::providers::get_provider(self.active_provider) {
            if let Some(model) = provider.models.iter().find(|m| m.id == self.current_model) {
                return model.context_window;
            }
        }

        // Ultimate fallback to default constant
        crate::constants::ai::CONTEXT_WINDOW_TOKENS
    }

    /// Context usage threshold for auto-pinch (80%)
    const AUTO_PINCH_THRESHOLD: f32 = 0.80;

    /// Check if context usage warrants auto-pinch and set the pending flag
    ///
    /// Called after AI response completes. If context is at threshold,
    /// sets `pending_auto_pinch` which triggers the pinch popup when idle.
    pub fn check_auto_pinch(&mut self) {
        // Don't trigger if already pending or no session
        if self.pending_auto_pinch || self.current_session_id.is_none() {
            return;
        }

        let max_tokens = self.max_context_tokens();
        if max_tokens == 0 {
            return;
        }

        let usage_ratio = self.context_tokens_used as f32 / max_tokens as f32;

        if usage_ratio >= Self::AUTO_PINCH_THRESHOLD {
            tracing::info!(
                "Context at {:.0}% ({}/{}) - will trigger auto-pinch after idle",
                usage_ratio * 100.0,
                self.context_tokens_used,
                max_tokens
            );
            self.pending_auto_pinch = true;
        }
    }

    /// Trigger auto-pinch if pending and conditions are right
    ///
    /// Called from main loop when not streaming/executing tools.
    /// Opens the pinch popup with an explanatory message.
    pub fn trigger_pending_auto_pinch(&mut self) {
        if !self.pending_auto_pinch {
            return;
        }

        // Don't trigger if still busy
        if self.chat.is_streaming || self.chat.is_executing_tools {
            return;
        }

        // Don't trigger if already in a popup
        if self.ui.popup != crate::tui::app::Popup::None {
            return;
        }

        // Don't trigger if no session
        if self.current_session_id.is_none() {
            self.pending_auto_pinch = false;
            return;
        }

        self.pending_auto_pinch = false;

        // Calculate usage percent
        let max_tokens = self.max_context_tokens();
        let usage_percent = if max_tokens > 0 {
            ((self.context_tokens_used as f64 / max_tokens as f64) * 100.0) as u8
        } else {
            0
        };

        // Show system message explaining why
        self.chat.messages.push((
            "system".to_string(),
            format!(
                "Context is at {}% capacity ({} / {} tokens). Starting pinch to continue conversation with fresh context...",
                usage_percent,
                self.context_tokens_used,
                max_tokens
            ),
        ));

        // Get top files for pinch context (same as manual /pinch command)
        let top_files = self.get_top_files_preview(5);

        // Open pinch popup (same as manual trigger)
        self.popups.pinch.start(usage_percent, top_files);
        self.ui.popup = crate::tui::app::Popup::Pinch;
    }

    /// Show a toast notification
    pub fn show_toast(&mut self, toast: crate::tui::components::Toast) {
        self.toasts.push(toast);
    }

    /// Get plan info for toolbar display
    pub fn get_plan_info(&self) -> Option<crate::tui::components::PlanInfo<'_>> {
        self.active_plan.as_ref().map(|plan| {
            let (completed, total) = plan.progress();
            crate::tui::components::PlanInfo {
                title: &plan.title,
                completed,
                total,
            }
        })
    }

    // =========================================================================
    // Processing State Helpers
    // =========================================================================

    /// Start streaming from AI - sets is_streaming flag
    pub fn start_streaming(&mut self) {
        self.chat.start_streaming();
    }

    /// Stop streaming from AI - clears is_streaming flag and related caches
    pub fn stop_streaming(&mut self) {
        self.chat.stop_streaming();
    }

    /// Start tool execution - sets is_executing_tools flag
    pub fn start_tool_execution(&mut self) {
        self.chat.start_tool_execution();
    }

    /// Stop tool execution - clears is_executing_tools flag
    pub fn stop_tool_execution(&mut self) {
        self.chat.stop_tool_execution();
    }

    /// Apply any pending view change (called at end of event loop iteration)
    pub fn apply_pending_view_change(&mut self) {
        if let Some(view) = self.pending_view_change.take() {
            self.ui.view = view;
        }
    }

    /// Check if busy (streaming OR executing tools)
    pub fn is_busy(&self) -> bool {
        self.chat.is_busy()
    }

    /// Start editing the session title
    pub fn start_title_edit(&mut self) {
        if self.ui.view == View::Chat {
            self.title_editor.start(self.session_title.as_deref());
        }
    }

    /// Cancel title editing and revert
    pub fn cancel_title_edit(&mut self) {
        self.title_editor.cancel();
    }

    /// Save the edited title
    pub fn save_title_edit(&mut self) {
        if let Some(new_title) = self.title_editor.finish() {
            self.session_title = Some(new_title.clone());

            // Save to database
            if let (Some(manager), Some(session_id)) =
                (&self.services.session_manager, &self.current_session_id)
            {
                let _ = manager.update_session_title(session_id, &new_title);
            }
        }
    }

    /// Initialize language servers from loaded WASM extensions
    pub async fn initialize_extension_servers(&self) -> Result<()> {
        crate::tui::extensions::initialize_extension_servers(
            self.services.wasm_host.as_ref(),
            &self.services.lsp_manager,
            &self.working_dir,
        )
        .await
    }

    /// Register language servers for a single extension
    /// Used after installing extensions mid-session
    pub async fn register_extension_servers(&self, extension_path: &Path) -> Result<()> {
        crate::tui::extensions::register_extension_servers(
            self.services.wasm_host.as_ref(),
            &self.services.lsp_manager,
            &self.working_dir,
            extension_path,
        )
        .await
    }

    /// Run the application
    pub async fn run(&mut self) -> Result<()> {
        let _ = self.try_load_auth().await;

        // Check if we just applied an update (set by main.rs)
        if let Ok(version) = std::env::var("KRUSTY_JUST_UPDATED") {
            std::env::remove_var("KRUSTY_JUST_UPDATED");
            self.show_toast(crate::tui::components::Toast::success(format!(
                "Updated to v{}",
                version
            )));
        }

        // Check for pending update from previous session (cleans up stale files)
        self.check_pending_update();

        // Check for updates in background
        self.start_update_check();

        // Start background refresh of OpenRouter models if configured and cache is stale
        if self
            .services
            .credential_store
            .get(&crate::ai::providers::ProviderId::OpenRouter)
            .is_some()
        {
            let should_refresh = self
                .services
                .preferences
                .as_ref()
                .map(|p| p.is_openrouter_cache_stale())
                .unwrap_or(true);

            if should_refresh {
                tracing::info!("Starting background OpenRouter model refresh");
                self.start_openrouter_fetch();
            }
        }

        // Start background refresh of OpenCode Zen models if configured and cache is stale
        if self
            .services
            .credential_store
            .get(&crate::ai::providers::ProviderId::OpenCodeZen)
            .is_some()
        {
            let should_refresh = self
                .services
                .preferences
                .as_ref()
                .map(|p| p.is_opencodezen_cache_stale())
                .unwrap_or(true);

            if should_refresh {
                tracing::info!("Starting background OpenCode Zen model refresh");
                self.start_opencodezen_fetch();
            }
        }

        // Register built-in LSPs (downloads if needed)
        let downloader = crate::lsp::LspDownloader::new();
        self.services
            .lsp_manager
            .register_all_builtins(&downloader)
            .await;

        // Then extension LSPs (can override built-ins)
        if let Err(e) = self.initialize_extension_servers().await {
            tracing::warn!("Failed to initialize extension servers: {}", e);
        }

        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste,
            // Enable Kitty keyboard protocol for better key detection
            // - DISAMBIGUATE_ESCAPE_CODES: Better escape sequence handling
            // - REPORT_EVENT_TYPES: Enables key release detection (needed for games)
            // Note: REPORT_ALL_KEYS_AS_ESCAPE_CODES breaks Shift+key for special chars
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            )
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        // Initialize Kitty graphics support for plugin window
        self.plugin_window.detect_graphics_support();
        self.plugin_window.update_cell_size();

        let result = self.main_loop(&mut terminal).await;

        // Kill all background processes on shutdown
        self.process_registry.kill_all().await;

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            PopKeyboardEnhancementFlags,
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        )?;
        terminal.show_cursor()?;
        result
    }

    /// Process a single terminal event
    fn process_event(&mut self, event: Event) {
        match event {
            Event::Key(key) => {
                self.handle_key(key);
                self.needs_redraw = true;
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse);
                self.needs_redraw = true;
            }
            Event::Paste(text) => {
                self.handle_paste(text);
                self.needs_redraw = true;
            }
            Event::Resize(_, _) => {
                // Update cell size for Kitty graphics on resize
                self.plugin_window.update_cell_size();
                self.needs_redraw = true;
            }
            _ => {}
        }
    }

    /// Main event loop
    async fn main_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        // Use async event stream to avoid blocking the runtime
        // This fixes the issue where the app freezes when mouse leaves terminal
        let mut event_stream = EventStream::new();

        loop {
            if let Some(area) = self.scroll_system.layout.input_area {
                self.input.set_width(area.width);
            }

            // Update running process count and elapsed time for status bar (non-blocking)
            if let Some(count) = self.process_registry.try_running_count() {
                self.running_process_count = count;
            }
            self.running_process_elapsed = self.process_registry.try_oldest_running_elapsed();

            // Keep process popup updated while open (non-blocking)
            if self.ui.popup == Popup::ProcessList {
                if let Some(processes) = self.process_registry.try_list() {
                    self.popups.process.update(processes);
                }
            }

            // Process streaming events (extracted to handlers/stream_events.rs)
            // Trigger redraw when events were processed - ensures buffered text renders
            // even after is_streaming becomes false (fixes GLM/Z.ai streaming freeze)
            if self.process_stream_events() {
                self.needs_redraw = true;
            }

            // Execute tools when ready
            self.check_and_execute_tools();

            // Check for completed tool execution
            if let Some(ref mut rx) = self.channels.tool_results {
                match rx.try_recv() {
                    Ok(tool_results) => {
                        self.channels.tool_results = None;
                        self.stop_streaming();
                        self.stop_tool_execution();
                        self.handle_tool_results(tool_results);
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                        // Still waiting for tools to complete
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        // Task finished without sending results (error case)
                        self.channels.tool_results = None;
                        self.stop_streaming();
                        self.stop_tool_execution();
                    }
                }
            }

            // Poll async operations
            self.poll_lsp_install();
            self.poll_builtin_lsp_install();
            self.poll_extension_lsp_install();
            self.poll_extensions_fetch();
            self.poll_openrouter_fetch();
            self.poll_opencodezen_fetch();
            self.poll_title_generation();
            self.poll_summarization();

            // Check for missing LSP notifications from tools (populates pending_lsp_install)
            self.poll_missing_lsp();
            // Then check if we should show the install prompt popup
            self.poll_pending_lsp_install();

            // Register language servers for newly installed extensions
            for path in std::mem::take(&mut self.pending_extension_paths) {
                if let Err(e) = self.register_extension_servers(&path).await {
                    tracing::error!("Failed to register extension servers: {}", e);
                }
            }

            // Update menu animations (only when on start menu for efficiency)
            if self.ui.view == View::StartMenu {
                // Use inner_area width (terminal width minus borders) so crab stays contained
                let term_size = terminal.size()?;
                let inner_width = term_size.width.saturating_sub(2); // Account for logo border
                self.menu_animator
                    .update(inner_width, term_size.height, Duration::from_millis(16));
            }

            // Poll bash output channel for streaming updates
            self.poll_bash_output();

            // Poll explore progress channel for agent updates
            self.poll_explore_progress();

            // Poll build progress channel for builder updates
            self.poll_build_progress();

            // Poll dual-mind dialogue for Big Claw / Little Claw updates
            let dual_mind_result = self.poll_dual_mind();
            if dual_mind_result.needs_redraw {
                self.needs_redraw = true;
            }
            self.process_poll_actions(dual_mind_result);

            // Poll /init exploration progress and result
            // Only detect languages if we have a pending exploration result
            let languages = if self.channels.init_exploration.is_some() {
                self.detect_project_languages()
            } else {
                Vec::new()
            };
            let init_result = poll_init_exploration(
                &mut self.channels,
                &mut self.blocks.explore,
                &mut self.init_explore_id,
                &self.working_dir,
                &languages,
            );
            if init_result.needs_redraw {
                self.needs_redraw = true;
            }
            self.process_poll_actions(init_result);

            // Poll MCP status updates from background tasks
            let mcp_result = poll_mcp_status(&mut self.channels, &mut self.popups.mcp);
            if mcp_result.needs_redraw {
                self.needs_redraw = true;
            }
            self.process_poll_actions(mcp_result);

            // Poll OAuth status updates from background tasks
            let oauth_result = poll_oauth_status(
                &mut self.channels,
                &mut self.popups.auth,
                self.active_provider,
            );
            if oauth_result.needs_redraw {
                self.needs_redraw = true;
            }
            self.process_poll_actions(oauth_result);

            // Poll update status and show toasts
            self.poll_update_status();

            // Poll terminal panes for output updates and cursor blink
            self.poll_terminal_panes();

            // Poll ProcessRegistry for background process status updates
            let process_result =
                poll_background_processes(&self.process_registry, &mut self.blocks.bash);
            if process_result.needs_redraw {
                self.needs_redraw = true;
            }

            // Tick toasts (auto-dismiss expired) - mark dirty if any expired
            if self.toasts.tick() {
                self.needs_redraw = true;
            }

            // Tick all animation blocks (before render, not during)
            // Returns true if any block is still animating
            if self.tick_blocks() {
                self.needs_redraw = true;
            }

            // Check if we should trigger auto-pinch (context at threshold)
            // Only triggers when idle (not streaming, not executing tools)
            self.trigger_pending_auto_pinch();

            // Process continuous edge scrolling during selection
            if self.scroll_system.edge_scroll.direction.is_some() {
                self.process_edge_scroll();
                self.needs_redraw = true;
            }

            // Always redraw if streaming is active (receiving deltas)
            if self.chat.is_streaming {
                self.needs_redraw = true;
            }

            // Only render if something changed
            if self.needs_redraw {
                terminal.draw(|f| self.ui(f))?;
                // Flush any pending Kitty graphics after buffer is rendered
                self.plugin_window.flush_pending_graphics();
                self.needs_redraw = false;
            }

            // 60fps polling - edge scroll needs faster polling for smooth scrolling
            let poll_timeout = if self.scroll_system.edge_scroll.direction.is_some() {
                Duration::from_millis(8) // 125fps for smooth edge scrolling
            } else {
                Duration::from_millis(16) // 60fps normal
            };

            // Async event handling - doesn't block the runtime when no events
            // This allows async tasks to progress even when mouse is outside terminal
            tokio::select! {
                biased; // Prefer events over timeout when both are ready

                maybe_event = event_stream.next() => {
                    if let Some(Ok(event)) = maybe_event {
                        self.process_event(event);

                        // Drain all pending events for snappy scrollbar dragging
                        // This prevents event queue buildup during rapid mouse movements
                        while let Ok(Some(Ok(event))) = tokio::time::timeout(
                            Duration::ZERO,
                            event_stream.next()
                        ).await {
                            self.process_event(event);
                        }
                    }
                }
                _ = tokio::time::sleep(poll_timeout) => {
                    // Timeout - continue loop for regular updates (animations, polling, etc.)
                }
            }

            // Apply any deferred view changes (after popup handling)
            self.apply_pending_view_change();

            if self.should_quit {
                // Save session state before exiting
                self.save_session_token_count();
                self.save_block_ui_states();
                break;
            }
        }
        Ok(())
    }
}
