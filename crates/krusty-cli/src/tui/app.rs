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
use std::{io, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::RwLock;

use crate::agent::{AgentCancellation, AgentConfig, AgentEventBus, AgentState, UserHookManager};
use crate::ai::client::AiClient;
use crate::ai::models::SharedModelRegistry;
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, AiToolCall, Content};
use crate::extensions::WasmHost;
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
    BlockManager, BlockUiStates, ChatState, PopupState, ScrollSystem, ToolResultCache,
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

    // Extensions (not yet wired into tool dispatch)
    #[allow(dead_code)]
    pub wasm_host: Option<Arc<WasmHost>>,

    // Skills/MCP
    pub skills_manager: Arc<RwLock<SkillsManager>>,
    pub mcp_manager: Arc<krusty_core::mcp::McpManager>,
    pub mcp_status_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::utils::McpStatusUpdate>,
    pub oauth_status_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::utils::OAuthStatusUpdate>,
}

/// UI-only state (view, popups, inputs, rendering, animations)
pub struct AppUi {
    /// Current view (StartMenu, Chat)
    pub view: View,
    /// Current active popup
    pub popup: Popup,
    /// Current work mode (Build, Plan)
    pub work_mode: WorkMode,
    /// Active theme
    pub theme: Arc<crate::tui::themes::Theme>,
    /// Theme name for display/saving
    pub theme_name: String,
    /// Pending view change to apply at end of event loop
    pub pending_view_change: Option<View>,
    /// Plan sidebar component state
    pub plan_sidebar: crate::tui::components::PlanSidebarState,
    /// Plugin window (Kitty graphics) state
    pub plugin_window: crate::tui::components::PluginWindowState,
    /// Decision prompt component state
    pub decision_prompt: crate::tui::components::DecisionPrompt,
    /// Multi-line text input
    pub input: MultiLineInput,
    /// Autocomplete popup
    pub autocomplete: AutocompletePopup,
    /// File search popup
    pub file_search: crate::tui::input::FileSearchPopup,
    /// Scroll and layout system
    pub scroll_system: ScrollSystem,
    /// All popup states
    pub popups: PopupState,
    /// Menu animation state
    pub menu_animator: MenuAnimator,
    /// ID-based UI state for blocks
    pub block_ui: BlockUiStates,
    /// Markdown rendering cache
    pub markdown_cache: MarkdownCache,
    /// Toast notification queue
    pub toasts: crate::tui::components::ToastQueue,
    /// Dirty-tracking flag for render optimization
    pub needs_redraw: bool,
}

impl AppUi {
    pub fn new(
        theme: Arc<crate::tui::themes::Theme>,
        theme_name: String,
        working_dir: PathBuf,
    ) -> Self {
        Self {
            view: View::StartMenu,
            popup: Popup::None,
            work_mode: WorkMode::Build,
            theme,
            theme_name,
            pending_view_change: None,
            plan_sidebar: crate::tui::components::PlanSidebarState::default(),
            plugin_window: crate::tui::components::PluginWindowState::default(),
            decision_prompt: crate::tui::components::DecisionPrompt::default(),
            input: MultiLineInput::new(5),
            autocomplete: AutocompletePopup::new(),
            file_search: crate::tui::input::FileSearchPopup::new(working_dir),
            scroll_system: ScrollSystem::new(),
            popups: PopupState::new(),
            menu_animator: MenuAnimator::new(),
            block_ui: BlockUiStates::new(),
            markdown_cache: MarkdownCache::new(),
            toasts: crate::tui::components::ToastQueue::new(),
            needs_redraw: true,
        }
    }
}

