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
    AgentCancellation, AgentConfig, AgentEventBus, AgentState, UserHookManager, UserPostToolHook,
    UserPreToolHook,
};
use crate::ai::client::AiClient;
use crate::ai::models::{create_model_registry, SharedModelRegistry};
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, AiToolCall, Content, ModelMessage};
use crate::extensions::WasmHost;
use crate::lsp::LspManager;
use crate::paths;
use crate::plan::{PlanFile, PlanManager};
use crate::process::ProcessRegistry;
use crate::storage::{CredentialStore, Database, Preferences, SessionManager};
use crate::tools::{register_all_tools, register_build_tool, register_explore_tool, ToolRegistry};
use crate::tui::animation::MenuAnimator;
use crate::tui::blocks::StreamBlock;
use crate::tui::input::{AutocompletePopup, MultiLineInput};
use crate::tui::markdown::MarkdownCache;
use crate::tui::state::PopupState;
use crate::tui::state::{
    BlockManager, BlockUiStates, EdgeScrollState, HoverState, LayoutState, ScrollState,
    SelectionState, ToolResultCache,
};
use crate::tui::streaming::StreamingManager;
use crate::tui::themes::{Theme, THEME_REGISTRY};
use crate::tui::utils::AppWorktreeDelegate;
use crate::tui::utils::{count_wrapped_lines, AsyncChannels, TitleEditor};
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

/// Application state
pub struct App {
    pub view: View,
    pub pending_view_change: Option<View>,
    pub theme: Arc<Theme>,
    pub theme_name: String,
    pub popup: Popup,
    pub work_mode: WorkMode,
    pub active_plan: Option<PlanFile>,
    pub plan_manager: PlanManager,
    pub plan_sidebar: crate::tui::components::PlanSidebarState,
    pub decision_prompt: crate::tui::components::DecisionPrompt,
    pub should_quit: bool,
    pub input: MultiLineInput,
    pub autocomplete: AutocompletePopup,
    pub file_search: crate::tui::input::FileSearchPopup,
    pub messages: Vec<(String, String)>,
    pub conversation: Vec<ModelMessage>,
    // Processing state - split into streaming vs tool execution to prevent race conditions
    pub is_streaming: bool,               // True while streaming from AI API
    pub is_executing_tools: bool,         // True while tools are running
    pub current_activity: Option<String>, // Current activity for top toolbar (e.g., "thinking", "reading", "writing")

    // Scroll state - centralized scroll management
    pub scroll: ScrollState,

    // Layout state - cached areas for hit testing
    pub layout: LayoutState,

    pub selection: SelectionState,
    pub hover: HoverState,
    pub edge_scroll: EdgeScrollState,
    pub current_model: String,

    // Token tracking
    pub context_tokens_used: usize,

    // AI client
    pub ai_client: Option<AiClient>,
    pub api_key: Option<String>,

    // Multi-provider support
    pub active_provider: ProviderId,
    pub credential_store: CredentialStore,
    pub model_registry: SharedModelRegistry,

    // Tool system
    pub tool_registry: Arc<ToolRegistry>,
    pub cached_ai_tools: Vec<AiTool>,
    /// User-configurable hooks
    pub user_hook_manager: Arc<RwLock<UserHookManager>>,

    // LSP Manager
    pub lsp_manager: Arc<LspManager>,
    /// Extensions to skip prompting for (user said "always skip")
    pub lsp_skip_list: std::collections::HashSet<String>,
    /// Pending LSP install prompt from tool execution
    pub pending_lsp_install: Option<crate::lsp::manager::MissingLspInfo>,

    // Process Registry for background processes
    pub process_registry: Arc<ProcessRegistry>,
    pub running_process_count: usize,
    pub running_process_elapsed: Option<std::time::Duration>,

    // Skills Manager
    pub skills_manager: Arc<RwLock<SkillsManager>>,

    // MCP Manager
    pub mcp_manager: Arc<krusty_core::mcp::McpManager>,
    /// Sender for MCP status updates from background tasks
    pub mcp_status_tx: tokio::sync::mpsc::UnboundedSender<crate::tui::utils::McpStatusUpdate>,

    // WASM Extension Host
    pub wasm_host: Option<Arc<WasmHost>>,

    // Working directory
    pub working_dir: PathBuf,

    // Session management
    pub session_manager: Option<SessionManager>,
    pub current_session_id: Option<String>,
    pub session_title: Option<String>,

    // Title editing state
    pub title_editor: TitleEditor,

    // User preferences
    pub preferences: Option<Preferences>,

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

