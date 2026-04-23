use super::*;

/// Application services and external systems
pub struct AppServices {
    // Plan/session storage
    pub plan_manager: Option<PlanManager>,
    pub session_manager: Option<SessionManager>,
    pub preferences: Option<Preferences>,

    // Credentials/models
    pub credential_store: CredentialStore,
    pub model_registry: SharedModelRegistry,

    // Tool system
    pub tool_registry: Arc<ToolRegistry>,
    pub cached_ai_tools: Vec<AiTool>,
    pub user_hook_manager: Arc<RwLock<UserHookManager>>,

    // Extensions (initialized for future tool dispatch wiring)
    pub _wasm_host: Option<Arc<WasmHost>>,
    pub plugin_manager: Option<Arc<PluginManager>>,

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
    /// Flag to trigger auto-pinch fallback after response completes
    pub pending_auto_pinch: bool,
    /// Reason recorded for the pending auto-pinch fallback
    pub pending_auto_pinch_reason: Option<String>,
    /// Auto-pinch in progress (bypasses popup when AI is busy)
    pub auto_pinch_in_progress: bool,
    /// When the current pinch summarization started
    pub summarization_started_at: Option<Instant>,
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
    /// Cached git status for status bar display
    pub git_status: Option<krusty_core::git::GitStatusSummary>,
    /// Last git status poll timestamp
    pub last_git_status_poll: Instant,
    /// Last installed-plugin catalog poll timestamp
    pub last_plugin_catalog_poll: Instant,
    /// Working directory
    pub working_dir: PathBuf,
    /// Current session ID
    pub current_session_id: Option<String>,
    /// Child session created by an automatic pinch handoff.
    pub pending_pinched_session_id: Option<String>,
    /// Session title
    pub session_title: Option<String>,
    /// Title editing state
    pub title_editor: TitleEditor,
    /// Async channel receivers
    pub channels: AsyncChannels,
    /// Dynamic model providers currently being refreshed
    pub dynamic_model_fetches: std::collections::HashSet<ProviderId>,
    /// /init exploration ID
    pub init_explore_id: Option<String>,
    /// Cached languages for /init
    pub cached_init_languages: Option<Vec<String>>,
    /// Agent event bus
    pub event_bus: AgentEventBus,
    /// Agent state
    pub agent_state: AgentState,
    /// Agent config
    pub agent_config: AgentConfig,
    /// Agent cancellation token
    pub cancellation: AgentCancellation,
    /// Extended thinking level (Tab cycles levels for Codex models)
    pub thinking_level: ThinkingLevel,
    /// Clipboard images pending resolution
    pub pending_clipboard_images: std::collections::HashMap<String, (usize, usize, Vec<u8>)>,
    /// Block manager (owns all block types)
    pub blocks: BlockManager,
    /// Tool result cache for rendering
    pub tool_results: ToolResultCache,
    /// Attached files mapping
    pub attached_files: std::collections::HashMap<String, PathBuf>,
    /// Permission mode (supervised/autonomous)
    pub permission_mode: PermissionMode,
    /// When a tool approval prompt was shown (for timeout)
    pub approval_requested_at: Option<Instant>,
    /// AskUserQuestion tool calls stored between ToolCallComplete and AwaitingInput events
    pub pending_ask_user_calls: Vec<AiToolCall>,
    /// Just updated flag
    pub just_updated: bool,
    /// Update status
    pub update_status: Option<krusty_core::updater::UpdateStatus>,
    /// Should quit flag
    pub should_quit: bool,
    /// Snapshot of installed plugin versions for update detection
    pub plugin_versions: HashMap<String, String>,
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
            pending_auto_pinch_reason: None,
            auto_pinch_in_progress: false,
            summarization_started_at: None,
            ai_client: None,
            api_key: None,
            active_provider,
            process_registry,
            running_process_count: 0,
            running_process_elapsed: None,
            git_status: None,
            last_git_status_poll: Instant::now() - Duration::from_secs(60),
            last_plugin_catalog_poll: Instant::now() - Duration::from_secs(60),
            working_dir,
            current_session_id: None,
            pending_pinched_session_id: None,
            session_title: None,
            title_editor: TitleEditor::new(),
            channels: AsyncChannels::new(),
            dynamic_model_fetches: std::collections::HashSet::new(),
            init_explore_id: None,
            cached_init_languages: None,
            event_bus: AgentEventBus::new(),
            agent_state: AgentState::new(),
            agent_config: AgentConfig::default(),
            cancellation: AgentCancellation::new(),
            thinking_level: ThinkingLevel::Off,
            pending_clipboard_images: std::collections::HashMap::new(),
            blocks: BlockManager::new(),
            tool_results: ToolResultCache::new(),
            attached_files: std::collections::HashMap::new(),
            permission_mode: PermissionMode::Supervised,
            approval_requested_at: None,
            pending_ask_user_calls: Vec::new(),
            just_updated: false,
            update_status: None,
            should_quit: false,
            plugin_versions: HashMap::new(),
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