/// Runtime state (AI, streaming, processes, sessions, plans, agents)
pub struct AppRuntime {
    /// Active plan file
    pub active_plan: Option<PlanFile>,
    /// Chat state (messages, conversation, streaming flags)
    pub chat: ChatState,
    /// Current model identifier
    pub current_model: String,
    /// Token usage tracking
    pub context_tokens_used: usize,
    /// Flag to trigger auto-pinch after response completes
    pub pending_auto_pinch: bool,
    /// Auto-pinch in progress (bypasses popup when AI is busy)
    pub auto_pinch_in_progress: bool,
    /// AI client
    pub ai_client: Option<AiClient>,
    /// API key
    pub api_key: Option<String>,
    /// Active AI provider
    pub active_provider: ProviderId,
    /// Background process registry
    pub process_registry: Arc<ProcessRegistry>,
    /// Running process count (cached for status bar)
    pub running_process_count: usize,
    /// Oldest running process elapsed time
    pub running_process_elapsed: Option<std::time::Duration>,
    /// Working directory
    pub working_dir: PathBuf,
    /// Current session ID
    pub current_session_id: Option<String>,
    /// Session title
    pub session_title: Option<String>,
    /// Title editing state
    pub title_editor: TitleEditor,
    /// Async channel receivers
    pub channels: AsyncChannels,
    /// /init exploration ID
    pub init_explore_id: Option<String>,
    /// Cached languages for /init
    pub cached_init_languages: Option<Vec<String>>,
    /// Queued tool calls waiting for explore
    pub queued_tools: Vec<AiToolCall>,
    /// Pending tool results to combine
    pub pending_tool_results: Vec<Content>,
    /// Agent event bus
    pub event_bus: AgentEventBus,
    /// Agent state
    pub agent_state: AgentState,
    /// Agent config
    pub agent_config: AgentConfig,
    /// Agent cancellation token
    pub cancellation: AgentCancellation,
    /// Extended thinking mode enabled
    pub thinking_enabled: bool,
    /// Streaming state machine
    pub streaming: StreamingManager,
    /// Clipboard images pending resolution
    pub pending_clipboard_images: std::collections::HashMap<String, (usize, usize, Vec<u8>)>,
    /// Block manager (owns all block types)
    pub blocks: BlockManager,
    /// Tool result cache for rendering
    pub tool_results: ToolResultCache,
    /// Attached files mapping
    pub attached_files: std::collections::HashMap<String, PathBuf>,
    /// Exploration budget tracking
    pub exploration_budget_count: usize,
    /// Just updated flag
    pub just_updated: bool,
    /// Update status
    pub update_status: Option<krusty_core::updater::UpdateStatus>,
    /// Should quit flag
    pub should_quit: bool,
}

impl AppRuntime {
    pub fn new(
        current_model: String,
        active_provider: ProviderId,
        working_dir: PathBuf,
        process_registry: Arc<ProcessRegistry>,
    ) -> Self {
        Self {
            active_plan: None,
            chat: ChatState::new(),
            current_model,
            context_tokens_used: 0,
            pending_auto_pinch: false,
            auto_pinch_in_progress: false,
            ai_client: None,
            api_key: None,
            active_provider,
            process_registry,
            running_process_count: 0,
            running_process_elapsed: None,
            working_dir,
            current_session_id: None,
            session_title: None,
            title_editor: TitleEditor::new(),
            channels: AsyncChannels::new(),
            init_explore_id: None,
            cached_init_languages: None,
            queued_tools: Vec::new(),
            pending_tool_results: Vec::new(),
            event_bus: AgentEventBus::new(),
            agent_state: AgentState::new(),
            agent_config: AgentConfig::default(),
            cancellation: AgentCancellation::new(),
            thinking_enabled: false,
            streaming: StreamingManager::new(),
            pending_clipboard_images: std::collections::HashMap::new(),
            blocks: BlockManager::new(),
            tool_results: ToolResultCache::new(),
            attached_files: std::collections::HashMap::new(),
            exploration_budget_count: 0,
            just_updated: false,
            update_status: None,
            should_quit: false,
        }
    }
}

/// Application state
pub struct App {
    /// UI-only state
    pub ui: AppUi,
    /// Runtime state
    pub runtime: AppRuntime,
    /// Application services
    pub services: AppServices,
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

        let ui = AppUi::new(theme, theme_name, working_dir.clone());
        let runtime = AppRuntime::new(
            current_model,
            active_provider,
            working_dir,
            process_registry,
        );

        // Manually set channels that were initialized in init_services
        let runtime = AppRuntime {
            channels,
            ..runtime
        };