    // Cache for streaming assistant message index (avoids O(n) scan per delta)
    pub streaming_assistant_idx: Option<usize>,

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
    /// Create new app, optionally with CLI theme override
    pub async fn new(cli_theme: Option<&str>) -> Self {
        let working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let lsp_manager = Arc::new(LspManager::new(working_dir.clone()));

        // Initialize process registry
        let process_registry = Arc::new(ProcessRegistry::new());

        // Initialize WASM extension host
        let extensions_dir = paths::extensions_dir();
        let http_client = reqwest::Client::new();
        let wasm_host = Some(WasmHost::new(http_client, extensions_dir.clone()));
        tracing::info!("WASM extension host initialized at {:?}", extensions_dir);

        // Initialize database path (used by multiple components)
        let db_path = paths::config_dir().join("krusty.db");

        // Initialize user hook manager (load from database)
        let user_hook_manager = Arc::new(RwLock::new(UserHookManager::new()));
        if let Ok(db) = Database::new(&db_path) {
            let hook_count = {
                let mut mgr = user_hook_manager.write().await;
                if let Err(e) = mgr.load(&db) {
                    tracing::warn!("Failed to load user hooks: {}", e);
                }
                mgr.hooks().len()
            };
            if hook_count > 0 {
                tracing::info!("Loaded {} user hooks", hook_count);
            }
        }

        // Initialize tool registry with safety hooks
        let mut tool_registry = ToolRegistry::new();
        tool_registry.add_pre_hook(Arc::new(crate::agent::SafetyHook::new()));
        tool_registry.add_pre_hook(Arc::new(crate::agent::PlanModeHook::new()));
        tool_registry.add_post_hook(Arc::new(crate::agent::LoggingHook::new()));
        // Add user-configurable hooks (after built-in hooks)
        tool_registry.add_pre_hook(Arc::new(UserPreToolHook::new(user_hook_manager.clone())));
        tool_registry.add_post_hook(Arc::new(UserPostToolHook::new(user_hook_manager.clone())));
        let tool_registry = Arc::new(tool_registry);
        register_all_tools(&tool_registry, Some(lsp_manager.clone())).await;
        let cached_ai_tools = tool_registry.get_ai_tools().await;

        // Initialize preferences (load theme)
        let (preferences, saved_theme) = match Database::new(&db_path) {
            Ok(db) => {
                let prefs = Preferences::new(db);
                let theme = prefs.get_theme();
                (Some(prefs), theme)
            }
            Err(e) => {
                tracing::warn!("Failed to initialize preferences: {}", e);
                (None, "krusty".to_string())
            }
        };

        // CLI theme overrides saved preference (if not default)
        let theme_name = cli_theme.unwrap_or(&saved_theme).to_string();
        let theme = THEME_REGISTRY.get_or_default(&theme_name);

        // Initialize session manager (separate connection)
        let session_manager = match Database::new(&db_path) {
            Ok(db) => {
                tracing::info!("Session database initialized at {:?}", db_path);
                Some(SessionManager::new(db))
            }
            Err(e) => {
                tracing::warn!("Failed to initialize session database: {}", e);
                None
            }
        };

        // Initialize plan manager with database path
        let plan_manager = PlanManager::new(db_path).expect("Failed to create plan manager");

        // Migrate legacy file-based plans to database (one-time migration)
        match plan_manager.migrate_legacy_plans() {
            Ok((migrated, skipped)) if migrated > 0 => {
                tracing::info!(
                    "Migrated {} legacy plans to database ({} skipped)",
                    migrated,
                    skipped
                );
            }
            Ok(_) => {} // No plans to migrate
            Err(e) => {
                tracing::warn!("Failed to migrate legacy plans: {}", e);
            }
        }

        // Initialize credential store and active provider
        let credential_store = CredentialStore::load().unwrap_or_else(|e| {
            tracing::warn!("Failed to load credential store: {}", e);
            CredentialStore::default()
        });
        let active_provider = crate::storage::credentials::ActiveProviderStore::load();

        // Initialize model registry with static models from builtin providers
        let model_registry = create_model_registry();
        {
            use crate::ai::models::ModelMetadata;
            use crate::ai::providers::builtin_providers;

            // Load static models from all builtin providers
            for provider in builtin_providers() {
                if provider.models.is_empty() {
                    continue; // Skip dynamic providers (e.g., OpenRouter)
                }
                let models: Vec<ModelMetadata> = provider
                    .models
                    .iter()
                    .map(|m| {
                        let mut meta = ModelMetadata::new(&m.id, &m.display_name, provider.id)
                            .with_context(m.context_window, m.max_output);
                        if let Some(format) = m.reasoning {
                            meta = meta.with_thinking(format);
                        }
                        meta
                    })
                    .collect();
                // Use block_on for sync context (this is during App initialization)
                futures::executor::block_on(model_registry.set_models(provider.id, models));
            }
            tracing::info!("Model registry initialized with static models");

            // Load cached OpenRouter models from preferences
            if let Some(ref prefs) = preferences {
                if let Some(cached_models) = prefs.get_cached_openrouter_models() {
                    futures::executor::block_on(model_registry.set_models(
                        crate::ai::providers::ProviderId::OpenRouter,
                        cached_models.clone(),
                    ));
                    tracing::info!(
                        "Loaded {} cached OpenRouter models from preferences",
                        cached_models.len()
                    );
                }
            }

            // Load cached OpenCode Zen models from preferences
            if let Some(ref prefs) = preferences {
                if let Some(cached_models) = prefs.get_cached_opencodezen_models() {
                    futures::executor::block_on(model_registry.set_models(
                        crate::ai::providers::ProviderId::OpenCodeZen,
                        cached_models.clone(),
                    ));
                    tracing::info!(
                        "Loaded {} cached OpenCode Zen models from preferences",
                        cached_models.len()
                    );
                }
            }

            // Load recent models from preferences
            if let Some(ref prefs) = preferences {
                let recent_ids = prefs.get_recent_models();
                if !recent_ids.is_empty() {
                    futures::executor::block_on(model_registry.set_recent_ids(recent_ids.clone()));
                    tracing::info!("Loaded {} recent models from preferences", recent_ids.len());
                }
            }
        }

        // Load current model from preferences
        let current_model = preferences
            .as_ref()
            .map(|p| p.get_current_model())
            .unwrap_or_else(|| "claude-opus-4-5-20251101".to_string());

        // Initialize skills manager with global skills dir and project-local dir
        let global_skills_dir = paths::config_dir().join("skills");
        let project_skills_dir = Some(working_dir.join(".krusty").join("skills"));
        let skills_manager = Arc::new(RwLock::new(SkillsManager::new(
            global_skills_dir,
            project_skills_dir,
        )));

        // Initialize MCP manager and load config
        let mcp_manager = Arc::new(krusty_core::mcp::McpManager::new(working_dir.clone()));
        let (mcp_status_tx, mcp_status_rx) = tokio::sync::mpsc::unbounded_channel();
        if let Err(e) = mcp_manager.load_config().await {
            tracing::warn!("Failed to load MCP config: {}", e);
        } else {
            // Connect to configured servers in background
            if mcp_manager.has_servers().await {
                let mcp = mcp_manager.clone();
                let registry = tool_registry.clone();
                let status_tx = mcp_status_tx.clone();
                tokio::spawn(async move {
                    if let Err(e) = mcp.connect_all().await {
                        tracing::warn!("MCP server connection errors: {}", e);
                    }
                    // Register MCP tools with the tool registry
                    krusty_core::mcp::tool::register_mcp_tools(mcp.clone(), &registry).await;

                    // Notify main thread to refresh AI tools
                    let tool_count = mcp.get_all_tools().await.len();
                    if tool_count > 0 {
                        let _ = status_tx.send(crate::tui::utils::McpStatusUpdate {
                            success: true,
                            message: format!("MCP initialized ({} tools)", tool_count),
                        });
                    }
                });
            }
        }

        let mut channels = AsyncChannels::new();
        channels.mcp_status = Some(mcp_status_rx);

        Self {
            view: View::StartMenu,
            pending_view_change: None,
            theme: Arc::new(theme.clone()),
            theme_name: theme_name.to_string(),
            popup: Popup::None,
            work_mode: WorkMode::Build,
            active_plan: None,
            plan_manager,
            plan_sidebar: crate::tui::components::PlanSidebarState::default(),
            decision_prompt: crate::tui::components::DecisionPrompt::default(),
            should_quit: false,
            input: MultiLineInput::new(5),
            autocomplete: AutocompletePopup::new(),
            file_search: crate::tui::input::FileSearchPopup::new(working_dir.clone()),
            messages: Vec::new(),
            conversation: Vec::new(),
            is_streaming: false,
            is_executing_tools: false,
            current_activity: None,
            scroll: ScrollState::new(),
            layout: LayoutState::new(),
            selection: SelectionState::default(),
            hover: HoverState::default(),
            edge_scroll: EdgeScrollState::default(),
            current_model,
            context_tokens_used: 0,
            ai_client: None,
            api_key: None,
            active_provider,
            credential_store,
            model_registry,
            tool_registry,
            cached_ai_tools,
            user_hook_manager,
            lsp_manager,
            lsp_skip_list: std::collections::HashSet::new(),
            pending_lsp_install: None,
            process_registry,
            running_process_count: 0,
            running_process_elapsed: None,
            skills_manager,
            mcp_manager,
            mcp_status_tx,
            wasm_host,
            working_dir,
            session_manager,
            current_session_id: None,
            session_title: None,
            title_editor: TitleEditor::new(),
            preferences,
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

            // Streaming assistant index cache
            streaming_assistant_idx: None,

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
        if let Some(metadata) = self.model_registry.try_get_model(&self.current_model) {
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
        self.is_streaming = true;
        self.current_activity = Some("thinking".to_string());
    }

    /// Stop streaming from AI - clears is_streaming flag and related caches
    pub fn stop_streaming(&mut self) {
        self.is_streaming = false;
        self.current_activity = None;
        self.streaming_assistant_idx = None;
    }

    /// Start tool execution - sets is_executing_tools flag
    pub fn start_tool_execution(&mut self) {
        self.is_executing_tools = true;
    }

    /// Stop tool execution - clears is_executing_tools flag
    pub fn stop_tool_execution(&mut self) {
        self.is_executing_tools = false;
        self.current_activity = None;
    }

    /// Apply any pending view change (called at end of event loop iteration)
    pub fn apply_pending_view_change(&mut self) {
        if let Some(view) = self.pending_view_change.take() {
            self.view = view;
        }
    }

    /// Check if busy (streaming OR executing tools)
    pub fn is_busy(&self) -> bool {
        self.is_streaming || self.is_executing_tools
    }

    /// Calculate total lines in messages for scrollbar
    /// Uses the same wrapping logic as render_messages for accurate counting
    /// NOTE: Takes &mut self to populate markdown cache for consistency with render
    pub fn calculate_message_lines(&mut self, width: u16) -> usize {
        use crate::tui::blocks::{BlockType, StreamBlock};
        use crate::tui::state::BlockIndices;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut total = 0;
        let mut indices = BlockIndices::new();
        // Account for borders (2) + scrollbar padding (4) = 6 total
        // MUST match render_messages() which uses: inner.width.saturating_sub(4)
        // where inner.width = area.width - 2 (from block.inner), so total = width - 6
        let inner_width = width.saturating_sub(6) as usize;
        let content_width = width.saturating_sub(6); // Must match inner_width for blocks

        // Pre-render markdown to cache (same as render_messages) to ensure consistent line counts
        self.markdown_cache.check_width(inner_width);

        for (role, content) in &self.messages {
            if let Some((block_type, idx)) = indices.get_and_increment(role) {
                // Handle block types
                let height = match block_type {
                    BlockType::Thinking => self
                        .blocks
                        .thinking
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                    BlockType::Bash => self
                        .blocks
                        .bash
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                    BlockType::TerminalPane => {
                        // Skip pinned terminal - it's rendered at top
                        if self.blocks.pinned_terminal == Some(idx) {
                            None
                        } else {
                            self.blocks
                                .terminal
                                .get(idx)
                                .map(|b| b.height(content_width, &self.theme))
                        }
                    }
                    BlockType::ToolResult => self
                        .blocks
                        .tool_result
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                    BlockType::Read => self
                        .blocks
                        .read
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                    BlockType::Edit => self
                        .blocks
                        .edit
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                    BlockType::Write => self
                        .blocks
                        .write
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                    BlockType::WebSearch => self
                        .blocks
                        .web_search
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                    BlockType::Explore => self
                        .blocks
                        .explore
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                    BlockType::Build => self
                        .blocks
                        .build
                        .get(idx)
                        .map(|b| b.height(content_width, &self.theme)),
                };
                if let Some(h) = height {
                    total += h as usize + 1; // +1 for blank after
                }
            } else if role == "assistant" {
                // Render markdown to cache and get line count (matches render_messages exactly)
                let mut hasher = DefaultHasher::new();
                content.hash(&mut hasher);
                let content_hash = hasher.finish();
                let rendered = self.markdown_cache.get_or_render_with_links(
                    content,
                    content_hash,
                    inner_width,
                    &self.theme,
                );
                total += rendered.lines.len() + 1; // +1 for blank after
            } else {
                // User/system messages - plain text with wrapping
                // Must match render_messages exactly: wrap each line, then blank after
                for line in content.lines() {
                    if line.is_empty() {
                        total += 1;
                    } else {
                        total += count_wrapped_lines(line, inner_width);
                    }
                }
                total += 1; // Blank line after
            }
        }
        total
    }

    /// Start editing the session title
    pub fn start_title_edit(&mut self) {
        if self.view == View::Chat {
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
                (&self.session_manager, &self.current_session_id)
            {
                let _ = manager.update_session_title(session_id, &new_title);
            }
        }
    }

    /// Initialize language servers from loaded WASM extensions
    pub async fn initialize_extension_servers(&self) -> Result<()> {
        let Some(wasm_host) = &self.wasm_host else {
            return Ok(());
        };

        let extensions_dir = paths::extensions_dir();
        if !extensions_dir.exists() {
            return Ok(());
        }

        let worktree = AppWorktreeDelegate::new(self.working_dir.clone());

        let entries = match std::fs::read_dir(&extensions_dir) {
            Ok(e) => e,
            Err(_) => return Ok(()),
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            if let Ok(extension) = wasm_host.load_extension_from_dir(&path).await {
                tracing::info!("Loaded extension: {}", extension.manifest.name);
                self.register_extension_lsp_servers(&extension, wasm_host, worktree.clone())
                    .await;
            }
        }

        Ok(())
    }

    /// Register language servers for a single extension
    /// Used after installing extensions mid-session
    pub async fn register_extension_servers(&self, extension_path: &Path) -> Result<()> {
        let Some(wasm_host) = &self.wasm_host else {
            return Ok(());
        };

        let worktree = AppWorktreeDelegate::new(self.working_dir.clone());
        let extension = wasm_host.load_extension_from_dir(extension_path).await?;
        tracing::info!(
            "Registering servers for extension: {}",
            extension.manifest.name
        );
        self.register_extension_lsp_servers(&extension, wasm_host, worktree)
            .await;

        Ok(())
    }

    /// Helper to register all language servers from a loaded extension
    async fn register_extension_lsp_servers(
        &self,
        extension: &crate::extensions::wasm_host::WasmExtension,
        wasm_host: &crate::extensions::wasm_host::WasmHost,
        worktree: std::sync::Arc<AppWorktreeDelegate>,
    ) {
        for (server_id, entry) in &extension.manifest.language_servers {
            match extension
                .language_server_command(
                    server_id.clone().into(),
                    worktree.clone()
                        as std::sync::Arc<dyn crate::extensions::types::WorktreeDelegate>,
                )
                .await
            {
                Ok(mut command) => {
                    // Resolve relative command path to absolute path
                    let extension_work_dir = wasm_host.work_dir.join(&extension.manifest.id);
                    let command_path = std::path::Path::new(&command.command);
                    if command_path.is_relative() {
                        let absolute_path = extension_work_dir.join(command_path);
                        command.command = absolute_path.to_string_lossy().into_owned();
                    }

                    // Use languages list, falling back to singular language field
                    let langs: Vec<&str> = if entry.languages.is_empty() {
                        entry.language.as_deref().into_iter().collect()
                    } else {
                        entry.languages.iter().map(|s| s.as_str()).collect()
                    };

                    let file_extensions: Vec<String> = langs
                        .iter()
                        .flat_map(|lang| Self::language_to_extensions(lang))
                        .collect();

                    let full_server_id = format!("{}-{}", extension.manifest.id, server_id);

                    tracing::info!(
                        "Registering LSP {} for languages {:?} (extensions: {:?})",
                        full_server_id,
                        langs,
                        file_extensions
                    );

                    if let Err(e) = self
                        .lsp_manager
                        .register_from_extension(
                            &full_server_id,
                            command,
                            file_extensions,
                            50, // Extensions get lower priority than builtins
                        )
                        .await
                    {
                        tracing::error!("Failed to register LSP {}: {}", full_server_id, e);
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to get language server command for {}/{}: {}",
                        extension.manifest.id,
                        server_id,
                        e
                    );
                }
            }
        }
    }

    /// Convert language name to file extensions
    fn language_to_extensions(language: &str) -> Vec<String> {
        match language.to_lowercase().as_str() {
            "rust" => vec!["rs".into()],
            "python" => vec!["py".into(), "pyi".into()],
            "javascript" => vec!["js".into(), "mjs".into(), "cjs".into()],
            "typescript" => vec!["ts".into(), "mts".into(), "cts".into()],
            "typescriptreact" | "tsx" => vec!["tsx".into()],
            "javascriptreact" | "jsx" => vec!["jsx".into()],
            "go" => vec!["go".into()],
            "c" => vec!["c".into(), "h".into()],
            "cpp" | "c++" => vec!["cpp".into(), "hpp".into(), "cc".into(), "cxx".into()],
            "java" => vec!["java".into()],
            "ruby" => vec!["rb".into()],
            "lua" => vec!["lua".into()],
            "zig" => vec!["zig".into()],
            "toml" => vec!["toml".into()],
            "json" => vec!["json".into()],
            "yaml" => vec!["yaml".into(), "yml".into()],
            "markdown" => vec!["md".into()],
            "html" => vec!["html".into(), "htm".into()],
            "css" => vec!["css".into()],
            "scss" => vec!["scss".into()],
            "sass" => vec!["sass".into()],
            "vue" => vec!["vue".into()],
            "svelte" => vec!["svelte".into()],
            "elixir" => vec!["ex".into(), "exs".into()],
            "erlang" => vec!["erl".into()],
            "haskell" => vec!["hs".into()],
            "ocaml" => vec!["ml".into(), "mli".into()],
            "kotlin" => vec!["kt".into(), "kts".into()],
            "swift" => vec!["swift".into()],
            "scala" => vec!["scala".into()],
            "clojure" => vec!["clj".into(), "cljs".into(), "cljc".into()],
            "php" => vec!["php".into()],
            "r" => vec!["r".into(), "R".into()],
            "julia" => vec!["jl".into()],
            "dart" => vec!["dart".into()],
            "gleam" => vec!["gleam".into()],
            _ => vec![language.to_lowercase()],
        }
    }

    /// Try to load existing authentication for the active provider
    pub async fn try_load_auth(&mut self) -> Result<()> {
        // Try credential store for all providers (unified API key storage)
        if let Some(key) = self.credential_store.get(&self.active_provider).cloned() {
            let config = self.create_client_config();
            self.ai_client = Some(AiClient::with_api_key(config, key.clone()));
            self.api_key = Some(key);
            self.register_explore_tool_if_client().await;
            return Ok(());
        }

        Ok(())
    }

    /// Register explore and build tools if client is available
    async fn register_explore_tool_if_client(&mut self) {
        let client = self.create_ai_client();

        if let Some(client) = client {
            let client = Arc::new(client);

            // Register explore tool
            register_explore_tool(
                &self.tool_registry,
                client.clone(),
                self.cancellation.clone(),
            )
            .await;

            // Register build tool (The Kraken)
            register_build_tool(&self.tool_registry, client, self.cancellation.clone()).await;

            // Update cached tools so API knows about explore and build
            self.cached_ai_tools = self.tool_registry.get_ai_tools().await;
            tracing::info!(
                "Registered explore and build tools, total tools: {}",
                self.cached_ai_tools.len()
            );
        }
    }

    /// Create AnthropicConfig for the current active provider
    pub fn create_client_config(&self) -> crate::ai::client::AiClientConfig {
        use crate::ai::models::ApiFormat;
        use crate::ai::providers::get_provider;

        let provider = get_provider(self.active_provider)
            .unwrap_or_else(|| get_provider(ProviderId::Anthropic).unwrap());

        // Get API format from model metadata (for OpenCode Zen multi-format routing)
        let api_format = self
            .model_registry
            .try_get_model(&self.current_model)
            .map(|m| m.api_format)
            .unwrap_or(ApiFormat::Anthropic);

        crate::ai::client::AiClientConfig {
            model: self.current_model.clone(),
            max_tokens: crate::constants::ai::MAX_OUTPUT_TOKENS,
            base_url: Some(provider.base_url.clone()),
            auth_header: provider.auth_header,
            provider_id: provider.id,
            api_format,
        }
    }

    /// Create an AI client with the current provider configuration
    pub fn create_ai_client(&self) -> Option<AiClient> {
        let config = self.create_client_config();
        self.api_key
            .as_ref()
            .map(|key| AiClient::with_api_key(config, key.clone()))
    }

    /// Set API key for current provider and create client
    pub fn set_api_key(&mut self, key: String) {
        // Create client with provider config
        let config = self.create_client_config();
        self.ai_client = Some(AiClient::with_api_key(config, key.clone()));
        self.api_key = Some(key.clone());

        // Save to credential store (unified storage for all providers)
        self.credential_store.set(self.active_provider, key);
        if let Err(e) = self.credential_store.save() {
            tracing::warn!("Failed to save credential store: {}", e);
        }
    }

    /// Switch to a different provider
    /// Automatically translates the current model to the equivalent in the new provider
    pub fn switch_provider(&mut self, provider_id: ProviderId) {
        use crate::ai::providers::{get_provider, translate_model_or_default};

        let previous_provider = self.active_provider;
        self.active_provider = provider_id;

        // Save active provider selection
        if let Err(e) = crate::storage::credentials::ActiveProviderStore::save(provider_id) {
            tracing::warn!("Failed to save active provider: {}", e);
        }

        // Translate model ID to the new provider's format
        // e.g., "claude-opus-4-5-20251101" -> "anthropic/claude-opus-4.5" for OpenRouter
        let translated =
            translate_model_or_default(&self.current_model, previous_provider, provider_id);

        if translated != self.current_model {
            tracing::info!(
                "Translated model '{}' -> '{}' for {}",
                self.current_model,
                translated,
                provider_id
            );
            self.current_model = translated.clone();

            // Save translated model to preferences
            if let Some(ref prefs) = self.preferences {
                if let Err(e) = prefs.set_current_model(&translated) {
                    tracing::warn!("Failed to save current model: {}", e);
                }
            }
        }

        // Validate the model exists for this provider (fallback to default if not)
        if let Some(provider) = get_provider(provider_id) {
            if !provider.has_model(&self.current_model) {
                let default = provider.default_model().to_string();
                tracing::info!(
                    "Model '{}' not available for {}, using default '{}'",
                    self.current_model,
                    provider_id,
                    default
                );
                self.current_model = default.clone();

                if let Some(ref prefs) = self.preferences {
                    if let Err(e) = prefs.set_current_model(&default) {
                        tracing::warn!("Failed to save current model: {}", e);
                    }
                }
            }
        }

        // Try to load credentials for the new provider
        if let Some(key) = self.credential_store.get(&provider_id).cloned() {
            let config = self.create_client_config();
            self.ai_client = Some(AiClient::with_api_key(config, key.clone()));
            self.api_key = Some(key);
            tracing::info!("Switched to provider {} (loaded existing key)", provider_id);
        } else {
            // No stored key - user will need to authenticate
            self.ai_client = None;
            self.api_key = None;
            tracing::info!(
                "Switched to provider {} (requires authentication)",
                provider_id
            );
        }
    }

    /// Get list of configured provider IDs (ones with API keys)
    pub fn configured_providers(&self) -> Vec<ProviderId> {
        self.credential_store.configured_providers()
    }

    /// Check if authenticated
    pub fn is_authenticated(&self) -> bool {
        self.ai_client.is_some()
    }

    /// Set theme and persist to preferences
    pub fn set_theme(&mut self, name: &str) {
        let theme = THEME_REGISTRY.get_or_default(name);
        self.theme = Arc::new(theme.clone());
        self.theme_name = name.to_string();

        // Update menu animator with theme color
        let accent_rgb = theme.get_bubble_rgb();
        self.menu_animator.set_theme_color(accent_rgb);

        // Save to preferences
        if let Some(ref prefs) = self.preferences {
            if let Err(e) = prefs.set_theme(name) {
                tracing::warn!("Failed to save theme preference: {}", e);
            }
        }
    }

    /// Preview theme without saving to preferences (for live preview)
    pub fn preview_theme(&mut self, name: &str) {
        let theme = THEME_REGISTRY.get_or_default(name);
        self.theme = Arc::new(theme.clone());
        self.theme_name = name.to_string();

        // Update menu animator with theme color
        let accent_rgb = theme.get_bubble_rgb();
        self.menu_animator.set_theme_color(accent_rgb);
        // Don't save to preferences - this is just a preview
    }

    /// Restore theme to original (cancel preview)
    pub fn restore_original_theme(&mut self) {
        if let Some(original) = self
            .popups
            .theme
            .get_original_theme_name()
            .map(|s| s.to_string())
        {
            self.preview_theme(&original);
        }
    }

    /// Poll bash output channel and update BashBlock with streaming output
    fn poll_bash_output(&mut self) {
        // Take the receiver temporarily to poll it
        if let Some(mut rx) = self.channels.bash_output.take() {
            // Poll all available chunks (non-blocking)
            loop {
                match rx.try_recv() {
                    Ok(chunk) => {
                        // Find the BashBlock with matching tool_use_id
                        // First try to find by ID, then fall back to last
                        let block_idx = self
                            .blocks
                            .bash
                            .iter()
                            .position(|b| b.tool_use_id() == Some(&chunk.tool_use_id))
                            .or_else(|| {
                                if self.blocks.bash.is_empty() {
                                    None
                                } else {
                                    Some(self.blocks.bash.len() - 1)
                                }
                            });
                        let block = block_idx.and_then(|i| self.blocks.bash.get_mut(i));

                        if let Some(block) = block {
                            if chunk.is_complete {
                                // Mark block as complete with exit code
                                let exit_code = chunk.exit_code.unwrap_or(0);
                                tracing::info!(
                                    tool_use_id = %chunk.tool_use_id,
                                    exit_code = exit_code,
                                    "Bash block complete signal received"
                                );
                                block.complete(exit_code);

                                // Update ProcessRegistry status (fire and forget)
                                let registry = self.process_registry.clone();
                                let tool_id = chunk.tool_use_id.clone();
                                tokio::spawn(async move {
                                    let status = if exit_code == 0 {
                                        crate::process::ProcessStatus::Completed {
                                            exit_code,
                                            duration_ms: 0, // Duration tracked by block
                                        }
                                    } else {
                                        crate::process::ProcessStatus::Failed {
                                            error: format!("Exit code: {}", exit_code),
                                            duration_ms: 0,
                                        }
                                    };
                                    registry.update_status(&tool_id, status).await;
                                });
                            } else if !chunk.chunk.is_empty() {
                                // Append output chunk
                                block.append(&chunk.chunk);
                            }
                        }
                        if self.scroll.auto_scroll {
                            self.scroll.request_scroll_to_bottom();
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        // No more data available, put receiver back
                        self.channels.bash_output = Some(rx);
                        break;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        // Channel closed - complete any remaining streaming blocks
                        // This usually means the tool finished but we missed the completion signal
                        tracing::debug!("Bash output channel disconnected");
                        for block in self.blocks.bash.iter_mut() {
                            // Skip background blocks - they're tracked by ProcessRegistry
                            if block.background_process_id().is_some() {
                                continue;
                            }
                            if block.is_streaming() {
                                tracing::info!("Completing bash block on channel disconnect (assuming success)");
                                block.complete(0); // Assume success - channel disconnect usually means clean exit

                                // Update ProcessRegistry if we have a tool_use_id
                                if let Some(tool_id) = block.tool_use_id() {
                                    let registry = self.process_registry.clone();
                                    let tool_id = tool_id.to_string();
                                    tokio::spawn(async move {
                                        let status = crate::process::ProcessStatus::Completed {
                                            exit_code: 0,
                                            duration_ms: 0,
                                        };
                                        registry.update_status(&tool_id, status).await;
                                    });
                                }
                            }
                        }
                        break;
                    }
                }
            }
        }
    }

    /// Poll explore progress channel and update ExploreBlock with agent progress
    fn poll_explore_progress(&mut self) {
        if let Some(mut rx) = self.channels.explore_progress.take() {
            loop {
                match rx.try_recv() {
                    Ok(progress) => {
                        // Find matching ExploreBlock by tool_use_id (derived from task_id prefix)
                        // Task IDs are like "dir-0", "file-1", "main" - we find the parent explore block
                        // by looking for blocks that are still streaming
                        for block in self.blocks.explore.iter_mut() {
                            if block.is_streaming() {
                                block.update_progress(progress.clone());
                                break;
                            }
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        self.channels.explore_progress = Some(rx);
                        break;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        tracing::debug!("Explore progress channel disconnected");
                        break;
                    }
                }
            }
        }
    }

    /// Poll build progress channel and update BuildBlock with builder progress
    fn poll_build_progress(&mut self) {
        if let Some(mut rx) = self.channels.build_progress.take() {
            loop {
                match rx.try_recv() {
                    Ok(progress) => {
                        // Find matching BuildBlock that is still streaming
                        for block in self.blocks.build.iter_mut() {
                            if block.is_streaming() {
                                block.update_progress(progress.clone());
                                break;
                            }
                        }

                        // Auto-complete plan task if specified
                        if let Some(ref task_id) = progress.completed_plan_task {
                            if let Some(ref mut plan) = self.active_plan {
                                if plan.check_task(task_id) {
                                    tracing::debug!(task_id = %task_id, "Kraken auto-completed plan task");
                                    if let Err(e) = self.plan_manager.save_plan(plan) {
                                        tracing::warn!(
                                            "Failed to save plan after task completion: {}",
                                            e
                                        );
                                    }
                                }
                            }
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        self.channels.build_progress = Some(rx);
                        break;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        tracing::debug!("Build progress channel disconnected");
                        break;
                    }
                }
            }
        }
    }

    /// Poll terminal panes for PTY output and update cursor animations
    fn poll_terminal_panes(&mut self) {
        self.blocks.poll_terminals();
    }

    /// Poll /init exploration progress and result
    fn poll_init_exploration(&mut self) {
        use crate::tui::handlers::commands::generate_krab_from_exploration;

        // Poll progress channel - route to ExploreBlock
        if let Some(mut rx) = self.channels.init_progress.take() {
            loop {
                match rx.try_recv() {
                    Ok(progress) => {
                        // Find the init ExploreBlock and update it
                        if let Some(ref explore_id) = self.init_explore_id {
                            for block in self.blocks.explore.iter_mut() {
                                if block.tool_use_id() == Some(explore_id.as_str()) {
                                    block.update_progress(progress.clone());
                                    break;
                                }
                            }
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        self.channels.init_progress = Some(rx);
                        break;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        break;
                    }
                }
            }
        }

        // Poll result channel for completion
        if let Some(mut rx) = self.channels.init_exploration.take() {
            match rx.try_recv() {
                Ok(result) => {
                    // Complete the ExploreBlock
                    if let Some(ref explore_id) = self.init_explore_id {
                        for block in self.blocks.explore.iter_mut() {
                            if block.tool_use_id() == Some(explore_id.as_str()) {
                                block.complete(String::new());
                                break;
                            }
                        }
                    }
                    self.init_explore_id = None;

                    if result.success {
                        // Generate KRAB.md from exploration results
                        let project_name = self
                            .working_dir
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Project".to_string());
                        let languages = self.detect_project_languages();

                        let krab_path = self.working_dir.join("KRAB.md");
                        let is_regenerate = krab_path.exists();

                        // Try to preserve user's "Notes for AI" section if regenerating
                        let preserved_notes = if is_regenerate {
                            std::fs::read_to_string(&krab_path)
                                .ok()
                                .and_then(|content| {
                                    content.find("## Notes for AI").map(|pos| {
                                        let notes_section = &content[pos..];
                                        notes_section
                                            .lines()
                                            .skip(1)
                                            .skip_while(|l| l.starts_with("<!--") || l.is_empty())
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    })
                                })
                                .filter(|s| !s.trim().is_empty())
                        } else {
                            None
                        };

                        let mut content =
                            generate_krab_from_exploration(&project_name, &languages, &result);

                        if let Some(notes) = preserved_notes {
                            content.push_str(&notes);
                            content.push('\n');
                        }

                        match std::fs::write(&krab_path, &content) {
                            Ok(_) => {
                                let action = if is_regenerate {
                                    "Regenerated"
                                } else {
                                    "Created"
                                };
                                // Add as assistant message (natural conversation flow)
                                self.messages.push((
                                    "assistant".to_string(),
                                    format!(
                                        "{} **KRAB.md** ({} bytes) from codebase analysis.\n\n\
                                        This file is now auto-injected into every AI conversation. \
                                        Edit it to customize how I understand your project.",
                                        action,
                                        content.len()
                                    ),
                                ));
                            }
                            Err(e) => {
                                self.messages.push((
                                    "assistant".to_string(),
                                    format!("Failed to write KRAB.md: {}", e),
                                ));
                            }
                        }
                    } else {
                        let error = result.error.unwrap_or_else(|| "Unknown error".to_string());
                        self.messages.push((
                            "assistant".to_string(),
                            format!("Exploration failed: {}", error),
                        ));
                    }
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Empty) => {
                    self.channels.init_exploration = Some(rx);
                }
                Err(tokio::sync::oneshot::error::TryRecvError::Closed) => {
                    self.init_explore_id = None;
                    self.messages.push((
                        "assistant".to_string(),
                        "Exploration was cancelled.".to_string(),
                    ));
                }
            }
        }
    }

    /// Poll ProcessRegistry for background process status updates
    /// Updates BashBlocks that are tracking background processes
    fn poll_background_processes(&mut self) {
        // Get list of processes without blocking
        let Some(processes) = self.process_registry.try_list() else {
            return;
        };

        // Check each background BashBlock
        for block in self.blocks.bash.iter_mut() {
            // Clone process_id to avoid borrow conflict with block.complete()
            let Some(process_id) = block.background_process_id().map(|s| s.to_string()) else {
                continue;
            };

            // Find matching process in registry
            if let Some(info) = processes.iter().find(|p| p.id == process_id) {
                // Check if process has completed and block is still streaming
                if !info.is_running() && block.is_streaming() {
                    // Process finished - update block status
                    let exit_code = match &info.status {
                        crate::process::ProcessStatus::Completed { exit_code, .. } => *exit_code,
                        crate::process::ProcessStatus::Failed { .. } => 1,
                        crate::process::ProcessStatus::Killed { .. } => 137, // SIGKILL
                        crate::process::ProcessStatus::Running
                        | crate::process::ProcessStatus::Suspended => continue, // Still alive
                    };
                    block.complete(exit_code);
                    tracing::info!(
                        process_id = %process_id,
                        exit_code = exit_code,
                        "Background BashBlock completed from ProcessRegistry"
                    );
                }
            }
        }
    }

    /// Poll MCP status updates from background connection tasks
    fn poll_mcp_status(&mut self) {
        if let Some(mut rx) = self.channels.mcp_status.take() {
            loop {
                match rx.try_recv() {
                    Ok(update) => {
                        // Update popup status message
                        let status_msg = if update.success {
                            format!("✓ {}", update.message)
                        } else {
                            format!("✗ {}", update.message)
                        };
                        self.popups.mcp.set_status(status_msg);

                        // Refresh server list to show updated state
                        self.refresh_mcp_popup();

                        // Refresh cached AI tools so new MCP tools are sent to the API
                        if update.success {
                            self.cached_ai_tools =
                                futures::executor::block_on(self.tool_registry.get_ai_tools());
                            tracing::info!(
                                "Refreshed AI tools after MCP update, total: {}",
                                self.cached_ai_tools.len()
                            );
                        }
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                        self.channels.mcp_status = Some(rx);
                        break;
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        break;
                    }
                }
            }
        }
    }

    /// Tick all animations. Returns true if any animation is still running.
    fn tick_blocks(&mut self) -> bool {
        let blocks = self.blocks.tick_all();
        self.popups.pinch.tick();
        let sidebar = self.plan_sidebar.tick();

        if self.plan_sidebar.should_clear_plan() {
            self.active_plan = None;
            tracing::info!("Plan cleared after sidebar collapse");
        }

        use crate::tui::popups::pinch::PinchStage;
        let pinch_active = matches!(
            self.popups.pinch.stage,
            PinchStage::Summarizing { .. } | PinchStage::Creating
        );

        blocks || sidebar || pinch_active || self.view == View::StartMenu
    }

    /// Close a terminal pane by index
    pub fn close_terminal(&mut self, idx: usize) {
        // Get the process_id before we close (needed for message lookup)
        let process_id = if idx < self.blocks.terminal.len() {
            self.blocks.terminal[idx]
                .get_process_id()
                .map(|s| s.to_string())
        } else {
            None
        };

        // Unregister from process registry before removing
        if let Some(ref id) = process_id {
            let registry = self.process_registry.clone();
            let id_clone = id.clone();
            tokio::spawn(async move {
                registry.unregister(&id_clone).await;
            });
        }

        // Close the terminal (handles focus/pin adjustments)
        self.blocks.close_terminal(idx);

        // Remove the corresponding "terminal" message by process_id (reliable lookup)
        if let Some(ref pid) = process_id {
            if let Some(msg_idx) = self
                .messages
                .iter()
                .position(|(role, content)| role == "terminal" && content == pid)
            {
                self.messages.remove(msg_idx);
            }
        }
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
            .credential_store
            .get(&crate::ai::providers::ProviderId::OpenRouter)
            .is_some()
        {
            let should_refresh = self
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
            .credential_store
            .get(&crate::ai::providers::ProviderId::OpenCodeZen)
            .is_some()
        {
            let should_refresh = self
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
        self.lsp_manager.register_all_builtins(&downloader).await;

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
            // Only use DISAMBIGUATE_ESCAPE_CODES - REPORT_ALL_KEYS_AS_ESCAPE_CODES
            // breaks Shift+key for special characters (e.g., Shift+1 becomes '1'+SHIFT, not '!')
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

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

    /// Main event loop
    async fn main_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        // Use async event stream to avoid blocking the runtime
        // This fixes the issue where the app freezes when mouse leaves terminal
        let mut event_stream = EventStream::new();

        loop {
            if let Some(area) = self.layout.input_area {
                self.input.set_width(area.width);
            }

            // Update running process count and elapsed time for status bar (non-blocking)
            if let Some(count) = self.process_registry.try_running_count() {
                self.running_process_count = count;
            }
            self.running_process_elapsed = self.process_registry.try_oldest_running_elapsed();

            // Keep process popup updated while open (non-blocking)
            if self.popup == Popup::ProcessList {
                if let Some(processes) = self.process_registry.try_list() {
                    self.popups.process.update(processes);
                }
            }

            // Process streaming events (extracted to handlers/stream_events.rs)
            self.process_stream_events();

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
            if self.view == View::StartMenu {
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

            // Poll /init exploration progress and result
            self.poll_init_exploration();

            // Poll MCP status updates from background tasks
            self.poll_mcp_status();

            // Poll update status and show toasts
            self.poll_update_status();

            // Poll terminal panes for output updates and cursor blink
            self.poll_terminal_panes();

            // Poll ProcessRegistry for background process status updates
            self.poll_background_processes();

            // Tick toasts (auto-dismiss expired) - mark dirty if any expired
            if self.toasts.tick() {
                self.needs_redraw = true;
            }

            // Tick all animation blocks (before render, not during)
            // Returns true if any block is still animating
            if self.tick_blocks() {
                self.needs_redraw = true;
            }

            // Process continuous edge scrolling during selection
            if self.edge_scroll.direction.is_some() {
                self.process_edge_scroll();
                self.needs_redraw = true;
            }

            // Always redraw if streaming is active (receiving deltas)
            if self.is_streaming {
                self.needs_redraw = true;
            }

            // Only render if something changed
            if self.needs_redraw {
                terminal.draw(|f| self.ui(f))?;
                self.needs_redraw = false;
            }

            // 60fps polling - edge scroll needs faster polling for smooth scrolling
            let poll_timeout = if self.edge_scroll.direction.is_some() {
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
                        match event {
                            Event::Key(key) => {
                                self.handle_key(key.code, key.modifiers);
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
                                self.needs_redraw = true;
                            }
                            _ => {}
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