        Self {
            ui,
            runtime,
            services,
        }
    }

    /// Get max context window size for current model
    pub fn max_context_tokens(&self) -> usize {
        // First check dynamic ModelRegistry (OpenRouter models live here)
        // Use try_get_model() to avoid blocking during rendering
        if let Some(metadata) = self
            .services
            .model_registry
            .try_get_model(&self.runtime.current_model)
        {
            return metadata.context_window;
        }

        // Fall back to static provider config (Anthropic, Z.ai, etc.)
        if let Some(provider) = crate::ai::providers::get_provider(self.runtime.active_provider) {
            if let Some(model) = provider
                .models
                .iter()
                .find(|m| m.id == self.runtime.current_model)
            {
                return model.context_window;
            }
        }

        // Ultimate fallback to default constant
        crate::constants::ai::CONTEXT_WINDOW_TOKENS
    }

    /// Clear the active plan and sync UI state
    pub fn clear_plan(&mut self) {
        self.runtime.active_plan = None;
        self.ui.work_mode = WorkMode::Build;
        self.ui.plan_sidebar.reset();
    }

    /// Set the active plan without changing work mode
    ///
    /// Callers are responsible for setting the appropriate WorkMode:
    /// - New plan from AI: set WorkMode::Plan
    /// - Session resume: choose based on plan progress
    pub fn set_plan(&mut self, plan: PlanFile) {
        self.runtime.active_plan = Some(plan);
    }

    /// Context usage threshold for auto-pinch (80%)
    const AUTO_PINCH_THRESHOLD: f32 = 0.80;

    /// Check if context usage warrants auto-pinch and set the pending flag
    ///
    /// Called after AI response completes. If context is at threshold,
    /// sets `pending_auto_pinch` which triggers the pinch popup when idle.
    pub fn check_auto_pinch(&mut self) {
        // Don't trigger if already pending or no session
        if self.runtime.pending_auto_pinch || self.runtime.current_session_id.is_none() {
            return;
        }

        let max_tokens = self.max_context_tokens();
        if max_tokens == 0 {
            return;
        }

        let usage_ratio = self.runtime.context_tokens_used as f32 / max_tokens as f32;

        if usage_ratio >= Self::AUTO_PINCH_THRESHOLD {
            tracing::info!(
                "Context at {:.0}% ({}/{}) - will trigger auto-pinch after idle",
                usage_ratio * 100.0,
                self.runtime.context_tokens_used,
                max_tokens
            );
            self.runtime.pending_auto_pinch = true;
        }
    }

    /// Trigger auto-pinch if pending and conditions are right
    ///
    /// Called from main loop. When AI is busy (autonomous work), bypasses the popup
    /// entirely and runs pinch in the background. When idle, shows the popup for
    /// manual interaction.
    pub fn trigger_pending_auto_pinch(&mut self) {
        if !self.runtime.pending_auto_pinch {
            return;
        }

        // Don't trigger if still busy with streaming or tools
        if self.runtime.chat.is_streaming || self.runtime.chat.is_executing_tools {
            return;
        }

        // Don't trigger if already in a popup or auto-pinch is running
        if self.ui.popup != crate::tui::app::Popup::None || self.runtime.auto_pinch_in_progress {
            return;
        }

        // Don't trigger if no session
        if self.runtime.current_session_id.is_none() {
            self.runtime.pending_auto_pinch = false;
            return;
        }

        self.runtime.pending_auto_pinch = false;

        // Calculate usage percent
        let max_tokens = self.max_context_tokens();
        let usage_percent = if max_tokens > 0 {
            ((self.runtime.context_tokens_used as f64 / max_tokens as f64) * 100.0) as u8
        } else {
            0
        };

        // Show system message explaining why
        self.runtime.chat.messages.push((
            "system".to_string(),
            format!(
                "Context is at {}% capacity ({} / {} tokens). Starting pinch to continue conversation with fresh context...",
                usage_percent,
                self.runtime.context_tokens_used,
                max_tokens
            ),
        ));

        // Check if conversation has pending AI work (multi-turn tool loop).
        // If the last message is a tool result or assistant message with tool calls,
        // the AI was mid-flow — bypass popup and auto-pinch silently.
        let was_autonomous = self.runtime.chat.conversation.last().is_some_and(|msg| {
            msg.role == crate::ai::types::Role::User
                && msg
                    .content
                    .iter()
                    .any(|c| matches!(c, crate::ai::types::Content::ToolResult { .. }))
        });

        if was_autonomous {
            // AI was working autonomously — bypass popup
            tracing::info!("Auto-pinch: AI was autonomous, bypassing popup");
            self.start_auto_pinch();
        } else {
            // User is interactive — show popup as before
            let top_files = self.get_top_files_preview(5);
            self.ui.popups.pinch.start(usage_percent, top_files);
            self.ui.popup = crate::tui::app::Popup::Pinch;
        }
    }

    /// Show a toast notification
    pub fn show_toast(&mut self, toast: crate::tui::components::Toast) {
        self.ui.toasts.push(toast);
    }

    /// Get plan info for toolbar display
    pub fn get_plan_info(&self) -> Option<crate::tui::components::PlanInfo<'_>> {
        self.runtime.active_plan.as_ref().map(|plan| {
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
        self.runtime.chat.start_streaming();
    }

    /// Stop streaming from AI - clears is_streaming flag and related caches
    pub fn stop_streaming(&mut self) {
        self.runtime.chat.stop_streaming();
    }

    /// Start tool execution - sets is_executing_tools flag
    pub fn start_tool_execution(&mut self) {
        self.runtime.chat.start_tool_execution();
    }

    /// Stop tool execution - clears is_executing_tools flag
    pub fn stop_tool_execution(&mut self) {
        self.runtime.chat.stop_tool_execution();
    }

    /// Apply any pending view change (called at end of event loop iteration)
    pub fn apply_pending_view_change(&mut self) {
        if let Some(view) = self.ui.pending_view_change.take() {
            self.ui.view = view;
        }
    }

    /// Check if busy (streaming OR executing tools)
    pub fn is_busy(&self) -> bool {
        self.runtime.chat.is_busy()
    }

    /// Start editing the session title
    pub fn start_title_edit(&mut self) {
        if self.ui.view == View::Chat {
            self.runtime
                .title_editor
                .start(self.runtime.session_title.as_deref());
        }
    }

    /// Cancel title editing and revert
    pub fn cancel_title_edit(&mut self) {
        self.runtime.title_editor.cancel();
    }

    /// Save the edited title
    pub fn save_title_edit(&mut self) {
        if let Some(new_title) = self.runtime.title_editor.finish() {
            self.runtime.session_title = Some(new_title.clone());

            // Save to database
            if let (Some(manager), Some(session_id)) = (
                &self.services.session_manager,
                &self.runtime.current_session_id,
            ) {
                let _ = manager.update_session_title(session_id, &new_title);
            }
        }
    }

    /// Run the application
    pub async fn run(&mut self) -> Result<()> {
        let _ = self.try_load_auth().await;

        // Check if we just applied an update (marker file written by apply_pending_update)
        if let Some(version) = krusty_core::updater::read_update_marker() {
            self.show_toast(crate::tui::components::Toast::success(format!(
                "Updated to v{}",
                version
            )));
            self.runtime.just_updated = true;
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
        self.ui.plugin_window.detect_graphics_support();
        self.ui.plugin_window.update_cell_size();

        let result = self.main_loop(&mut terminal).await;

        // Kill all background processes on shutdown
        self.runtime.process_registry.kill_all().await;

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
                self.ui.needs_redraw = true;
            }
            Event::Mouse(mouse) => {
                self.handle_mouse_event(mouse);
                self.ui.needs_redraw = true;
            }
            Event::Paste(text) => {
                self.handle_paste(text);
                self.ui.needs_redraw = true;
            }
            Event::Resize(_, _) => {
                // Update cell size for Kitty graphics on resize
                self.ui.plugin_window.update_cell_size();
                self.ui.needs_redraw = true;
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
            if let Some(area) = self.ui.scroll_system.layout.input_area {
                self.ui.input.set_width(area.width);
            }

            // Update running process count and elapsed time for status bar (non-blocking)
            if let Some(count) = self.runtime.process_registry.try_running_count() {
                self.runtime.running_process_count = count;
            }
            self.runtime.running_process_elapsed =
                self.runtime.process_registry.try_oldest_running_elapsed();

            // Keep process popup updated while open (non-blocking)
            if self.ui.popup == Popup::ProcessList {
                if let Some(processes) = self.runtime.process_registry.try_list() {
                    self.ui.popups.process.update(processes);
                }
            }

            // Process streaming events (extracted to handlers/stream_events.rs)
            // Trigger redraw when events were processed - ensures buffered text renders
            // even after is_streaming becomes false (fixes GLM/Z.ai streaming freeze)
            if self.process_stream_events() {
                self.ui.needs_redraw = true;
            }

            // Execute tools when ready
            self.check_and_execute_tools();

            // Check for completed tool execution
            if let Some(ref mut rx) = self.runtime.channels.tool_results {
                match rx.try_recv() {
                    Ok(tool_results) => {
                        self.runtime.channels.tool_results = None;
                        self.stop_streaming();
                        self.stop_tool_execution();
                        self.handle_tool_results(tool_results);
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                        // Still waiting for tools to complete
                    }
                    Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                        // Task finished without sending results (error case)
                        self.runtime.channels.tool_results = None;
                        self.stop_streaming();
                        self.stop_tool_execution();
                    }
                }
            }

            // Poll async operations
            self.poll_openrouter_fetch();
            self.poll_title_generation();
            self.poll_summarization();

            // Poll auto-pinch (background pinch without popup)
            if self.runtime.auto_pinch_in_progress {
                self.poll_auto_pinch();
            }

            // Update menu animations (only when on start menu for efficiency)
            if self.ui.view == View::StartMenu {
                // Use inner_area width (terminal width minus borders) so crab stays contained
                let term_size = terminal.size()?;
                let inner_width = term_size.width.saturating_sub(2); // Account for logo border
                self.ui.menu_animator.update(
                    inner_width,
                    term_size.height,
                    Duration::from_millis(16),
                );
            }

            // Poll bash output channel for streaming updates
            self.poll_bash_output();

            // Poll explore progress channel for agent updates
            self.poll_explore_progress();

            // Poll build progress channel for builder updates
            self.poll_build_progress();

            // Poll /init exploration progress and result
            // Clone cached languages to avoid borrow conflict (cleared on completion)
            let languages = self
                .runtime
                .cached_init_languages
                .clone()
                .unwrap_or_default();
            let init_result = poll_init_exploration(
                &mut self.runtime.channels,
                &mut self.runtime.blocks.explore,
                &mut self.runtime.init_explore_id,
                &mut self.runtime.cached_init_languages,
                &self.runtime.working_dir,
                &languages,
            );
            if init_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(init_result);

            // Poll MCP status updates from background tasks
            let mcp_result = poll_mcp_status(&mut self.runtime.channels, &mut self.ui.popups.mcp);
            if mcp_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(mcp_result);

            // Poll OAuth status updates from background tasks
            let oauth_result = poll_oauth_status(
                &mut self.runtime.channels,
                &mut self.ui.popups.auth,
                self.runtime.active_provider,
            );
            if oauth_result.needs_redraw {
                self.ui.needs_redraw = true;
            }
            self.process_poll_actions(oauth_result);

            // Poll update status and show toasts
            self.poll_update_status();

            // Poll terminal panes for output updates and cursor blink
            self.poll_terminal_panes();

            // Poll ProcessRegistry for background process status updates
            let process_result = poll_background_processes(
                &self.runtime.process_registry,
                &mut self.runtime.blocks.bash,
            );
            if process_result.needs_redraw {
                self.ui.needs_redraw = true;
            }

            // Tick toasts (auto-dismiss expired) - mark dirty if any expired
            if self.ui.toasts.tick() {
                self.ui.needs_redraw = true;
            }

            // Tick all animation blocks (before render, not during)
            // Returns true if any block is still animating
            if self.tick_blocks() {
                self.ui.needs_redraw = true;
            }

            // Check if we should trigger auto-pinch (context at threshold)
            // Only triggers when idle (not streaming, not executing tools)
            self.trigger_pending_auto_pinch();

            // Process continuous edge scrolling during selection
            if self.ui.scroll_system.edge_scroll.direction.is_some() {
                self.process_edge_scroll();
                self.ui.needs_redraw = true;
            }

            // Always redraw if streaming is active (receiving deltas)
            if self.runtime.chat.is_streaming {
                self.ui.needs_redraw = true;
            }

            // Only render if something changed
            if self.ui.needs_redraw {
                terminal.draw(|f| self.ui(f))?;
                // Flush any pending Kitty graphics after buffer is rendered
                self.ui.plugin_window.flush_pending_graphics();
                self.ui.needs_redraw = false;
            }

            // 60fps polling - edge scroll needs faster polling for smooth scrolling
            let poll_timeout = if self.ui.scroll_system.edge_scroll.direction.is_some() {
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

            if self.runtime.should_quit {
                // Save session state before exiting
                self.save_session_token_count();
                self.save_block_ui_states();
                break;
            }
        }
        Ok(())
    }
}
