//! Root Mitsuro desktop window: Codex-like chrome + app-server / fixture turns.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use base64::Engine as _;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    div, AppContext as _, Context, Entity, FocusHandle, Focusable, ImageFormat,
    InteractiveElement as _, IntoElement, ParentElement as _, PathPromptOptions, Render,
    ScrollHandle, SharedString, Styled as _, Window,
};
use gpui_component::input::{InputEvent, InputState};
use mitsuro_desktop_backend::{
    activity_item_fields, command_execution_fields, file_change_fields,
    fixture_demo_account_response, fixture_demo_collaboration_modes, fixture_demo_config,
    fixture_demo_environments, fixture_demo_mcp_servers, fixture_demo_models, fixture_demo_plugins,
    fixture_demo_rate_limits, fixture_demo_skills, fixture_demo_usage, join_abs,
    load_sample_turn_events, normalize_abs_path, summarize_file_changes, Account, ActivityFields,
    AgentBackend, ApprovalChoice, BackendKind, BackendSelection, BackendSessionId,
    CancelLoginAccountParams, CancelLoginAccountStatus, CollaborationModeListParams,
    CollaborationModeMask, ConfigReadParams, ConversationAudio, ConversationImage,
    ConversationReference, ConversationReferenceKind, CreateSession, DesktopBackend,
    EnvironmentAddParams, EnvironmentInfoParams, EnvironmentInfoResponse, EnvironmentStatusParams,
    EnvironmentStatusResponse, EnvironmentSummary, FixtureBackend, FsReadDirectoryEntry,
    FsReadDirectoryParams, FsReadFileParams, FuzzyFileSearchParams, FuzzyFileSearchResult,
    GetAccountParams, GetAccountRateLimitsResponse, GetAccountTokenUsageResponse,
    LifecycleNotification, ListMcpServerStatusParams, LiveApprovalBridge, LoginAccountParams,
    McpAuthStatus, McpElicitationMode, McpServerInfo, McpServerOauthLoginCompleted,
    McpServerOauthLoginParams, McpServerStatus, MessageRole, ModeKind, ModelInfo, ModelListParams,
    ModelServiceTier, PendingApproval, PendingMcpElicitation, PendingUserInput, PlanType,
    PluginInstallParams, PluginInterface, PluginListParams, PluginSource, PluginSummary,
    PluginUninstallParams, ProcessKillParams, ProcessSpawnParams, ProcessWriteStdinParams,
    ProductAccessMode, ProductAttachment, ProductBackend, ProductExtension, ProductFileMatch,
    ProductHiveSnapshot, ProductMcpServer, ProductModel, ProductProcess, ProductReview,
    ProductReviewTarget, ProductSchedule, ProductSkill, ProductSpeedMode, ProductSteer,
    ProductTurn, ProductWorkMode, RealtimeEvent, RealtimeOutputModality, RealtimeVoice,
    RealtimeVoicesList, ReasoningEffortOption, SessionDelegationProjection, SessionSummary,
    SkillMetadata, SkillsListParams, ThreadArchiveParams, ThreadDeleteParams, ThreadForkParams,
    ThreadGoalClearParams, ThreadGoalGetParams, ThreadGoalSetParams, ThreadGoalStatus,
    ThreadListParams, ThreadRealtimeAppendAudioParams, ThreadRealtimeAudioChunk,
    ThreadRealtimeStartParams, ThreadRealtimeStopParams, ThreadSetNameParams, ThreadSummary,
    ThreadUnarchiveParams, TurnInterruptParams, TurnStreamEvent, DEFAULT_LIVE_TURN_TIMEOUT,
    FIXTURE_PROJECT_ROOT,
};

use crate::browser::open_system_browser;
#[cfg(feature = "browser-native")]
use crate::browser::NativeWebViewHost;
use crate::browser::{create_default_host, BrowserHost, DesktopBrowserHost};
use crate::components;
use crate::demo::{
    self, DemoAudioAttachment, DemoAudioSource, DemoGoal, DemoGoalStatus, DemoImageAttachment,
    DemoImageSource, DemoMessage, DemoMessageKind, DemoPlanItem, DemoReferenceAttachment,
    DemoReferenceKind, DemoThread, ThreadSurface,
};
use crate::preferences::DesktopPreferences;
use crate::theme;

/// Top-level product shell mode (ChatGPT + Codex desktop unified chrome).
///
/// Bar home sidebar drives Chat/Codex + stub routes (PRs / Sites / Scheduled /
/// Plugins). Activity rail remains for advanced surfaces (Work / Atlas / …).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ProductMode {
    /// Simplified conversational chat surface (`mode=chat` threads).
    Chat,
    /// Long-running goals / plans.
    Work,
    /// Agent threads — current main product surface.
    #[default]
    Codex,
    /// Atlas / browser-use panel.
    Atlas,
    /// Terminal / process panel.
    Terminal,
    /// Files panel (`fs/*` + `fuzzyFileSearch`).
    Files,
    /// Computer-use / environment status panel.
    Computer,
    /// Extensions: MCP servers + plugins (sidebar "Plugins").
    Extensions,
    /// Settings (two-column tree matching ChatGPT/Codex desktop).
    Settings,
    /// Pull requests destination (sidebar).
    PullRequests,
    /// Sites destination (sidebar).
    Sites,
    /// Scheduled tasks destination (sidebar).
    Scheduled,
}

/// Plugins marketplace category chips (Public catalog vs Personal / MCP).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PluginsFilter {
    #[default]
    Public,
    Personal,
    Mcp,
}

/// Top-level Plugins surface tab (Plugins marketplace vs Skills).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PluginsSurfaceTab {
    #[default]
    Plugins,
    Skills,
}

/// Settings left-nav sections (bar 1:1 Personal / Integrations / Coding / Archived).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Hash)]
pub enum SettingsSection {
    #[default]
    General,
    LinuxDesktop,
    Import,
    Profile,
    Appearance,
    Voice,
    Configuration,
    Personalization,
    Pets,
    KeyboardShortcuts,
    UsageBilling,
    Account,
    Plugins,
    Browser,
    ComputerUse,
    Hooks,
    Connections,
    Git,
    Environments,
    Worktrees,
    ArchivedChats,
}

impl SettingsSection {
    pub fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::LinuxDesktop => "Linux desktop",
            Self::Import => "Import",
            Self::Profile => "Profile",
            Self::Appearance => "Appearance",
            Self::Voice => "Voice",
            Self::Configuration => "Configuration",
            Self::Personalization => "Personalization",
            Self::Pets => "Pets",
            Self::KeyboardShortcuts => "Keyboard shortcuts",
            Self::UsageBilling => "Usage & billing",
            Self::Account => "Account",
            Self::Plugins => "Plugins",
            Self::Browser => "Browser",
            Self::ComputerUse => "Computer use",
            Self::Hooks => "Hooks",
            Self::Connections => "Connections",
            Self::Git => "Git",
            Self::Environments => "Environments",
            Self::Worktrees => "Worktrees",
            Self::ArchivedChats => "Archived chats",
        }
    }

    pub fn group(self) -> SettingsNavGroup {
        match self {
            Self::General
            | Self::LinuxDesktop
            | Self::Import
            | Self::Profile
            | Self::Appearance
            | Self::Voice
            | Self::Configuration
            | Self::Personalization
            | Self::Pets
            | Self::KeyboardShortcuts
            | Self::UsageBilling
            | Self::Account => SettingsNavGroup::Personal,
            Self::Plugins | Self::Browser | Self::ComputerUse => SettingsNavGroup::Integrations,
            Self::Hooks | Self::Connections | Self::Git | Self::Environments | Self::Worktrees => {
                SettingsNavGroup::Coding
            }
            Self::ArchivedChats => SettingsNavGroup::Archived,
        }
    }

    pub fn all() -> &'static [SettingsSection] {
        &SETTINGS_SECTIONS
    }

    pub fn matches_query(self, q: &str) -> bool {
        if q.is_empty() {
            return true;
        }
        let q = q.to_ascii_lowercase();
        self.label().to_ascii_lowercase().contains(&q)
            || self.group().label().to_ascii_lowercase().contains(&q)
    }
}

/// Settings nav group headers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsNavGroup {
    Personal,
    Integrations,
    Coding,
    Archived,
}

impl SettingsNavGroup {
    pub fn label(self) -> &'static str {
        match self {
            Self::Personal => "Personal",
            Self::Integrations => "Integrations",
            Self::Coding => "Coding",
            Self::Archived => "Archived",
        }
    }

    pub fn all() -> &'static [SettingsNavGroup] {
        &[
            Self::Personal,
            Self::Integrations,
            Self::Coding,
            Self::Archived,
        ]
    }
}

/// Optional start mode for surface capture / demos (`MITSURO_START_MODE`).
///
/// Accepts: chat, codex, work, atlas, terminal, files, computer, plugins|extensions,
/// settings, pull-requests|prs|pr, sites, scheduled, thread-open|thread (Codex + first
/// non-empty demo thread).
fn parse_start_mode() -> Option<ProductMode> {
    let raw = std::env::var("MITSURO_START_MODE").ok()?;
    let key = raw.trim().to_ascii_lowercase().replace('_', "-");
    Some(match key.as_str() {
        "chat" | "chatgpt" => ProductMode::Chat,
        "codex" => ProductMode::Codex,
        "work" => ProductMode::Work,
        "atlas" | "browser" => ProductMode::Atlas,
        "terminal" => ProductMode::Terminal,
        "files" => ProductMode::Files,
        "computer" => ProductMode::Computer,
        "plugins" | "extensions" => ProductMode::Extensions,
        "settings" => ProductMode::Settings,
        "pull-requests" | "prs" | "pr" | "pullrequests" => ProductMode::PullRequests,
        "sites" => ProductMode::Sites,
        "scheduled" | "schedule" => ProductMode::Scheduled,
        // Open-thread capture: land on Codex surface; thread id applied after seed.
        "thread-open" | "thread" | "open-thread" => ProductMode::Codex,
        _ => return None,
    })
}

/// Optional thread id/title to select after bootstrap (`MITSURO_START_THREAD`).
///
/// Accepts:
/// - exact server thread id
/// - case-insensitive title substring (e.g. `Core Fix`)
/// - `@first` — first non-archived server thread after list load
///
/// When `MITSURO_START_MODE` is `thread-open` and no thread env is set, defaults
/// to `@first` (live) rather than a fixture demo id — selection is applied
/// **after** `thread/list` fills Recents (see [`MitsuroApp::apply_pending_start_thread`]).
fn parse_start_thread(mode_raw: Option<&str>) -> Option<String> {
    if let Ok(id) = std::env::var("MITSURO_START_THREAD") {
        let id = id.trim();
        if !id.is_empty() {
            return Some(id.to_string());
        }
    }
    let key = mode_raw
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
        .replace('_', "-");
    if matches!(key.as_str(), "thread-open" | "thread" | "open-thread") {
        return Some("@first".into());
    }
    None
}

const SETTINGS_SECTIONS: [SettingsSection; 21] = [
    SettingsSection::General,
    SettingsSection::LinuxDesktop,
    SettingsSection::Import,
    SettingsSection::Profile,
    SettingsSection::Appearance,
    SettingsSection::Voice,
    SettingsSection::Configuration,
    SettingsSection::Personalization,
    SettingsSection::Pets,
    SettingsSection::KeyboardShortcuts,
    SettingsSection::UsageBilling,
    SettingsSection::Account,
    SettingsSection::Plugins,
    SettingsSection::Browser,
    SettingsSection::ComputerUse,
    SettingsSection::Hooks,
    SettingsSection::Connections,
    SettingsSection::Git,
    SettingsSection::Environments,
    SettingsSection::Worktrees,
    SettingsSection::ArchivedChats,
];

fn default_settings_toggles() -> std::collections::HashMap<String, bool> {
    let mut m = std::collections::HashMap::new();
    // General
    m.insert("default_permissions".into(), true);
    m.insert("full_access".into(), true);
    m.insert("bottom_panel".into(), true);
    m.insert("prevent_sleep".into(), false);
    m.insert("suggested_prompts".into(), true);
    m.insert("show_context_usage".into(), false);
    m.insert("popout_standalone".into(), false);
    // Linux desktop
    m.insert("compact_prompt".into(), true);
    m.insert("system_tray".into(), true);
    m.insert("warm_start".into(), true);
    m.insert("install_updates_on_close".into(), false);
    // Appearance
    m.insert("use_system_theme".into(), false);
    m.insert("reduce_motion".into(), false);
    m.insert("high_contrast".into(), false);
    m.insert("translucent_sidebar".into(), true);
    // Voice
    m.insert("voice_auto_send".into(), true);
    m.insert("voice_noise_suppression".into(), true);
    m.insert("voice_push_to_talk".into(), true);
    m.insert("voice_auto_start".into(), false);
    // Pets / personalization / import
    m.insert("pets_enabled".into(), false);
    m.insert("pets_react".into(), true);
    m.insert("remember_project_prefs".into(), true);
    m.insert("enable_local_memories".into(), true);
    m.insert("memory_from_tools".into(), false);
    m.insert("import_archived".into(), false);
    // Plugins / browser / computer / hooks / git / worktrees
    m.insert("plugins_auto_update".into(), true);
    m.insert("browser_persist_cookies".into(), true);
    m.insert("computer_use_enabled".into(), true);
    m.insert("computer_confirm_actions".into(), true);
    m.insert("computer_network".into(), false);
    m.insert("hooks_enabled".into(), false);
    m.insert("auto_reconnect".into(), true);
    m.insert("git_auto_stage".into(), false);
    m.insert("git_sign_commits".into(), false);
    m.insert("git_pr_helper".into(), true);
    m.insert("git_force_push".into(), false);
    m.insert("worktrees_enabled".into(), true);
    m.insert("worktrees_auto_prune".into(), true);
    m.insert("archived_show_in_recents".into(), false);
    m.insert("prefer_agents_md".into(), true);
    m.insert("env_prefer_local".into(), true);
    m.insert("emacs_bindings".into(), false);
    m.insert("profile_show_name".into(), true);
    m
}

fn default_settings_choices() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    m.insert("file_open_dest".into(), "Zed".into());
    m.insert("language".into(), "Auto detect".into());
    m.insert("terminal_location".into(), "Bottom".into());
    m.insert("speed".into(), "Fast".into());
    m.insert("send_shortcut".into(), "Enter".into());
    m.insert("follow_up".into(), "Queue".into());
    m.insert("theme".into(), "Dark".into());
    m.insert("density".into(), "Comfortable".into());
    m.insert("font_size".into(), "Default".into());
    m.insert("font_scale".into(), "100%".into());
    m.insert("code_font".into(), "JetBrains Mono".into());
    m.insert("code_font_size".into(), "13px".into());
    m.insert("accent_color".into(), "Blue".into());
    m.insert("voice_input".into(), "System default".into());
    // Reverse voice names (settings.general.realtimeVoice.voice.*) — Sol default.
    m.insert("voice_output".into(), "Sol".into());
    // Reverse personality: Friendly | Pragmatic.
    m.insert("personality".into(), "Friendly".into());
    m.insert("reduce_motion".into(), "Off".into());
    m.insert("diff_markers".into(), "Color".into());
    m.insert("contrast".into(), "Default".into());
    m.insert("ui_font".into(), "Inter".into());
    m.insert("ui_font_size".into(), "14px".into());
    m.insert("review_delivery".into(), "Inline".into());
    m.insert("pet_kind".into(), "Fox".into());
    m.insert("pet_size".into(), "Medium".into());
    m.insert("pet_position".into(), "Bottom-right".into());
    m.insert("browser_engine".into(), "System".into());
    m.insert("default_browser".into(), "System default".into());
    m.insert("browser_approval".into(), "Always ask".into());
    m.insert("computer_env".into(), "Local".into());
    m.insert("git_default_branch".into(), "main".into());
    m.insert("git_pr_merge".into(), "Squash".into());
    m.insert("worktree_strategy".into(), "Git worktree".into());
    m.insert("worktree_keep_count".into(), "5".into());
    m
}

/// Initials from a display name (`"Jacob Burgess"` → `"JB"`).
pub fn profile_initials_from_name(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [] => "?".into(),
        [one] => one
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into()),
        [a, b, ..] => {
            let mut s = String::new();
            if let Some(c) = a.chars().next() {
                s.extend(c.to_uppercase());
            }
            if let Some(c) = b.chars().next() {
                s.extend(c.to_uppercase());
            }
            s
        }
    }
}

/// Optional settings left-nav section for capture (`MITSURO_SETTINGS_SECTION`).
///
/// Accepts labels like `appearance`, `voice`, `pets`, `keyboard`, `usage`, etc.
fn parse_settings_section() -> Option<SettingsSection> {
    let raw = std::env::var("MITSURO_SETTINGS_SECTION").ok()?;
    let key = raw.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    Some(match key.as_str() {
        "general" => SettingsSection::General,
        "linux" | "linux-desktop" => SettingsSection::LinuxDesktop,
        "import" => SettingsSection::Import,
        "profile" => SettingsSection::Profile,
        "appearance" => SettingsSection::Appearance,
        "voice" => SettingsSection::Voice,
        "configuration" | "config" => SettingsSection::Configuration,
        "personalization" => SettingsSection::Personalization,
        "pets" | "pet" => SettingsSection::Pets,
        "keyboard" | "keyboard-shortcuts" | "shortcuts" => SettingsSection::KeyboardShortcuts,
        "usage" | "usage-billing" | "billing" => SettingsSection::UsageBilling,
        "account" => SettingsSection::Account,
        "plugins" => SettingsSection::Plugins,
        "browser" => SettingsSection::Browser,
        "computer" | "computer-use" => SettingsSection::ComputerUse,
        "hooks" => SettingsSection::Hooks,
        "connections" | "mcp" => SettingsSection::Connections,
        "git" => SettingsSection::Git,
        "environments" | "envs" => SettingsSection::Environments,
        "worktrees" | "worktree" => SettingsSection::Worktrees,
        "archived" | "archived-chats" => SettingsSection::ArchivedChats,
        _ => return None,
    })
}

impl ProductMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            Self::Work => "Work",
            Self::Codex => "Codex",
            Self::Atlas => "Atlas",
            Self::Terminal => "Terminal",
            Self::Files => "Files",
            Self::Computer => "Computer",
            Self::Extensions => "Plugins",
            Self::Settings => "Settings",
            Self::PullRequests => "Pull requests",
            Self::Sites => "Sites",
            Self::Scheduled => "Scheduled",
        }
    }

    /// Window title bar text, e.g. `"Mitsuro — Codex"`.
    pub fn window_title(self) -> String {
        format!("Mitsuro — {}", self.label())
    }

    /// Whether this mode shows the home thread sidebar (bar left nav).
    pub fn shows_thread_sidebar(self) -> bool {
        matches!(
            self,
            Self::Chat
                | Self::Codex
                | Self::PullRequests
                | Self::Sites
                | Self::Scheduled
                | Self::Extensions
        )
    }

    /// Thin icon activity rail — only for advanced modes not in bar home nav.
    /// Settings uses its own two-column tree (no rail chrome).
    pub fn shows_activity_rail(self) -> bool {
        matches!(
            self,
            Self::Work | Self::Atlas | Self::Terminal | Self::Files | Self::Computer
        )
    }

    /// Mode switcher pill label (Chat vs Codex surfaces).
    pub fn mode_switcher_label(self) -> &'static str {
        match self {
            Self::Chat => "Chat",
            _ => "Codex",
        }
    }
}

/// Terminal / process panel session status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TerminalSessionStatus {
    #[default]
    Idle,
    Running,
    Exited,
    Error,
}

/// Snapshot for the Files panel (`fs/*` + fuzzy search).
#[derive(Clone, Debug)]
pub struct FilesSession {
    pub cwd: SharedString,
    pub entries: Vec<FsReadDirectoryEntry>,
    pub selected_path: Option<String>,
    pub preview: SharedString,
    pub preview_error: Option<String>,
    pub search_query: String,
    pub fuzzy_results: Vec<FuzzyFileSearchResult>,
    pub backend_label: SharedString,
}

impl FilesSession {
    fn new(backend_label: impl Into<SharedString>) -> Self {
        Self {
            cwd: String::new().into(),
            entries: Vec::new(),
            selected_path: None,
            preview: SharedString::from(""),
            preview_error: None,
            search_query: String::new(),
            fuzzy_results: Vec::new(),
            backend_label: backend_label.into(),
        }
    }
}

/// Snapshot for the Terminal panel (process/spawn UI).
#[derive(Clone, Debug)]
pub struct TerminalSession {
    pub process_handle: Option<String>,
    pub output: SharedString,
    pub running: bool,
    pub status: TerminalSessionStatus,
    pub exit_code: Option<i32>,
    pub backend_label: SharedString,
}

impl TerminalSession {
    fn idle(backend_label: impl Into<SharedString>) -> Self {
        Self {
            process_handle: None,
            output: SharedString::from(""),
            running: false,
            status: TerminalSessionStatus::Idle,
            exit_code: None,
            backend_label: backend_label.into(),
        }
    }

    pub fn status_label(&self) -> &'static str {
        match self.status {
            TerminalSessionStatus::Idle => "Idle",
            TerminalSessionStatus::Running => "Running",
            TerminalSessionStatus::Exited => "Exited",
            TerminalSessionStatus::Error => "Error",
        }
    }
}

/// Atlas / agent browser-use session state (fixture until wry host lands).
///
/// Variants beyond `NoNativeHost` / `Idle` are reserved for the native host
/// and agent-driving paths; matched in `browser_panel` status chrome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[allow(dead_code)] // Connecting/Ready/AgentDriving/Error set by future host hooks
pub enum BrowserSessionStatus {
    /// Panel open, no active agent drive.
    #[default]
    Idle,
    Connecting,
    Ready,
    /// Agent tools are navigating / interacting.
    AgentDriving,
    Error,
    /// Explicit: no native WebView linked (default for P4).
    NoNativeHost,
}

/// Snapshot of Atlas browser session for the panel (derived from [`DesktopBrowserHost`]).
#[derive(Clone, Debug)]
pub struct BrowserSession {
    pub url: SharedString,
    pub title: SharedString,
    pub status: BrowserSessionStatus,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    /// Page-state summary from the local bridge. Never remote page content.
    pub page_body: SharedString,
    /// Host backend chip label.
    pub host_kind: SharedString,
    /// Optional WebKit version when wry is linked.
    pub engine_version: Option<SharedString>,
    /// Bridge / attach detail (external, sibling, embed probe).
    pub bridge_detail: SharedString,
    /// Short bridge mode label for chips.
    pub bridge_mode: SharedString,
}

impl BrowserSession {
    fn from_host(
        host: &DesktopBrowserHost,
        bridge_detail: SharedString,
        bridge_mode: SharedString,
        host_kind_override: Option<SharedString>,
    ) -> Self {
        Self {
            url: host.url().to_string().into(),
            title: host.title().to_string().into(),
            status: host.status(),
            can_go_back: host.can_go_back(),
            can_go_forward: host.can_go_forward(),
            page_body: host.page_body().to_string().into(),
            host_kind: host_kind_override.unwrap_or_else(|| SharedString::from(host.host_kind())),
            engine_version: host
                .engine_version()
                .map(|v| SharedString::from(v.to_string())),
            bridge_detail,
            bridge_mode,
        }
    }
}

#[derive(Clone, Debug)]
pub enum UiConnection {
    /// Offline chrome with static demo data (reserved / legacy chip).
    #[allow(dead_code)]
    Demo,
    /// Explicit fixture backend (sample-turn.jsonl).
    Fixture,
    Connecting,
    Ready {
        detail: String,
        /// True when account/read reports a non-null account.
        has_auth: bool,
    },
    #[allow(dead_code)]
    Error {
        message: String,
    },
}

impl UiConnection {
    pub fn detail(&self) -> Option<&str> {
        match self {
            Self::Ready { detail, .. } => Some(detail.as_str()),
            Self::Error { message } => Some(message.as_str()),
            Self::Demo | Self::Fixture | Self::Connecting => None,
        }
    }

    pub fn chip_label(&self) -> &'static str {
        match self {
            // Keep offline backends out of "fixture"/"demo" primary chrome.
            Self::Demo => "Offline",
            Self::Fixture => "Offline",
            Self::Connecting => "Connecting",
            Self::Ready { .. } => "Ready",
            Self::Error { .. } => "Error",
        }
    }
}

/// Provenance for data rendered by a product surface.
///
/// Live modes must never transition to `Fixture`; that state is reserved for an
/// explicitly selected fixture backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceDataState {
    Loading,
    Live,
    Fixture,
    Unsupported,
    Error,
}

impl SurfaceDataState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Loading => "Loading",
            Self::Live => "Live",
            Self::Fixture => "Fixture",
            Self::Unsupported => "Unsupported",
            Self::Error => "Error",
        }
    }
}

/// How Send should produce an assistant reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SendMode {
    /// Replay `fixtures/sample-turn.jsonl` only when explicitly selected for development.
    Fixture,
    /// Live product turn through the selected Ready/authenticated backend.
    Live,
    /// The selected backend cannot currently accept a turn.
    Unavailable,
}

fn decide_send_mode(
    connection: &UiConnection,
    backend_kind: Option<BackendKind>,
    backend_present: bool,
    force_fixture: bool,
) -> SendMode {
    if force_fixture || fixture_records_allowed(connection, backend_kind) {
        return SendMode::Fixture;
    }
    if backend_kind == Some(BackendKind::Fixture) {
        return SendMode::Unavailable;
    }
    match connection {
        UiConnection::Ready { has_auth: true, .. } if backend_present => SendMode::Live,
        _ => SendMode::Unavailable,
    }
}

fn fixture_records_allowed(connection: &UiConnection, backend_kind: Option<BackendKind>) -> bool {
    matches!(connection, UiConnection::Fixture) && backend_kind == Some(BackendKind::Fixture)
}

/// Account / usage surface for Settings (offline fixture demo + live probe).
#[derive(Clone, Debug)]
pub struct AccountSession {
    /// Whether `account/read` reports a usable account.
    pub signed_in: bool,
    /// Masked email or account type label.
    pub email_display: Option<String>,
    /// Plan tier label (e.g. Plus).
    pub plan_label: Option<String>,
    /// Token usage snapshot (fixture demo numbers offline).
    pub usage: GetAccountTokenUsageResponse,
    /// Rate-limit windows for usage bars.
    pub rate_limits: GetAccountRateLimitsResponse,
    /// Current OAuth/device-code status shown in Settings.
    pub login_detail: Option<String>,
    /// Server identity for an asynchronous login that may be canceled.
    pub pending_login_id: Option<String>,
    /// Login URL retained so the user can reopen the browser flow.
    pub pending_login_url: Option<String>,
    /// Backend source label for status chips.
    pub source: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerAttachment {
    pub path: String,
    pub name: String,
    pub kind: ComposerAttachmentKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposerAttachmentKind {
    Image,
    Audio,
    Skill,
    Mention,
}

impl AccountSession {
    /// Seeded fixture demo profile (signed-in Pro + usage bars + Jacob Burgess).
    pub fn fixture_demo() -> Self {
        let account_resp = fixture_demo_account_response();
        let (email, plan) = match account_resp.account.as_ref() {
            Some(Account::Chatgpt { email, plan_type }) => (
                email
                    .as_ref()
                    .map(|e| mitsuro_desktop_backend::mask_email(e))
                    .or_else(|| Some(mitsuro_desktop_backend::FIXTURE_DEMO_EMAIL_MASKED.into())),
                Some(plan_type.label().to_string()),
            ),
            Some(a) => (
                a.email_display(),
                a.plan_type().map(|p| p.label().to_string()),
            ),
            None => (None, None),
        };
        Self {
            signed_in: account_resp.has_account(),
            email_display: email,
            plan_label: plan,
            usage: fixture_demo_usage(),
            rate_limits: fixture_demo_rate_limits(),
            login_detail: None,
            pending_login_id: None,
            pending_login_url: None,
            source: "fixture",
        }
    }

    pub fn empty(source: &'static str) -> Self {
        Self {
            signed_in: false,
            email_display: None,
            plan_label: None,
            usage: GetAccountTokenUsageResponse {
                summary: Default::default(),
                daily_usage_buckets: None,
            },
            rate_limits: GetAccountRateLimitsResponse {
                rate_limits: Default::default(),
            },
            login_detail: None,
            pending_login_id: None,
            pending_login_url: None,
            source,
        }
    }

    pub fn primary_used_percent(&self) -> i32 {
        self.rate_limits
            .rate_limits
            .primary
            .as_ref()
            .map(|w| w.used_percent)
            .unwrap_or(0)
    }

    pub fn secondary_used_percent(&self) -> i32 {
        self.rate_limits
            .rate_limits
            .secondary
            .as_ref()
            .map(|w| w.used_percent)
            .unwrap_or(0)
    }

    /// True when primary window is exhausted and no credit balance remains.
    pub fn is_rate_limited_out(&self) -> bool {
        let primary_full = self.primary_used_percent() >= 100;
        let credits = self
            .rate_limits
            .rate_limits
            .credits
            .as_ref()
            .map(|c| c.has_credits || c.unlimited)
            .unwrap_or(false);
        primary_full && !credits
    }

    pub fn lifetime_tokens(&self) -> i64 {
        self.usage.summary.lifetime_tokens.unwrap_or(0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AccountLoginCompletion {
    success: bool,
    login_id: Option<String>,
    error: Option<String>,
}

fn account_login_completion(event: &LifecycleNotification) -> Option<AccountLoginCompletion> {
    if event.method != "account/login/completed" {
        return None;
    }
    let params = event.params.as_ref()?;
    Some(AccountLoginCompletion {
        success: params
            .get("success")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        login_id: params
            .get("loginId")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        error: params
            .get("error")
            .and_then(serde_json::Value::as_str)
            .filter(|message| !message.is_empty())
            .map(str::to_owned),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RealtimeVoicePhase {
    Starting,
    Active,
    Stopping,
}

struct RealtimePlayback {
    sample_rate: u32,
    channels: u16,
    audio_tx: mpsc::Sender<Vec<u8>>,
}

struct RealtimeVoiceRuntime {
    session_id: BackendSessionId,
    ui_thread_id: String,
    capture_stop: Arc<AtomicBool>,
    phase: RealtimeVoicePhase,
    playback: Option<RealtimePlayback>,
}

pub struct MitsuroApp {
    focus_handle: FocusHandle,
    connection: UiConnection,
    threads: Vec<DemoThread>,
    /// Canonical reconnect/live delegation state retained independently from
    /// ephemeral transcript bubbles and keyed by backend-qualified thread id.
    delegations: std::collections::HashMap<String, SessionDelegationProjection>,
    /// Per-thread transcript window. History is revealed deliberately so long
    /// sessions never force GPUI to lay out the entire conversation at once.
    transcript_visible_limits: std::collections::HashMap<String, usize>,
    /// Explicit user expansion for long transcript messages. Keys include the
    /// backend-qualified UI thread id and stable item id (or message index).
    expanded_transcript_messages: std::collections::HashSet<String>,
    transcript_scroll_handle: ScrollHandle,
    selected_thread: Option<String>,
    status_line: SharedString,
    /// Active product shell mode (rail selection).
    active_mode: ProductMode,
    /// Selected Settings left-nav section (only meaningful in Settings mode).
    settings_section: SettingsSection,
    /// Mode to restore when leaving Settings via "Back to app".
    settings_return_mode: ProductMode,
    /// Client-side filter for Settings left-nav labels.
    settings_search_query: String,
    /// Settings search input entity.
    settings_search_input: Entity<InputState>,
    /// Extensions catalog search shared by Plugins and Skills tabs.
    plugins_search_input: Entity<InputState>,
    /// Marketplace sections explicitly expanded by the user.
    expanded_plugin_sections: std::collections::HashSet<String>,
    /// Desktop-local toggles. Persistence does not imply a server configuration write.
    settings_toggles: std::collections::HashMap<String, bool>,
    /// Desktop-local string choices (e.g. "Bottom"/"Right", "Fast").
    settings_choices: std::collections::HashMap<String, String>,
    /// Last selected Chat-surface thread (restored when re-entering Chat).
    selected_chat_thread: Option<String>,
    /// Last selected Codex-surface thread (restored when re-entering Codex).
    selected_codex_thread: Option<String>,
    /// Work-mode goals (local list + optional `thread/goal/*` for linked threads).
    goals: Vec<DemoGoal>,
    selected_goal: Option<String>,
    /// True when Work rows are a read-only projection of Mitsuro Hive runs.
    goals_are_live_hive: bool,
    /// Native Hive status retained separately from its Work-row projection.
    hive_snapshot: Option<ProductHiveSnapshot>,
    /// Models from `model/list` (or fixture demo catalog).
    models: Vec<ModelInfo>,
    /// Selected model id (matches [`ModelInfo::id`]).
    selected_model_id: Option<String>,
    /// Backend/model-scoped effort chosen from the live model capability list.
    selected_reasoning_effort: Option<String>,
    /// Live Codex realtime voice catalog. Mitsuro deliberately leaves this empty.
    realtime_voices: Option<RealtimeVoicesList>,
    realtime_voices_state: SurfaceDataState,
    realtime_voice_runtime: Option<RealtimeVoiceRuntime>,
    realtime_voice_generation: u64,
    /// Whether the selected model's advertised accelerated response mode is active.
    selected_fast_mode: bool,
    /// Short config snippet from `config/read` (Settings).
    config_snippet: SharedString,
    /// Skills from `skills/list` (or fixture demo).
    skills: Vec<SkillMetadata>,
    /// MCP servers from `mcpServerStatus/list` (or fixture demo).
    mcp_servers: Vec<McpServerStatus>,
    pending_mcp_oauth: std::collections::HashSet<String>,
    /// Plugins from `plugin/list` (flattened marketplace entries).
    plugins: Vec<PluginSummary>,
    extensions_state: SurfaceDataState,
    /// Plugin id currently being installed or removed through Codex app-server.
    plugin_mutation_in_progress: Option<String>,
    /// Environments catalog (fixture demo; no protocol list method).
    environments: Vec<EnvironmentSummary>,
    environments_state: SurfaceDataState,
    environment_id_input: Entity<InputState>,
    environment_url_input: Entity<InputState>,
    environment_add_in_progress: bool,
    /// Selected environment id for status/info detail.
    selected_environment_id: Option<String>,
    /// Last `environment/status` response for the selection.
    environment_status_detail: Option<EnvironmentStatusResponse>,
    /// Last `environment/info` response for the selection.
    environment_info_detail: Option<EnvironmentInfoResponse>,
    /// Collaboration mode presets (`collaborationMode/list`).
    collaboration_modes: Vec<CollaborationModeMask>,
    /// False is Codex Default / Mitsuro Build; true is Plan for either backend.
    composer_plan_mode: bool,
    /// Account / usage session for Settings Account section.
    account: AccountSession,
    account_state: SurfaceDataState,
    composer_input: Entity<InputState>,
    composer_attachments: Vec<ComposerAttachment>,
    composer_add_menu_open: bool,
    /// Search field and visibility for the real model catalog picker.
    composer_model_search_input: Entity<InputState>,
    composer_model_menu_open: bool,
    /// Explicit reasoning-effort picker; never cycles a hidden choice on click.
    composer_reasoning_menu_open: bool,
    /// Workspace picked for the next optimistic draft before it has a thread id.
    composer_default_workspace_dir: Option<String>,
    /// Backend-specific access preset picked before an optimistic draft exists.
    composer_default_access_mode: Option<ProductAccessMode>,
    /// Explicit per-thread access overrides; absent means preserve backend defaults.
    composer_access_modes: std::collections::HashMap<String, ProductAccessMode>,
    composer_access_menu_open: bool,
    search_input: Entity<InputState>,
    search_query: String,
    backend: Option<Arc<DesktopBackend>>,
    /// Rejects stale async bootstrap results after an in-app backend switch.
    backend_generation: u64,
    preferences: DesktopPreferences,
    fixture: Option<Arc<FixtureBackend>>,
    turn_in_progress: bool,
    /// Monotonic identity for the active turn. Incrementing invalidates late
    /// producer events after Stop, backend replacement, or failed promotion.
    turn_generation: u64,
    /// UI thread id that owns the active turn, independent from sidebar selection.
    active_turn_thread_id: Option<String>,
    /// Active turn id for `turn/interrupt` (from TurnStarted).
    active_turn_id: Option<String>,
    /// Cancel flag for fixture stream replay (Stop → set true).
    turn_cancel: Option<Arc<AtomicBool>>,
    /// When true, sidebar includes archived threads; default hides them.
    show_archived: bool,
    #[allow(dead_code)]
    samples_loaded: bool,
    /// Demo/sample threads loaded into sidebar Recents.
    /// Mode switcher dropdown (Chat / Codex) open state.
    mode_menu_open: bool,
    /// Thread title overflow menu (Archive / Fork / Delete) open state.
    thread_menu_open: bool,
    /// Dismissible home promo card: usage.
    dismiss_usage_card: bool,
    /// Active server approval request (exec / patch) awaiting user decision.
    pending_approval: Option<PendingApproval>,
    /// Active structured request_user_input interaction.
    pending_user_input: Option<PendingUserInput>,
    user_input_question_index: usize,
    user_input_answers: BTreeMap<String, Vec<String>>,
    server_request_input: Entity<InputState>,
    server_request_secret_input: Entity<InputState>,
    /// Active MCP elicitation. Standard form fields are answered sequentially.
    pending_mcp_elicitation: Option<PendingMcpElicitation>,
    mcp_form_field_index: usize,
    mcp_form_values: BTreeMap<String, serde_json::Value>,
    /// Remaining fixture events after stream paused on an approval.
    fixture_resume: Option<(String, Vec<TurnStreamEvent>)>,
    /// Live progressive turn: UI submits choice here; runner writes respond_approval.
    live_approval_bridge: Option<Arc<LiveApprovalBridge>>,
    /// Atlas browser host (URL history + external/native bridge state).
    browser_host: DesktopBrowserHost,
    /// Snapshot synced from `browser_host` for the panel.
    browser: BrowserSession,
    /// Editable Atlas URL bar.
    browser_url_input: Entity<InputState>,
    /// Best-effort native / external bridge (wry child or xdg-open). UI-thread only.
    #[cfg(feature = "browser-native")]
    native_host: NativeWebViewHost,
    /// Terminal panel session (process/spawn).
    terminal: TerminalSession,
    /// Command line for process/spawn.
    terminal_cmd_input: Entity<InputState>,
    /// Stdin line for process/writeStdin.
    terminal_stdin_input: Entity<InputState>,
    /// Monotonic handle counter for client-supplied processHandle.
    terminal_handle_seq: u64,
    /// Mitsuro background-process catalog. Interactive process semantics remain disabled.
    background_processes: Vec<ProductProcess>,
    /// Files panel session (fs + fuzzy).
    files: FilesSession,
    /// Path bar input for `fs/readDirectory`.
    files_path_input: Entity<InputState>,
    /// Fuzzy search query input.
    files_search_input: Entity<InputState>,
    /// Scheduled: show explicit-fixture task rows (vs suggestions only).
    scheduled_show_tasks: bool,
    /// Scheduled fixture row enabled toggles.
    scheduled_enabled: Vec<bool>,
    /// Some, including an empty vec, means the Mitsuro Hive schedule API is live.
    scheduled_tasks: Option<Vec<ProductSchedule>>,
    /// Plugins marketplace category filter (Public / Personal / MCP).
    plugins_filter: PluginsFilter,
    /// Plugins surface top tab (Plugins | Skills).
    plugins_surface_tab: PluginsSurfaceTab,
    /// Deferred open-thread after async bootstrap (`MITSURO_START_THREAD` / thread-open).
    /// Applied once Recents are filled from app-server (or fixture).
    pending_start_thread: Option<String>,
}

impl MitsuroApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let preferences = DesktopPreferences::load_default().unwrap_or_else(|error| {
            eprintln!("[mitsuro] desktop preference load failed: {error}");
            DesktopPreferences::default()
        });
        let mut settings_toggles = default_settings_toggles();
        settings_toggles.extend(preferences.settings_toggles.clone());
        let mut settings_choices = default_settings_choices();
        settings_choices.extend(preferences.settings_choices.clone());
        let composer_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Do anything")
                .multi_line(true)
        });
        // Re-render composer trailing control (voice disc ↔ send) as draft changes.
        cx.subscribe_in(
            &composer_input,
            window,
            |app, _input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let _ = app;
                    cx.notify();
                }
            },
        )
        .detach();
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let composer_model_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search models…"));
        cx.subscribe_in(
            &composer_model_search_input,
            window,
            |app, _input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let _ = app;
                    cx.notify();
                }
            },
        )
        .detach();
        let browser_host = create_default_host();
        let initial_url = browser_host.url().to_string();
        let browser_url_input = cx.new(|cx| {
            let initial_url = if initial_url == "about:blank" {
                String::new()
            } else {
                initial_url
            };
            InputState::new(window, cx)
                .placeholder("Enter URL")
                .default_value(initial_url)
        });
        // Enter follows the visible Atlas action: record the URL, then open the real
        // page through the system-browser bridge.
        cx.subscribe_in(
            &browser_url_input,
            window,
            |app, _input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    app.browser_navigate(window, cx);
                    app.browser_open_external(cx);
                }
            },
        )
        .detach();

        let terminal_cmd_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("bash -lc 'echo hello from mitsuro'")
                .default_value("echo hello from mitsuro")
        });
        let terminal_stdin_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("stdin (writeStdin)…"));
        cx.subscribe_in(
            &terminal_cmd_input,
            window,
            |app, _input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    app.terminal_spawn(window, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &terminal_stdin_input,
            window,
            |app, _input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    app.terminal_write_stdin(window, cx);
                }
            },
        )
        .detach();

        let files_path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("/workspace")
                .default_value(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
        });
        let files_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Fuzzy search file names…"));
        let settings_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search settings…"));
        let plugins_search_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search plugins and skills…"));
        cx.subscribe_in(
            &plugins_search_input,
            window,
            |app, _input, event: &InputEvent, _window, cx| {
                if matches!(event, InputEvent::Change) {
                    let _ = app;
                    cx.notify();
                }
            },
        )
        .detach();
        let environment_id_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Environment id"));
        let environment_url_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("wss://exec-server.example.com"));
        let server_request_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Type an answer…"));
        let server_request_secret_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Type a private answer…")
                .masked(true)
        });
        cx.subscribe_in(
            &files_path_input,
            window,
            |app, _input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    app.files_navigate_path_bar(window, cx);
                }
            },
        )
        .detach();
        cx.subscribe_in(
            &files_search_input,
            window,
            |app, _input, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    app.files_run_fuzzy(window, cx);
                }
            },
        )
        .detach();

        let fixture = Arc::new(FixtureBackend::new().with_stream_delay(Duration::from_millis(35)));

        #[cfg(feature = "browser-native")]
        let native_host = NativeWebViewHost::new();
        #[cfg(feature = "browser-native")]
        let (bridge_mode, bridge_detail, host_kind_override) = {
            let mode = SharedString::from(native_host.bridge_mode().label());
            let detail = SharedString::from(native_host.report().detail.clone());
            let kind = SharedString::from(native_host.host_kind_label());
            (mode, detail, Some(kind))
        };
        #[cfg(not(feature = "browser-native"))]
        let (bridge_mode, bridge_detail, host_kind_override) = (
            SharedString::from("External"),
            SharedString::from("System browser owns page content; Mitsuro keeps URL history only"),
            None,
        );

        let browser = BrowserSession::from_host(
            &browser_host,
            bridge_detail,
            bridge_mode,
            host_kind_override,
        );

        let mut app = Self {
            focus_handle: cx.focus_handle(),
            connection: UiConnection::Connecting,
            threads: Vec::new(),
            delegations: std::collections::HashMap::new(),
            transcript_visible_limits: std::collections::HashMap::new(),
            expanded_transcript_messages: std::collections::HashSet::new(),
            transcript_scroll_handle: ScrollHandle::new(),
            selected_thread: None,
            selected_chat_thread: None,
            selected_codex_thread: None,
            status_line: SharedString::from(""),
            samples_loaded: false,
            mode_menu_open: false,
            thread_menu_open: false,
            dismiss_usage_card: false,
            pending_user_input: None,
            user_input_question_index: 0,
            user_input_answers: BTreeMap::new(),
            server_request_input,
            server_request_secret_input,
            pending_mcp_elicitation: None,
            mcp_form_field_index: 0,
            mcp_form_values: BTreeMap::new(),
            active_mode: parse_start_mode().unwrap_or(ProductMode::Codex),
            settings_section: SettingsSection::General,
            settings_return_mode: ProductMode::Codex,
            settings_search_query: String::new(),
            settings_search_input,
            plugins_search_input,
            expanded_plugin_sections: std::collections::HashSet::new(),
            settings_toggles,
            settings_choices,
            goals: Vec::new(),
            selected_goal: None,
            goals_are_live_hive: false,
            hive_snapshot: None,
            models: Vec::new(),
            selected_model_id: None,
            selected_reasoning_effort: None,
            realtime_voices: None,
            realtime_voices_state: SurfaceDataState::Loading,
            realtime_voice_runtime: None,
            realtime_voice_generation: 0,
            selected_fast_mode: false,
            config_snippet: SharedString::from(""),
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            pending_mcp_oauth: std::collections::HashSet::new(),
            plugins: Vec::new(),
            extensions_state: SurfaceDataState::Loading,
            plugin_mutation_in_progress: None,
            environments: Vec::new(),
            environments_state: SurfaceDataState::Loading,
            environment_id_input,
            environment_url_input,
            environment_add_in_progress: false,
            selected_environment_id: None,
            environment_status_detail: None,
            environment_info_detail: None,
            collaboration_modes: Vec::new(),
            composer_plan_mode: false,
            account: AccountSession::empty("loading"),
            account_state: SurfaceDataState::Loading,
            composer_input,
            composer_attachments: Vec::new(),
            composer_add_menu_open: false,
            composer_model_search_input,
            composer_model_menu_open: false,
            composer_reasoning_menu_open: false,
            composer_default_workspace_dir: std::env::current_dir()
                .ok()
                .map(|path| path.display().to_string()),
            composer_default_access_mode: None,
            composer_access_modes: std::collections::HashMap::new(),
            composer_access_menu_open: false,
            search_input,
            search_query: String::new(),
            backend: None,
            backend_generation: 0,
            preferences: preferences.clone(),
            fixture: Some(Arc::clone(&fixture)),
            turn_in_progress: false,
            turn_generation: 0,
            active_turn_thread_id: None,
            active_turn_id: None,
            turn_cancel: None,
            show_archived: false,
            pending_approval: None,
            fixture_resume: None,
            live_approval_bridge: None,
            browser_host,
            browser,
            browser_url_input,
            #[cfg(feature = "browser-native")]
            native_host,
            terminal: TerminalSession::idle("loading"),
            terminal_cmd_input,
            terminal_stdin_input,
            terminal_handle_seq: 1,
            background_processes: Vec::new(),
            files: FilesSession::new("loading"),
            files_path_input,
            files_search_input,
            // Suggestions-first like bar; Create / suggestion pick reveals Your tasks.
            scheduled_show_tasks: false,
            scheduled_enabled: vec![true, true],
            scheduled_tasks: None,
            plugins_filter: PluginsFilter::Public,
            plugins_surface_tab: PluginsSurfaceTab::Plugins,
            pending_start_thread: {
                let mode_raw = std::env::var("MITSURO_START_MODE").ok();
                parse_start_thread(mode_raw.as_deref()).or_else(|| {
                    preferences
                        .selected_session
                        .as_ref()
                        .map(BackendSessionId::qualified)
                })
            },
        };

        // Connect the selected backend without blocking first paint. Fixture data
        // is reachable only through an explicit fixture selection.
        if std::env::var_os("MITSURO_FORCE_FIXTURE").is_some() {
            app.bootstrap_fixture(cx);
        } else if std::env::var_os("MITSURO_SKIP_APPSERVER").is_some() {
            app.connection = UiConnection::Error {
                message: "backend startup disabled by MITSURO_SKIP_APPSERVER".into(),
            };
            app.status_line = "Backend startup disabled; no fixture data loaded.".into();
        } else {
            app.bootstrap_backend(cx);
        }

        // Honor MITSURO_START_MODE for capture / demos (title + status line).
        let start = app.active_mode;
        app.set_mode(start, window, cx);

        // Optional settings section deep-link for surface capture.
        // MITSURO_SETTINGS_SECTION=appearance|voice|pets|… (also forces Settings mode).
        if let Some(section) = parse_settings_section() {
            if !matches!(app.active_mode, ProductMode::Settings) {
                app.set_mode(ProductMode::Settings, window, cx);
            }
            app.settings_section = section;
            app.status_line = format!("Settings · {}", section.label()).into();
            window.set_window_title(&ProductMode::Settings.window_title());
            cx.notify();
        }

        // Eager select only if seed already has the id (fixture demo path).
        // Live server threads arrive async — see apply_pending_start_thread.
        if let Some(thread_id) = app.pending_start_thread.clone() {
            if thread_id != "@first" && app.threads.iter().any(|t| t.summary.id == thread_id) {
                app.pending_start_thread = None;
                app.select_thread(thread_id, cx);
                app.update_composer_placeholder(window, cx);
            }
        }

        app
    }

    pub fn connection(&self) -> &UiConnection {
        &self.connection
    }

    pub fn active_backend_kind(&self) -> Option<BackendKind> {
        self.backend
            .as_ref()
            .map(|backend| backend.kind())
            .or(self.preferences.selected_backend)
    }

    pub fn is_explicit_fixture(&self) -> bool {
        fixture_records_allowed(&self.connection, self.active_backend_kind())
    }

    pub fn extensions_state(&self) -> SurfaceDataState {
        self.extensions_state
    }

    pub fn environments_state(&self) -> SurfaceDataState {
        self.environments_state
    }

    pub fn account_state(&self) -> SurfaceDataState {
        self.account_state
    }

    pub fn work_state(&self) -> SurfaceDataState {
        if self.is_explicit_fixture() {
            return SurfaceDataState::Fixture;
        }
        match (&self.connection, self.active_backend_kind()) {
            (UiConnection::Connecting, _) => SurfaceDataState::Loading,
            (UiConnection::Error { .. }, _) => SurfaceDataState::Error,
            (UiConnection::Ready { .. }, Some(BackendKind::MitsuroHttp)) => SurfaceDataState::Live,
            _ => SurfaceDataState::Unsupported,
        }
    }

    pub fn scheduled_state(&self) -> SurfaceDataState {
        if self.is_explicit_fixture() {
            return SurfaceDataState::Fixture;
        }
        match (&self.connection, self.active_backend_kind()) {
            (UiConnection::Connecting, _) => SurfaceDataState::Loading,
            (UiConnection::Error { .. }, _) => SurfaceDataState::Error,
            (UiConnection::Ready { .. }, Some(BackendKind::MitsuroHttp)) => SurfaceDataState::Live,
            _ => SurfaceDataState::Unsupported,
        }
    }

    pub fn backend_display_name(kind: BackendKind) -> &'static str {
        match kind {
            BackendKind::MitsuroHttp => "Mitsuro server",
            BackendKind::CodexStdio => "ChatGPT / Codex",
            BackendKind::CodexWebSocket => "Codex WebSocket",
            BackendKind::Fixture => "Offline fixtures",
        }
    }

    pub fn switch_backend(&mut self, kind: BackendKind, cx: &mut Context<Self>) {
        if self.active_backend_kind() == Some(kind)
            && matches!(
                self.connection,
                UiConnection::Ready { .. } | UiConnection::Connecting
            )
        {
            self.status_line =
                format!("{} is already selected.", Self::backend_display_name(kind)).into();
            cx.notify();
            return;
        }
        self.preferences.remember_backend(kind);
        self.save_preferences_best_effort();
        self.pending_start_thread = self
            .preferences
            .selected_session
            .as_ref()
            .map(BackendSessionId::qualified);
        let selection = match kind {
            BackendKind::MitsuroHttp => BackendSelection::MitsuroHttp,
            BackendKind::CodexStdio => BackendSelection::CodexStdio,
            BackendKind::CodexWebSocket => BackendSelection::CodexWebSocket,
            BackendKind::Fixture => BackendSelection::Fixture,
        };
        self.connect_backend_selection(selection, cx);
    }

    pub fn reconnect_backend(&mut self, cx: &mut Context<Self>) {
        let kind = self
            .active_backend_kind()
            .unwrap_or(BackendKind::MitsuroHttp);
        let selection = match kind {
            BackendKind::MitsuroHttp => BackendSelection::MitsuroHttp,
            BackendKind::CodexStdio => BackendSelection::CodexStdio,
            BackendKind::CodexWebSocket => BackendSelection::CodexWebSocket,
            BackendKind::Fixture => BackendSelection::Fixture,
        };
        self.connect_backend_selection(selection, cx);
    }

    pub fn status_line(&self) -> &SharedString {
        &self.status_line
    }

    pub fn set_status_line(&mut self, line: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status_line = line.into();
        cx.notify();
    }

    pub fn active_mode(&self) -> ProductMode {
        self.active_mode
    }

    /// Switch product mode (activity rail) and refresh status chrome.
    ///
    /// Selection is preserved: Chat/Codex each remember their last thread; Work
    /// keeps `selected_goal` across hops (goals list is never cleared here).
    pub fn set_mode(&mut self, mode: ProductMode, window: &mut Window, cx: &mut Context<Self>) {
        self.remember_thread_selection_for_mode(self.active_mode);

        // Entering Settings from any other mode: land on General + remember return.
        if matches!(mode, ProductMode::Settings)
            && !matches!(self.active_mode, ProductMode::Settings)
        {
            self.settings_return_mode = self.active_mode;
            self.settings_section = SettingsSection::General;
            self.settings_search_query.clear();
            self.settings_search_input.update(cx, |state, cx| {
                state.set_value("", window, cx);
            });
        }

        self.active_mode = mode;
        window.set_window_title(&mode.window_title());
        self.status_line = match mode {
            ProductMode::Chat => {
                let n = self
                    .threads
                    .iter()
                    .filter(|t| t.surface == ThreadSurface::Chat)
                    .count();
                format!("Chat · {n} conversation(s)").into()
            }
            ProductMode::Work => {
                let n = self.goals.len();
                if self.goals_are_live_hive {
                    let running = self
                        .hive_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.status.running_count)
                        .unwrap_or(0);
                    format!("Hive · {n} run(s) · {running} running · read-only").into()
                } else {
                    format!("Work · {n} goal(s)").into()
                }
            }
            ProductMode::Codex => {
                let n = self
                    .threads
                    .iter()
                    .filter(|t| t.surface == ThreadSurface::Codex)
                    .count();
                format!("Codex · {n} agent thread(s)").into()
            }
            ProductMode::Atlas => {
                // Attach probe runs when the user first opens Atlas (window already live).
                // Full call with Window happens from browser_panel / navigate; here we only
                // refresh status from whatever attach state we already have.
                let kind = self.browser_host_kind_label();
                format!("Atlas / browser · {kind}").into()
            }
            ProductMode::Terminal => {
                if self
                    .backend
                    .as_ref()
                    .is_some_and(|backend| backend.kind() == BackendKind::MitsuroHttp)
                {
                    format!(
                        "Processes · {} background process(es) · read-only",
                        self.background_processes.len()
                    )
                    .into()
                } else {
                    let h = self
                        .terminal
                        .process_handle
                        .as_deref()
                        .unwrap_or("no process");
                    format!("Terminal · {} · {h}", self.terminal.status_label()).into()
                }
            }
            ProductMode::Files => {
                let n = if self.files.search_query.is_empty() {
                    self.files.entries.len()
                } else {
                    self.files.fuzzy_results.len()
                };
                format!("Files · {} · {n} item(s)", self.files.cwd).into()
            }
            ProductMode::Computer => {
                let n = self.environments.len();
                let connected = self
                    .environments
                    .iter()
                    .filter(|e| e.is_connected())
                    .count();
                format!("Computer · {n} environment(s) · {connected} connected").into()
            }
            ProductMode::Extensions => {
                let m = self.mcp_servers.len();
                let p = self.plugins.len();
                let installed = self.plugins.iter().filter(|x| x.installed).count();
                format!("Plugins · {m} MCP · {p} plugin(s) ({installed} installed)").into()
            }
            ProductMode::Settings => format!("Settings · {}", self.settings_section.label()).into(),
            ProductMode::PullRequests => {
                let backend = self
                    .active_backend_kind()
                    .map(Self::backend_display_name)
                    .unwrap_or("no backend");
                format!("Pull requests · unavailable · {backend}").into()
            }
            ProductMode::Sites => "Sites · unavailable on selected backend".into(),
            ProductMode::Scheduled => match self.scheduled_state() {
                SurfaceDataState::Live => format!(
                    "Hive schedules · {} task(s) · read-only",
                    self.scheduled_tasks.as_ref().map_or(0, Vec::len)
                )
                .into(),
                SurfaceDataState::Fixture if self.scheduled_show_tasks => {
                    "Scheduled · explicit fixture tasks".into()
                }
                SurfaceDataState::Fixture => "Scheduled · explicit fixture suggestions".into(),
                state => format!("Scheduled · {}", state.label()).into(),
            },
        };

        if matches!(mode, ProductMode::Files) {
            // Live Ready: prefer real workspace cwd (thread or $HOME), not /fixture-project.
            if self.live_backend().is_some() {
                let cwd = self.files.cwd.as_ref();
                if cwd == FIXTURE_PROJECT_ROOT || cwd.is_empty() {
                    self.files.cwd = self.preferred_workspace_cwd().into();
                    self.files.backend_label = self.files_backend_label();
                    self.files_refresh_directory(window, cx);
                } else if self.files.entries.is_empty() {
                    self.files_refresh_directory(window, cx);
                }
            } else if self.is_explicit_fixture() && self.files.entries.is_empty() {
                self.files_refresh_directory(window, cx);
            }
        }
        if matches!(mode, ProductMode::Computer) {
            if self.environments.is_empty() {
                self.refresh_environments(window, cx);
            } else if self.environment_status_detail.is_none() {
                self.refresh_selected_environment_detail(cx);
            }
        }
        // Re-hit plugin/list + mcpServerStatus/list + skills/list when Ready so
        // the panel reflects the latest live (or honestly empty) catalog.
        if matches!(mode, ProductMode::Extensions)
            && (matches!(self.connection, UiConnection::Ready { .. })
                || self.mcp_servers.is_empty()
                || self.plugins.is_empty())
        {
            self.refresh_extensions(window, cx);
        }

        // Restore per-surface thread selection when entering Chat/Codex.
        // Prefer remembered selection; otherwise keep empty (calm greeting) rather
        // than auto-picking the first demo thread.
        if matches!(mode, ProductMode::Chat | ProductMode::Codex) {
            let surface = match mode {
                ProductMode::Chat => ThreadSurface::Chat,
                _ => ThreadSurface::Codex,
            };
            let remembered = match surface {
                ThreadSurface::Chat => self.selected_chat_thread.clone(),
                ThreadSurface::Codex => self.selected_codex_thread.clone(),
            };
            let remembered_ok = remembered
                .as_ref()
                .and_then(|id| self.threads.iter().find(|t| &t.summary.id == id))
                .map(|t| t.surface == surface)
                .unwrap_or(false);
            if remembered_ok {
                self.selected_thread = remembered;
            } else {
                let selected_ok = self
                    .selected_thread
                    .as_ref()
                    .and_then(|id| self.threads.iter().find(|t| &t.summary.id == id))
                    .map(|t| t.surface == surface)
                    .unwrap_or(false);
                if !selected_ok {
                    // Empty selection → centered greeting (product density, not demo wall).
                    self.selected_thread = None;
                }
            }
            self.remember_thread_selection_for_mode(mode);
            self.update_composer_placeholder(window, cx);
        }

        cx.notify();
    }

    /// Persist current `selected_thread` into the Chat or Codex slot.
    fn remember_thread_selection_for_mode(&mut self, mode: ProductMode) {
        let Some(id) = self.selected_thread.clone() else {
            return;
        };
        let surface = self
            .threads
            .iter()
            .find(|t| t.summary.id == id)
            .map(|t| t.surface);
        match surface {
            Some(ThreadSurface::Chat) if matches!(mode, ProductMode::Chat) => {
                self.selected_chat_thread = Some(id);
            }
            Some(ThreadSurface::Codex) if matches!(mode, ProductMode::Codex) => {
                self.selected_codex_thread = Some(id);
            }
            Some(ThreadSurface::Chat) => {
                self.selected_chat_thread = Some(id);
            }
            Some(ThreadSurface::Codex) => {
                self.selected_codex_thread = Some(id);
            }
            None => {}
        }
    }

    /// Composer placeholder: Chat home uses "Message…"; Codex home "Do anything".
    fn update_composer_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let placeholder = match self.active_mode {
            ProductMode::Chat => "Message…",
            ProductMode::Codex if self.is_calm_stage() || self.is_empty_conversation() => {
                "Do anything"
            }
            _ => "Ask Mitsuro…",
        };
        self.composer_input.update(cx, |state, cx| {
            state.set_placeholder(placeholder, window, cx);
        });
    }

    pub fn goals(&self) -> &[DemoGoal] {
        &self.goals
    }

    pub fn selected_goal_id(&self) -> Option<&str> {
        self.selected_goal.as_deref()
    }

    pub fn selected_goal(&self) -> Option<&DemoGoal> {
        let id = self.selected_goal.as_ref()?;
        self.goals.iter().find(|g| &g.id == id)
    }

    pub fn select_goal(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_goal = Some(id.clone());
        let thread_id = self
            .goals
            .iter()
            .find(|g| g.id == id)
            .and_then(|g| g.thread_id.clone());
        if let Some(g) = self.goals.iter().find(|g| g.id == id) {
            self.status_line = format!("Work · {}", g.objective).into();
        }
        // Best-effort `thread/goal/get` for linked threads (fixture or live).
        if let Some(tid) = thread_id.filter(|_| !self.goals_are_live_hive) {
            self.dispatch_goal_get(tid, cx);
        }
        cx.notify();
    }

    /// CTA: create a fixture goal with plan steps; also `thread/goal/set` when linked.
    pub fn start_new_goal(&mut self, cx: &mut Context<Self>) {
        if !self.is_explicit_fixture() {
            self.status_line = match self.work_state() {
                SurfaceDataState::Live => {
                    "Hive dispatch is not wired from the GPUI client yet; this view is read-only."
                }
                SurfaceDataState::Loading => "Work data is still loading.",
                SurfaceDataState::Error => "Work is unavailable while the backend is in error.",
                _ => "Work goals are not exposed by this backend.",
            }
            .into();
            cx.notify();
            return;
        }
        let id = format!("goal-local-{}", self.goals.len() + 1);
        // Link to a new synthetic thread id so protocol goal/* has a stable key.
        let thread_id = format!("work-thread-{id}");
        let objective = "New goal — describe the long-running outcome".to_string();
        let goal = DemoGoal {
            id: id.clone(),
            objective: objective.clone(),
            status: DemoGoalStatus::Active,
            thread_id: Some(thread_id.clone()),
            updated_at: None,
            plan_items: demo::new_goal_plan_items(&id),
        };
        self.goals.insert(0, goal);
        self.selected_goal = Some(id);
        self.active_mode = ProductMode::Work;
        self.status_line = "Started a new Work goal (fixture + thread/goal/set).".into();
        self.dispatch_goal_set(thread_id, objective, cx);
        cx.notify();
    }

    /// Clear selected goal via `thread/goal/clear` (and drop from local list).
    pub fn clear_selected_goal(&mut self, cx: &mut Context<Self>) {
        if !self.is_explicit_fixture() {
            self.status_line =
                "Work mutations are unavailable outside explicit fixture mode.".into();
            cx.notify();
            return;
        }
        let Some(id) = self.selected_goal.clone() else {
            self.status_line = "Work · no goal selected to clear".into();
            cx.notify();
            return;
        };
        let thread_id = self
            .goals
            .iter()
            .find(|g| g.id == id)
            .and_then(|g| g.thread_id.clone());
        self.goals.retain(|g| g.id != id);
        self.selected_goal = self.goals.first().map(|g| g.id.clone());
        if let Some(tid) = thread_id {
            self.dispatch_goal_clear(tid, cx);
            self.status_line = "Work · goal cleared (thread/goal/clear)".into();
        } else {
            self.status_line = "Work · goal removed (local)".into();
        }
        cx.notify();
    }

    /// Toggle a plan item done flag on the selected goal (local/fixture).
    pub fn toggle_goal_plan_item(&mut self, goal_id: &str, item_id: &str, cx: &mut Context<Self>) {
        if !self.is_explicit_fixture() {
            self.status_line =
                "Work plan mutations are unavailable outside explicit fixture mode.".into();
            cx.notify();
            return;
        }
        if let Some(goal) = self.goals.iter_mut().find(|g| g.id == goal_id) {
            if let Some(item) = goal.plan_items.iter_mut().find(|i| i.id == item_id) {
                item.done = !item.done;
                self.status_line = format!(
                    "Work · plan item {} · {}",
                    if item.done { "done" } else { "open" },
                    item.title
                )
                .into();
            }
        }
        cx.notify();
    }

    fn dispatch_goal_set(&self, thread_id: String, objective: String, cx: &mut Context<Self>) {
        let params = ThreadGoalSetParams::new(thread_id)
            .with_objective(objective)
            .with_status(ThreadGoalStatus::Active);
        if let Some(backend) = self.backend.clone() {
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|e| e.to_string())?;
                        rt.block_on(async {
                            backend
                                .thread_goal_set(params)
                                .await
                                .map_err(|e| e.to_string())
                        })
                    })
                    .await;
            })
            .detach();
        } else if let Some(fixture) = self.fixture.clone().filter(|_| self.is_explicit_fixture()) {
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        // Ensure fixture is connected for typed goal methods.
                        if !fixture.status().is_usable() {
                            let _ = fixture.connect().await;
                        }
                        fixture
                            .thread_goal_set(params)
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
            })
            .detach();
        }
    }

    fn dispatch_goal_get(&self, thread_id: String, cx: &mut Context<Self>) {
        let params = ThreadGoalGetParams::new(thread_id);
        if let Some(backend) = self.backend.clone() {
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|e| e.to_string())?;
                        rt.block_on(async {
                            backend
                                .thread_goal_get(params)
                                .await
                                .map_err(|e| e.to_string())
                        })
                    })
                    .await;
            })
            .detach();
        } else if let Some(fixture) = self.fixture.clone().filter(|_| self.is_explicit_fixture()) {
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        if !fixture.status().is_usable() {
                            let _ = fixture.connect().await;
                        }
                        fixture
                            .thread_goal_get(params)
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
            })
            .detach();
        }
    }

    fn dispatch_goal_clear(&self, thread_id: String, cx: &mut Context<Self>) {
        let params = ThreadGoalClearParams::new(thread_id);
        if let Some(backend) = self.backend.clone() {
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|e| e.to_string())?;
                        rt.block_on(async {
                            backend
                                .thread_goal_clear(params)
                                .await
                                .map_err(|e| e.to_string())
                        })
                    })
                    .await;
            })
            .detach();
        } else if let Some(fixture) = self.fixture.clone().filter(|_| self.is_explicit_fixture()) {
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        if !fixture.status().is_usable() {
                            let _ = fixture.connect().await;
                        }
                        fixture
                            .thread_goal_clear(params)
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
            })
            .detach();
        }
    }

    /// Thread surface for the current Chat/Codex mode (default Codex).
    pub fn active_thread_surface(&self) -> ThreadSurface {
        match self.active_mode {
            ProductMode::Chat => ThreadSurface::Chat,
            ProductMode::Codex
            | ProductMode::PullRequests
            | ProductMode::Sites
            | ProductMode::Scheduled
            | ProductMode::Extensions => ThreadSurface::Codex,
            _ => ThreadSurface::Codex,
        }
    }

    /// Home hero stage: Chat/Codex with no selected thread.
    /// Sidebar stays visible (bar density); main column shows centered hero + composer.
    pub fn is_calm_stage(&self) -> bool {
        matches!(self.active_mode, ProductMode::Chat | ProductMode::Codex)
            && self.selected_thread.is_none()
    }

    /// Whether the main transcript column is empty (no selection or no messages).
    /// Used to quiet composer chrome (hide model chip) until a turn starts.
    pub fn is_empty_conversation(&self) -> bool {
        match self.selected_thread() {
            None => true,
            Some(t) => t.messages.is_empty(),
        }
    }

    pub fn selected_delegation(&self) -> Option<&SessionDelegationProjection> {
        self.selected_thread
            .as_ref()
            .and_then(|thread_id| self.delegations.get(thread_id))
            .filter(|projection| !projection.groups.is_empty())
    }

    pub fn terminal_session(&self) -> &TerminalSession {
        &self.terminal
    }

    pub fn terminal_interactive_available(&self) -> bool {
        self.backend
            .as_ref()
            .map(|backend| backend.capabilities().processes)
            .unwrap_or_else(|| self.is_explicit_fixture())
    }

    pub fn terminal_cmd_input(&self) -> &Entity<InputState> {
        &self.terminal_cmd_input
    }

    pub fn terminal_stdin_input(&self) -> &Entity<InputState> {
        &self.terminal_stdin_input
    }

    pub fn files_session(&self) -> &FilesSession {
        &self.files
    }

    pub fn files_path_input(&self) -> &Entity<InputState> {
        &self.files_path_input
    }

    pub fn files_search_input(&self) -> &Entity<InputState> {
        &self.files_search_input
    }

    fn files_backend_label(&self) -> SharedString {
        if self.live_backend().is_some() {
            "app-server".into()
        } else if self.is_explicit_fixture() {
            "fixture".into()
        } else {
            "unavailable".into()
        }
    }

    fn terminal_backend_label(&self) -> SharedString {
        if self.live_backend().is_some() {
            "app-server".into()
        } else if self.is_explicit_fixture() {
            "fixture".into()
        } else {
            "unavailable".into()
        }
    }

    /// Preferred workspace root for Files/Terminal when live: selected thread cwd → any thread cwd → $HOME.
    fn preferred_workspace_cwd(&self) -> String {
        let pick = |cwd: &str| -> Option<String> {
            let p = path_from_cwd_field(cwd);
            if p.is_empty() || p == FIXTURE_PROJECT_ROOT {
                return None;
            }
            if p.starts_with('/') {
                Some(normalize_abs_path(&p))
            } else {
                None
            }
        };
        if let Some(id) = &self.selected_thread {
            if let Some(t) = self.threads.iter().find(|t| &t.summary.id == id) {
                if let Some(c) = t.summary.cwd.as_deref().and_then(pick) {
                    return c;
                }
            }
        }
        for t in &self.threads {
            if let Some(c) = t.summary.cwd.as_deref().and_then(pick) {
                return c;
            }
        }
        std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())
    }

    /// MCP server that looks like GitHub (for PR honesty chrome).
    pub fn mcp_github_server(&self) -> Option<&McpServerStatus> {
        self.mcp_servers.iter().find(|s| {
            let name = s.name.to_lowercase();
            let title = s.display_title().to_lowercase();
            name.contains("github") || title.contains("github")
        })
    }

    /// Refresh directory listing for current Files cwd (`fs/readDirectory`).
    pub fn files_refresh_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = normalize_abs_path(self.files.cwd.as_ref());
        self.files.cwd = path.clone().into();
        self.files.search_query.clear();
        self.files.fuzzy_results.clear();
        self.files_path_input.update(cx, |state, cx| {
            state.set_value(path, window, cx);
        });
        self.files_search_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });

        self.files_refresh_directory_data(cx);
    }

    fn files_refresh_directory_data(&mut self, cx: &mut Context<Self>) {
        let path = normalize_abs_path(self.files.cwd.as_ref());
        self.files.cwd = path.clone().into();
        let result = self.files_call_read_directory(FsReadDirectoryParams::new(path), cx);
        match result {
            Ok(entries) => {
                self.files.entries = entries;
                self.files.backend_label = self.files_backend_label();
                self.files.preview = SharedString::from("");
                self.files.preview_error = None;
                self.files.selected_path = None;
                self.status_line = format!(
                    "Files · {} · {} · {} item(s)",
                    self.files.backend_label,
                    self.files.cwd,
                    self.files.entries.len()
                )
                .into();
            }
            Err(e) => {
                self.files.entries.clear();
                self.files.preview_error = Some(e.clone());
                self.status_line = format!("Files · list failed: {e}").into();
            }
        }
        cx.notify();
    }

    /// Navigate path bar value as cwd.
    pub fn files_navigate_path_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.files_path_input.read(cx).value().to_string();
        let path = normalize_abs_path(raw.trim());
        if path.is_empty() {
            self.status_line = "Files · empty path".into();
            cx.notify();
            return;
        }
        self.files.cwd = path.into();
        self.files_refresh_directory(window, cx);
    }

    /// Parent directory of current cwd.
    pub fn files_go_up(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let cwd = normalize_abs_path(self.files.cwd.as_ref());
        let root_fallback = if self.live_backend().is_some() {
            self.preferred_workspace_cwd()
        } else {
            FIXTURE_PROJECT_ROOT.to_string()
        };
        let parent = if cwd == "/" {
            "/".to_string()
        } else {
            let trimmed = cwd.trim_end_matches('/');
            match trimmed.rsplit_once('/') {
                Some(("", _)) => "/".to_string(),
                Some((p, _)) => normalize_abs_path(p),
                None => root_fallback,
            }
        };
        self.files.cwd = parent.into();
        self.files_refresh_directory(window, cx);
    }

    /// Activate a directory entry (enter dir) or open a file preview.
    pub fn files_activate_entry(
        &mut self,
        name: String,
        is_dir: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let full = join_abs(self.files.cwd.as_ref(), &name);
        if is_dir {
            self.files.cwd = full.into();
            self.files_refresh_directory(window, cx);
        } else {
            self.files_open_path(full, window, cx);
        }
    }

    /// Open/preview a file via `fs/readFile`.
    pub fn files_open_path(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let path = normalize_abs_path(&path);
        self.files.selected_path = Some(path.clone());
        let params = FsReadFileParams::new(path.clone());
        match self.files_call_read_file(params, cx) {
            Ok(text) => {
                self.files.preview = text.into();
                self.files.preview_error = None;
                self.status_line = format!("Files · preview · {path}").into();
            }
            Err(e) => {
                self.files.preview = SharedString::from("");
                self.files.preview_error = Some(e.clone());
                self.status_line = format!("Files · read failed: {e}").into();
            }
        }
        let _ = window;
        cx.notify();
    }

    /// Run `fuzzyFileSearch` against fixture/project roots.
    pub fn files_run_fuzzy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.files_search_input.read(cx).value().to_string();
        let query = query.trim().to_string();
        self.files.search_query = query.clone();
        if query.is_empty() {
            self.files.fuzzy_results.clear();
            self.files_refresh_directory(window, cx);
            return;
        }
        let roots = vec![normalize_abs_path(self.files.cwd.as_ref())];
        let params = FuzzyFileSearchParams::new(query.clone(), roots);
        match self.files_call_fuzzy(params, cx) {
            Ok(files) => {
                self.files.fuzzy_results = files;
                self.files.backend_label = self.files_backend_label();
                self.status_line = format!(
                    "Files · {} · fuzzy “{query}” · {} hit(s)",
                    self.files.backend_label,
                    self.files.fuzzy_results.len()
                )
                .into();
            }
            Err(e) => {
                self.files.fuzzy_results.clear();
                self.status_line = format!("Files · fuzzy failed: {e}").into();
            }
        }
        let _ = window;
        cx.notify();
    }

    fn files_call_read_directory(
        &self,
        params: FsReadDirectoryParams,
        cx: &mut Context<Self>,
    ) -> Result<Vec<FsReadDirectoryEntry>, String> {
        // Prefer live app-server when Ready — fixture is always present for turns offline.
        if let Some(backend) = self.live_backend() {
            let runner = Arc::clone(&backend);
            return backend.block_on(async move {
                runner
                    .browse_directory(params.path)
                    .await
                    .map(|entries| {
                        entries
                            .into_iter()
                            .map(|entry| FsReadDirectoryEntry {
                                file_name: entry.name,
                                is_directory: entry.is_directory,
                                is_file: entry.is_file,
                            })
                            .collect()
                    })
                    .map_err(|e| e.to_string())
            });
        }
        if self.is_explicit_fixture() {
            let Some(fixture) = self.fixture.clone() else {
                return Err("fixture backend is unavailable".into());
            };
            return cx.background_executor().block(async {
                if !fixture.status().is_usable() {
                    let _ = fixture.connect().await;
                }
                fixture
                    .fs_read_directory(params)
                    .await
                    .map(|r| r.entries)
                    .map_err(|e| e.to_string())
            });
        }
        Err("no backend".into())
    }

    fn files_call_read_file(
        &self,
        params: FsReadFileParams,
        cx: &mut Context<Self>,
    ) -> Result<String, String> {
        if let Some(backend) = self.live_backend() {
            let runner = Arc::clone(&backend);
            return backend.block_on(async move {
                runner
                    .read_text_file(params.path)
                    .await
                    .map(|file| file.text)
                    .map_err(|e| e.to_string())
            });
        }
        if self.is_explicit_fixture() {
            let Some(fixture) = self.fixture.clone() else {
                return Err("fixture backend is unavailable".into());
            };
            return cx.background_executor().block(async {
                if !fixture.status().is_usable() {
                    let _ = fixture.connect().await;
                }
                fixture
                    .fs_read_file(params)
                    .await
                    .map(|r| r.text_lossy())
                    .map_err(|e| e.to_string())
            });
        }
        Err("no backend".into())
    }

    fn files_call_fuzzy(
        &self,
        params: FuzzyFileSearchParams,
        cx: &mut Context<Self>,
    ) -> Result<Vec<FuzzyFileSearchResult>, String> {
        if let Some(backend) = self.live_backend() {
            let runner = Arc::clone(&backend);
            return backend.block_on(async move {
                runner
                    .search_files(params.query, params.roots)
                    .await
                    .map(|files| files.into_iter().map(file_match_from_product).collect())
                    .map_err(|e| e.to_string())
            });
        }
        if self.is_explicit_fixture() {
            let Some(fixture) = self.fixture.clone() else {
                return Err("fixture backend is unavailable".into());
            };
            return cx.background_executor().block(async {
                if !fixture.status().is_usable() {
                    let _ = fixture.connect().await;
                }
                fixture
                    .fuzzy_file_search(params)
                    .await
                    .map(|r| r.files)
                    .map_err(|e| e.to_string())
            });
        }
        Err("no backend".into())
    }

    /// Apply process stream events to the terminal output buffer.
    fn apply_process_events(&mut self, events: &[TurnStreamEvent]) {
        for ev in events {
            match ev {
                TurnStreamEvent::ProcessOutputDelta { delta, .. } => {
                    let mut out = self.terminal.output.to_string();
                    out.push_str(delta);
                    self.terminal.output = out.into();
                }
                TurnStreamEvent::ProcessExited {
                    exit_code,
                    process_handle,
                    stdout,
                    stderr,
                    ..
                } => {
                    if !stdout.is_empty() {
                        let mut out = self.terminal.output.to_string();
                        out.push_str(stdout);
                        self.terminal.output = out.into();
                    }
                    if !stderr.is_empty() {
                        let mut out = self.terminal.output.to_string();
                        out.push_str(stderr);
                        self.terminal.output = out.into();
                    }
                    let mut out = self.terminal.output.to_string();
                    out.push_str(&format!(
                        "\n[exited {exit_code}] processHandle={process_handle}\n"
                    ));
                    self.terminal.output = out.into();
                    self.terminal.running = false;
                    self.terminal.status = TerminalSessionStatus::Exited;
                    self.terminal.exit_code = Some(*exit_code);
                }
                _ => {}
            }
        }
    }

    /// Start a process via the selected live backend or explicit fixture backend.
    pub fn terminal_spawn(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.terminal.running {
            self.status_line = "Terminal · already running".into();
            cx.notify();
            return;
        }
        if self
            .live_backend()
            .is_some_and(|backend| !backend.capabilities().processes)
        {
            self.status_line = "Terminal spawn is not supported by the selected backend.".into();
            self.terminal.status = TerminalSessionStatus::Error;
            cx.notify();
            return;
        }
        if self.live_backend().is_none() && !self.is_explicit_fixture() {
            self.status_line =
                "Terminal is unavailable until a process-capable backend is ready.".into();
            self.terminal.status = TerminalSessionStatus::Error;
            cx.notify();
            return;
        }
        let raw = self.terminal_cmd_input.read(cx).value().to_string();
        let cmd = raw.trim().to_string();
        if cmd.is_empty() {
            self.status_line = "Terminal · empty command".into();
            cx.notify();
            return;
        }
        self.terminal_handle_seq = self.terminal_handle_seq.saturating_add(1);
        let handle = format!("mitsuro-term-{}", self.terminal_handle_seq);
        let cwd = if self.live_backend().is_some() {
            self.preferred_workspace_cwd()
        } else {
            "/tmp/mitsuro-fixture".into()
        };
        let params = if cmd.starts_with("echo ") || !cmd.contains(' ') {
            // Simple argv: `echo hello…` or single token.
            let parts: Vec<String> = shell_split_simple(&cmd);
            ProcessSpawnParams::streaming(parts, handle.clone(), cwd)
        } else {
            ProcessSpawnParams::bash_lc(cmd.clone(), handle.clone(), cwd)
        };

        self.terminal.backend_label = self.terminal_backend_label();
        self.terminal.process_handle = Some(handle.clone());
        self.terminal.output = format!("$ {cmd}\n").into();
        self.terminal.running = true;
        self.terminal.status = TerminalSessionStatus::Running;
        self.terminal.exit_code = None;
        self.status_line = format!("Terminal · spawning {handle}").into();
        cx.notify();

        // Live failures remain live failures; production never substitutes a fixture process.
        if let Some(backend) = self.live_backend() {
            let result = cx
                .background_executor()
                .block(async move { backend.process_spawn(params).await });
            match result {
                Ok(resp) => {
                    if let Some(h) = resp.process_handle {
                        self.terminal.process_handle = Some(h);
                    }
                    self.terminal.backend_label = "app-server".into();
                    let mut out = self.terminal.output.to_string();
                    out.push_str(
                        "[process/spawn · app-server]\n\
                         (stdout/stderr via process/outputDelta when notification bridge is active)\n",
                    );
                    self.terminal.output = out.into();
                    self.status_line = "Terminal · process/spawn (app-server)".into();
                }
                Err(e) => {
                    self.terminal.running = false;
                    self.terminal.status = TerminalSessionStatus::Error;
                    let mut out = self.terminal.output.to_string();
                    out.push_str(&format!("[error] {e}\n"));
                    self.terminal.output = out.into();
                    self.status_line = format!("Terminal · spawn failed: {e}").into();
                }
            }
            let _ = window;
            cx.notify();
            return;
        }

        if self.is_explicit_fixture() {
            let Some(fixture) = self.fixture.clone() else {
                self.terminal.running = false;
                self.terminal.status = TerminalSessionStatus::Error;
                self.status_line = "Terminal · fixture backend unavailable".into();
                cx.notify();
                return;
            };
            // Ensure fixture is connected.
            cx.background_executor().block(async {
                if !fixture.status().is_usable() {
                    let _ = fixture.connect().await;
                }
            });
            let result = cx
                .background_executor()
                .block(async { fixture.process_spawn(params).await });
            match result {
                Ok(resp) => {
                    if let Some(h) = resp.process_handle {
                        self.terminal.process_handle = Some(h);
                    }
                    let events = fixture.take_process_events();
                    self.apply_process_events(&events);
                    self.terminal.backend_label = "fixture".into();
                    self.status_line = if self.terminal.running {
                        "Terminal · running (fixture)".into()
                    } else {
                        "Terminal · exited (fixture)".into()
                    };
                }
                Err(e) => {
                    self.terminal.running = false;
                    self.terminal.status = TerminalSessionStatus::Error;
                    let mut out = self.terminal.output.to_string();
                    out.push_str(&format!("[error] {e}\n"));
                    self.terminal.output = out.into();
                    self.status_line = format!("Terminal · spawn failed: {e}").into();
                }
            }
        } else {
            self.terminal.running = false;
            self.terminal.status = TerminalSessionStatus::Error;
            self.status_line = "Terminal · no backend".into();
        }
        let _ = window;
        cx.notify();
    }

    /// Write stdin via live `process/writeStdin` when the session is app-server, else fixture.
    pub fn terminal_write_stdin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = match self.terminal.process_handle.clone() {
            Some(h) if self.terminal.running => h,
            _ => {
                self.status_line = "Terminal · no running process".into();
                cx.notify();
                return;
            }
        };
        let text = self.terminal_stdin_input.read(cx).value().to_string();
        if text.is_empty() {
            return;
        }
        let payload = if text.ends_with('\n') {
            text
        } else {
            format!("{text}\n")
        };
        let params = ProcessWriteStdinParams::text(&handle, &payload);
        let use_live =
            self.terminal.backend_label.as_ref() == "app-server" && self.live_backend().is_some();

        if use_live {
            if let Some(backend) = self.live_backend() {
                let result = cx
                    .background_executor()
                    .block(async move { backend.process_write_stdin(params).await });
                match result {
                    Ok(_) => {
                        let mut out = self.terminal.output.to_string();
                        out.push_str(&format!("→ {payload}"));
                        self.terminal.output = out.into();
                        self.terminal_stdin_input.update(cx, |state, cx| {
                            state.set_value("", window, cx);
                        });
                        self.status_line = "Terminal · writeStdin (app-server)".into();
                    }
                    Err(e) => {
                        let mut out = self.terminal.output.to_string();
                        out.push_str(&format!("[stdin error] {e}\n"));
                        self.terminal.output = out.into();
                        self.status_line = format!("Terminal · writeStdin failed: {e}").into();
                    }
                }
                cx.notify();
                return;
            }
        }

        if self.is_explicit_fixture() && self.terminal.backend_label.as_ref() == "fixture" {
            if let Some(fixture) = self.fixture.clone() {
                let result = cx
                    .background_executor()
                    .block(async { fixture.process_write_stdin(params).await });
                match result {
                    Ok(_) => {
                        let events = fixture.take_process_events();
                        self.apply_process_events(&events);
                        self.terminal_stdin_input.update(cx, |state, cx| {
                            state.set_value("", window, cx);
                        });
                        self.status_line = "Terminal · writeStdin (fixture)".into();
                    }
                    Err(e) => {
                        let mut out = self.terminal.output.to_string();
                        out.push_str(&format!("[stdin error] {e}\n"));
                        self.terminal.output = out.into();
                        self.status_line = format!("Terminal · writeStdin failed: {e}").into();
                    }
                }
            }
        }
        cx.notify();
    }

    /// Kill the running process via live `process/kill` when session is app-server, else fixture.
    pub fn terminal_kill(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let handle = match self.terminal.process_handle.clone() {
            Some(h) if self.terminal.running => h,
            _ => {
                self.status_line = "Terminal · nothing to kill".into();
                cx.notify();
                return;
            }
        };
        let use_live =
            self.terminal.backend_label.as_ref() == "app-server" && self.live_backend().is_some();

        if use_live {
            if let Some(backend) = self.live_backend() {
                let result = cx.background_executor().block(async move {
                    backend.process_kill(ProcessKillParams::new(handle)).await
                });
                match result {
                    Ok(_) => {
                        self.terminal.running = false;
                        self.terminal.status = TerminalSessionStatus::Exited;
                        self.terminal.exit_code = Some(137);
                        let mut out = self.terminal.output.to_string();
                        out.push_str("\n[killed · process/kill · app-server · exit 137]\n");
                        self.terminal.output = out.into();
                        self.status_line = "Terminal · killed (app-server)".into();
                    }
                    Err(e) => {
                        let mut out = self.terminal.output.to_string();
                        out.push_str(&format!("[kill error] {e}\n"));
                        self.terminal.output = out.into();
                        self.status_line = format!("Terminal · kill failed: {e}").into();
                    }
                }
                cx.notify();
                return;
            }
        }

        if self.is_explicit_fixture() && self.terminal.backend_label.as_ref() == "fixture" {
            if let Some(fixture) = self.fixture.clone() {
                let result = cx
                    .background_executor()
                    .block(async { fixture.process_kill(ProcessKillParams::new(handle)).await });
                match result {
                    Ok(_) => {
                        let events = fixture.take_process_events();
                        self.apply_process_events(&events);
                        self.status_line = "Terminal · killed (fixture)".into();
                    }
                    Err(e) => {
                        let mut out = self.terminal.output.to_string();
                        out.push_str(&format!("[kill error] {e}\n"));
                        self.terminal.output = out.into();
                        self.status_line = format!("Terminal · kill failed: {e}").into();
                    }
                }
            }
        }
        cx.notify();
    }

    /// Open Atlas mode and request native host attach (handle probe / optional embed).
    pub fn open_atlas(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.browser_request_attach(window, cx);
        self.set_mode(ProductMode::Atlas, window, cx);
    }

    /// Leave Settings via "Back to app" (restore Chat/Codex or prior mode).
    pub fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dest = match self.settings_return_mode {
            ProductMode::Settings => ProductMode::Codex,
            other => other,
        };
        self.set_mode(dest, window, cx);
    }

    pub fn settings_section(&self) -> SettingsSection {
        self.settings_section
    }

    pub fn set_settings_section(&mut self, section: SettingsSection, cx: &mut Context<Self>) {
        self.settings_section = section;
        self.status_line = format!("Settings · {}", section.label()).into();
        cx.notify();
    }

    pub fn scheduled_show_tasks(&self) -> bool {
        self.scheduled_tasks
            .as_ref()
            .is_some_and(|tasks| !tasks.is_empty())
            || self.scheduled_show_tasks
    }

    pub fn scheduled_tasks(&self) -> Option<&[ProductSchedule]> {
        self.scheduled_tasks.as_deref()
    }

    pub fn set_scheduled_show_tasks(&mut self, show: bool, cx: &mut Context<Self>) {
        if !self.is_explicit_fixture() {
            self.status_line =
                "Scheduled tasks are read-only or unsupported by this backend.".into();
            cx.notify();
            return;
        }
        self.scheduled_show_tasks = show;
        self.status_line = if show {
            "Scheduled · fixture demo tasks".into()
        } else {
            "Scheduled · suggestions (no schedule protocol)".into()
        };
        cx.notify();
    }

    pub fn request_schedule_creation(&mut self, suggestion: Option<&str>, cx: &mut Context<Self>) {
        if self.is_explicit_fixture() {
            self.scheduled_show_tasks = true;
            self.status_line = suggestion.map_or_else(
                || "Scheduled · fixture demo tasks".into(),
                |name| format!("Scheduled · added fixture suggestion “{name}”").into(),
            );
        } else {
            self.status_line = "Schedule creation is unavailable for this backend.".into();
        }
        cx.notify();
    }

    pub fn scheduled_enabled(&self) -> &[bool] {
        &self.scheduled_enabled
    }

    pub fn toggle_scheduled_enabled(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.is_explicit_fixture() {
            self.status_line =
                "Schedule mutations are unavailable outside explicit fixture mode.".into();
            cx.notify();
            return;
        }
        if let Some(slot) = self.scheduled_enabled.get_mut(index) {
            *slot = !*slot;
            let on = *slot;
            self.status_line = format!(
                "Scheduled · task {} {}",
                index + 1,
                if on { "enabled" } else { "disabled" }
            )
            .into();
            cx.notify();
        }
    }

    pub fn plugins_filter(&self) -> PluginsFilter {
        self.plugins_filter
    }

    pub fn set_plugins_filter(&mut self, filter: PluginsFilter, cx: &mut Context<Self>) {
        self.plugins_filter = filter;
        let label = match filter {
            PluginsFilter::Public => "Public",
            PluginsFilter::Personal => "Personal",
            PluginsFilter::Mcp => "MCP",
        };
        self.status_line = format!("Plugins · {label}").into();
        cx.notify();
    }

    pub fn plugins_surface_tab(&self) -> PluginsSurfaceTab {
        self.plugins_surface_tab
    }

    pub fn plugins_search_input(&self) -> &Entity<InputState> {
        &self.plugins_search_input
    }

    pub fn expanded_plugin_sections(&self) -> &std::collections::HashSet<String> {
        &self.expanded_plugin_sections
    }

    pub fn expand_plugin_section(&mut self, section: String, cx: &mut Context<Self>) {
        self.expanded_plugin_sections.insert(section.clone());
        self.status_line = format!("Plugins · expanded {section}").into();
        cx.notify();
    }

    pub fn set_plugins_surface_tab(&mut self, tab: PluginsSurfaceTab, cx: &mut Context<Self>) {
        self.plugins_surface_tab = tab;
        self.status_line = match tab {
            PluginsSurfaceTab::Plugins => "Plugins".into(),
            PluginsSurfaceTab::Skills => "Skills".into(),
        };
        cx.notify();
    }

    pub fn settings_search_input(&self) -> &Entity<InputState> {
        &self.settings_search_input
    }

    pub fn settings_search_query(&self) -> &str {
        &self.settings_search_query
    }

    /// Sync settings search box → filter string (call from render).
    pub fn sync_settings_search(&mut self, cx: &mut Context<Self>) {
        let value = self.settings_search_input.read(cx).value().to_string();
        if value != self.settings_search_query {
            self.settings_search_query = value;
        }
    }

    pub fn settings_toggle(&self, key: &str, default: bool) -> bool {
        self.settings_toggles.get(key).copied().unwrap_or(default)
    }

    pub fn flip_settings_toggle(&mut self, key: &str, default: bool, cx: &mut Context<Self>) {
        let next = !self.settings_toggle(key, default);
        self.settings_toggles.insert(key.to_string(), next);
        self.preferences
            .settings_toggles
            .insert(key.to_string(), next);
        self.save_preferences_best_effort();
        self.status_line = format!(
            "Settings · {} · {} · saved locally (runtime wiring unchanged)",
            self.settings_section.label(),
            if next { "on" } else { "off" }
        )
        .into();
        cx.notify();
    }

    pub fn settings_choice(&self, key: &str, default: &str) -> String {
        self.settings_choices
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string())
    }

    pub fn realtime_voices_state(&self) -> SurfaceDataState {
        self.realtime_voices_state
    }

    pub fn realtime_voice_options(&self) -> Vec<String> {
        let Some(voices) = self.realtime_voices.as_ref() else {
            return Vec::new();
        };
        voices.v2.iter().map(|voice| voice.label()).collect()
    }

    pub fn selected_realtime_voice_label(&self) -> String {
        let default = self
            .realtime_voices
            .as_ref()
            .map(|voices| voices.default_v2.label())
            .unwrap_or_else(|| "Unavailable".to_owned());
        self.settings_choice("voice_output", &default)
    }

    pub fn select_realtime_voice(&mut self, label: String, cx: &mut Context<Self>) {
        let is_live_choice = self.realtime_voices.as_ref().is_some_and(|voices| {
            voices
                .v2
                .iter()
                .any(|voice| voice.label().eq_ignore_ascii_case(&label))
        });
        if !is_live_choice {
            self.status_line = "That voice is not in the connected Codex catalog.".into();
            cx.notify();
            return;
        }
        self.settings_choices
            .insert("voice_output".to_owned(), label.clone());
        self.preferences
            .settings_choices
            .insert("voice_output".to_owned(), label.clone());
        self.save_preferences_best_effort();
        self.status_line = format!("Voice · {label} · used for new realtime sessions").into();
        cx.notify();
    }

    fn apply_realtime_voices(&mut self, voices: RealtimeVoicesList) {
        let selected = self.settings_choice("voice_output", "");
        let selected_is_valid = voices
            .v2
            .iter()
            .any(|voice| voice.label().eq_ignore_ascii_case(&selected));
        if !selected_is_valid {
            let label = voices.default_v2.label();
            self.settings_choices
                .insert("voice_output".to_owned(), label.clone());
            self.preferences
                .settings_choices
                .insert("voice_output".to_owned(), label);
            self.save_preferences_best_effort();
        }
        self.realtime_voices = Some(voices);
        self.realtime_voices_state = SurfaceDataState::Live;
    }

    pub fn selected_realtime_voice(&self) -> Option<RealtimeVoice> {
        let selected = self.selected_realtime_voice_label();
        self.realtime_voices
            .as_ref()?
            .v2
            .iter()
            .copied()
            .find(|voice| voice.label().eq_ignore_ascii_case(&selected))
    }

    pub fn realtime_voice_active(&self) -> bool {
        self.realtime_voice_runtime.is_some()
    }

    pub fn realtime_voice_available(&self) -> bool {
        if self.realtime_voice_active() {
            return true;
        }
        !self.turn_in_progress
            && matches!(self.connection, UiConnection::Ready { has_auth: true, .. })
            && self
                .live_backend()
                .is_some_and(|backend| backend.capabilities().realtime_voice)
            && self.realtime_voices_state == SurfaceDataState::Live
            && self.selected_realtime_voice().is_some()
            && command_available("pw-record")
    }

    pub fn toggle_realtime_voice(&mut self, cx: &mut Context<Self>) {
        if self.realtime_voice_runtime.is_some() {
            self.stop_realtime_voice(cx);
        } else {
            self.start_realtime_voice(cx);
        }
    }

    fn start_realtime_voice(&mut self, cx: &mut Context<Self>) {
        if !self.realtime_voice_available() {
            self.status_line = if !command_available("pw-record") {
                "Voice chat requires PipeWire's pw-record command.".into()
            } else {
                "Voice chat is unavailable for the current backend or account state.".into()
            };
            cx.notify();
            return;
        }
        let Some(backend) = self.live_backend() else {
            return;
        };
        let Some(voice) = self.selected_realtime_voice() else {
            return;
        };
        if self.selected_thread.is_none() {
            self.new_thread_local(self.active_thread_surface(), cx);
        }
        let Some(ui_thread_id) = self.selected_thread.clone() else {
            self.status_line = "Voice chat could not create a conversation.".into();
            cx.notify();
            return;
        };

        self.realtime_voice_generation = self.realtime_voice_generation.wrapping_add(1);
        let generation = self.realtime_voice_generation;
        if ui_thread_id.starts_with("local-") {
            self.status_line = "Creating a server conversation for voice chat…".into();
            self.promote_local_then_realtime(ui_thread_id, backend, voice, generation, cx);
        } else if let Some(session_id) = self.live_session_id(&ui_thread_id) {
            self.begin_realtime_voice(backend, session_id, ui_thread_id, voice, generation, cx);
        } else {
            self.status_line =
                "Voice chat refused: this conversation has no backend session identity.".into();
        }
        cx.notify();
    }

    fn promote_local_then_realtime(
        &mut self,
        local_id: String,
        backend: Arc<DesktopBackend>,
        voice: RealtimeVoice,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let backend_generation = self.backend_generation;
        let cwd = self.composer_workspace_dir().map(ToOwned::to_owned);
        let model = self.selected_model_slug();
        let access_mode = self.composer_access_mode();
        let speed_mode = self.selected_speed_mode();
        cx.spawn(async move |this, cx| {
            let create_backend = Arc::clone(&backend);
            let result = cx
                .background_spawn(async move {
                    create_backend
                        .create_session(CreateSession {
                            working_dir: cwd,
                            model,
                            ephemeral: false,
                            access_mode,
                            speed_mode,
                        })
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != backend_generation
                    || app.realtime_voice_generation != generation
                {
                    if let Ok(session) = result {
                        delete_session_best_effort(backend, session.id, cx);
                    }
                    return;
                }
                match result {
                    Ok(session) => {
                        let backend_session_id = session.id.clone();
                        let summary = thread_summary_from_session(session);
                        let new_id = summary.id.clone();
                        if let Some(index) = app
                            .threads
                            .iter()
                            .position(|thread| thread.summary.id == local_id)
                        {
                            let mut thread = app.threads.remove(index);
                            thread.summary = summary;
                            thread.backend_session_id = Some(backend_session_id.clone());
                            app.threads.insert(0, thread);
                        }
                        if let Some(mode) = app.composer_access_modes.remove(&local_id) {
                            app.composer_access_modes.insert(new_id.clone(), mode);
                        }
                        app.selected_thread = Some(new_id.clone());
                        match app.active_thread_surface() {
                            ThreadSurface::Chat => app.selected_chat_thread = Some(new_id.clone()),
                            ThreadSurface::Codex => {
                                app.selected_codex_thread = Some(new_id.clone())
                            }
                        }
                        app.begin_realtime_voice(
                            backend,
                            backend_session_id,
                            new_id,
                            voice,
                            generation,
                            cx,
                        );
                    }
                    Err(error) => {
                        app.status_line =
                            format!("Voice chat session creation failed · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn begin_realtime_voice(
        &mut self,
        backend: Arc<DesktopBackend>,
        session_id: BackendSessionId,
        ui_thread_id: String,
        voice: RealtimeVoice,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let capture_stop = Arc::new(AtomicBool::new(false));
        self.realtime_voice_runtime = Some(RealtimeVoiceRuntime {
            session_id: session_id.clone(),
            ui_thread_id,
            capture_stop: Arc::clone(&capture_stop),
            phase: RealtimeVoicePhase::Starting,
            playback: None,
        });
        self.status_line = format!("Starting {} voice chat…", voice.label()).into();

        let backend_generation = self.backend_generation;
        let mut params = ThreadRealtimeStartParams::websocket(
            session_id.raw.clone(),
            RealtimeOutputModality::Audio,
        );
        params.voice = Some(voice);
        let request_backend = Arc::clone(&backend);
        let request_session = session_id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    request_backend
                        .realtime_start(&request_session, params)
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != backend_generation
                    || app.realtime_voice_generation != generation
                {
                    return;
                }
                match result {
                    Ok(_) => {
                        if let Some(runtime) = app.realtime_voice_runtime.as_mut() {
                            runtime.phase = RealtimeVoicePhase::Active;
                        }
                        app.status_line =
                            format!("Voice chat active · {} · system microphone", voice.label())
                                .into();
                        app.spawn_realtime_capture(
                            backend,
                            session_id,
                            capture_stop,
                            generation,
                            cx,
                        );
                    }
                    Err(error) => {
                        app.realtime_voice_runtime = None;
                        app.status_line = format!("Voice chat failed to start · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn spawn_realtime_capture(
        &mut self,
        backend: Arc<DesktopBackend>,
        session_id: BackendSessionId,
        capture_stop: Arc<AtomicBool>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    stream_pipewire_microphone(backend, session_id, capture_stop)
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.realtime_voice_generation != generation {
                    return;
                }
                let stopping = app
                    .realtime_voice_runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.phase == RealtimeVoicePhase::Stopping);
                if !stopping {
                    app.realtime_voice_runtime = None;
                    app.status_line = match result {
                        Ok(()) => "Voice microphone stream ended.".into(),
                        Err(error) => format!("Voice microphone failed · {error}").into(),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn stop_realtime_voice(&mut self, cx: &mut Context<Self>) {
        let Some(runtime) = self.realtime_voice_runtime.as_mut() else {
            return;
        };
        runtime.phase = RealtimeVoicePhase::Stopping;
        runtime.capture_stop.store(true, Ordering::SeqCst);
        let session_id = runtime.session_id.clone();
        let Some(backend) = self.backend.clone() else {
            self.realtime_voice_runtime = None;
            return;
        };
        self.realtime_voice_generation = self.realtime_voice_generation.wrapping_add(1);
        let generation = self.realtime_voice_generation;
        self.status_line = "Ending voice chat…".into();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    backend
                        .realtime_stop(
                            &session_id,
                            ThreadRealtimeStopParams {
                                thread_id: session_id.raw.clone(),
                            },
                        )
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.realtime_voice_generation != generation {
                    return;
                }
                app.realtime_voice_runtime = None;
                app.status_line = match result {
                    Ok(_) => "Voice chat ended.".into(),
                    Err(error) => format!("Voice chat stop failed · {error}").into(),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub fn set_settings_choice(
        &mut self,
        key: &str,
        value: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        let value = value.into();
        self.settings_choices.insert(key.to_string(), value.clone());
        self.preferences
            .settings_choices
            .insert(key.to_string(), value.clone());
        self.save_preferences_best_effort();
        self.status_line = format!(
            "Settings · {} · {value} · saved locally (runtime wiring unchanged)",
            self.settings_section.label()
        )
        .into();
        cx.notify();
    }

    /// Catalog models currently shown in Settings / composer chip.
    #[allow(dead_code)]
    pub fn models(&self) -> &[ModelInfo] {
        &self.models
    }

    pub fn selected_model(&self) -> Option<&ModelInfo> {
        let id = self.selected_model_id.as_deref()?;
        self.models.iter().find(|m| m.id == id)
    }

    #[allow(dead_code)]
    pub fn selected_model_id(&self) -> Option<&str> {
        self.selected_model_id.as_deref()
    }

    /// Model slug for `TurnStartParams.model` (prefer `ModelInfo.model`, else id).
    pub fn selected_model_slug(&self) -> Option<String> {
        self.selected_model().map(|m| {
            if !m.model.trim().is_empty() {
                m.model.clone()
            } else {
                m.id.clone()
            }
        })
    }

    pub fn selected_reasoning_effort(&self) -> Option<&str> {
        self.selected_reasoning_effort.as_deref()
    }

    pub fn reasoning_effort_label(&self) -> Option<String> {
        self.selected_reasoning_effort()
            .map(reasoning_effort_display_name)
    }

    pub fn has_reasoning_effort_control(&self) -> bool {
        self.reasoning_options_for_selected_model().len() > 1
    }

    pub fn fast_mode_available(&self) -> bool {
        self.selected_model()
            .is_some_and(|model| !model.service_tiers.is_empty())
    }

    pub fn fast_mode_enabled(&self) -> bool {
        self.fast_mode_available() && self.selected_fast_mode
    }

    pub fn fast_mode_label(&self) -> String {
        self.selected_model()
            .and_then(|model| model.service_tiers.first())
            .map(|tier| tier.name.clone())
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| "Fast".to_owned())
    }

    pub fn toggle_fast_mode(&mut self, cx: &mut Context<Self>) {
        if !self.fast_mode_available() {
            self.status_line = "The selected model does not advertise a fast service tier.".into();
            cx.notify();
            return;
        }
        self.selected_fast_mode = !self.selected_fast_mode;
        if let (Some(backend), Some(model_id)) =
            (self.active_backend_kind(), self.selected_model_id.clone())
        {
            self.preferences
                .remember_fast(backend, &model_id, self.selected_fast_mode);
            self.save_preferences_best_effort();
        }
        self.status_line = if self.selected_fast_mode {
            format!("{} mode enabled.", self.fast_mode_label()).into()
        } else {
            "Standard response speed selected.".into()
        };
        cx.notify();
    }

    fn selected_speed_mode(&self) -> Option<ProductSpeedMode> {
        let backend = self.active_backend_kind()?;
        Some(match backend {
            BackendKind::CodexStdio | BackendKind::CodexWebSocket | BackendKind::Fixture => {
                if self.fast_mode_enabled() {
                    let tier = self.selected_model()?.service_tiers.first()?.id.clone();
                    ProductSpeedMode::CodexServiceTier(tier)
                } else {
                    ProductSpeedMode::CodexStandard
                }
            }
            BackendKind::MitsuroHttp => {
                if self.fast_mode_enabled() {
                    ProductSpeedMode::MitsuroFast
                } else {
                    ProductSpeedMode::MitsuroStandard
                }
            }
        })
    }

    fn reasoning_options_for_selected_model(&self) -> Vec<String> {
        let Some(model) = self.selected_model() else {
            return Vec::new();
        };
        let mut options = model
            .supported_reasoning_efforts
            .iter()
            .map(|effort| effort.reasoning_effort.trim())
            .filter(|effort| !effort.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        options.dedup();
        if options.is_empty() && !model.default_reasoning_effort.trim().is_empty() {
            options.push(model.default_reasoning_effort.trim().to_owned());
        }
        options
    }

    fn restore_reasoning_for_selected_model(&mut self) {
        let options = self.reasoning_options_for_selected_model();
        let Some(model_id) = self.selected_model_id.as_deref() else {
            self.selected_reasoning_effort = None;
            return;
        };
        let remembered = self
            .active_backend_kind()
            .and_then(|backend| self.preferences.reasoning_for(backend, model_id))
            .filter(|effort| options.iter().any(|option| option == effort))
            .map(str::to_owned);
        let default = self
            .selected_model()
            .map(|model| model.default_reasoning_effort.trim())
            .filter(|effort| options.iter().any(|option| option == effort))
            .map(str::to_owned);
        self.selected_reasoning_effort =
            remembered.or(default).or_else(|| options.first().cloned());
    }

    fn restore_speed_for_selected_model(&mut self) {
        let available = self.fast_mode_available();
        let default_enabled = self
            .selected_model()
            .and_then(|model| model.default_service_tier.as_ref())
            .is_some();
        let remembered = self
            .active_backend_kind()
            .zip(self.selected_model_id.as_deref())
            .and_then(|(backend, model_id)| self.preferences.fast_for(backend, model_id));
        self.selected_fast_mode = available && remembered.unwrap_or(default_enabled);
    }

    fn remember_selected_reasoning(&mut self) {
        let (Some(backend), Some(model_id), Some(effort)) = (
            self.active_backend_kind(),
            self.selected_model_id.as_deref(),
            self.selected_reasoning_effort.clone(),
        ) else {
            return;
        };
        self.preferences
            .remember_reasoning(backend, model_id, effort);
        self.save_preferences_best_effort();
    }

    /// Config snippet from `config/read` for Settings.
    pub fn config_snippet(&self) -> &SharedString {
        &self.config_snippet
    }

    /// Skills loaded via `skills/list` (or fixture demo).
    pub fn skills(&self) -> &[SkillMetadata] {
        &self.skills
    }

    #[allow(dead_code)]
    pub fn skills_enabled_count(&self) -> usize {
        self.skills.iter().filter(|s| s.enabled).count()
    }

    /// Apply a `model/list` result and pick default when selection is missing.
    fn apply_models(&mut self, models: Vec<ModelInfo>) {
        if models.is_empty() {
            return;
        }
        let remembered = self
            .active_backend_kind()
            .and_then(|kind| self.preferences.models_by_backend.get(&kind))
            .and_then(|id| models.iter().find(|model| &model.id == id))
            .map(|model| model.id.clone());
        let keep = self
            .selected_model_id
            .as_ref()
            .and_then(|id| models.iter().find(|m| &m.id == id).map(|m| m.id.clone()));
        let default_id = models
            .iter()
            .find(|m| m.is_default && !m.hidden)
            .or_else(|| models.iter().find(|m| !m.hidden))
            .or_else(|| models.first())
            .map(|m| m.id.clone());
        self.selected_model_id = remembered.or(keep).or(default_id);
        self.models = models;
        self.restore_reasoning_for_selected_model();
        self.restore_speed_for_selected_model();
    }

    fn apply_skills(&mut self, skills: Vec<SkillMetadata>) {
        self.skills = skills;
    }

    fn apply_mcp_servers(&mut self, servers: Vec<McpServerStatus>) {
        self.mcp_servers = servers;
    }

    fn apply_plugins(&mut self, plugins: Vec<PluginSummary>) {
        self.plugins = plugins;
    }

    /// MCP servers for the Extensions panel.
    pub fn mcp_servers(&self) -> &[McpServerStatus] {
        &self.mcp_servers
    }

    pub fn mcp_oauth_available(&self) -> bool {
        matches!(self.connection, UiConnection::Ready { .. })
            && self
                .backend
                .as_ref()
                .is_some_and(|backend| backend.capabilities().mcp_oauth)
    }

    pub fn mcp_oauth_pending(&self, name: &str) -> bool {
        self.pending_mcp_oauth.contains(name)
    }

    pub fn start_mcp_oauth(&mut self, server: McpServerStatus, cx: &mut Context<Self>) {
        if server.auth_status != McpAuthStatus::NotLoggedIn {
            self.status_line = format!("MCP · {} does not require OAuth login", server.name).into();
            cx.notify();
            return;
        }
        if self.pending_mcp_oauth.contains(&server.name) {
            return;
        }
        let Some(backend) = self.live_backend() else {
            self.status_line = "MCP OAuth requires a connected Codex app-server".into();
            cx.notify();
            return;
        };
        if !backend.capabilities().mcp_oauth {
            self.status_line = "MCP OAuth is unavailable for this backend".into();
            cx.notify();
            return;
        }
        let name = server.name;
        let generation = self.backend_generation;
        self.pending_mcp_oauth.insert(name.clone());
        self.status_line = format!("MCP · starting {name} sign-in…").into();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let request_name = name.clone();
            let result = cx
                .background_spawn(async move {
                    backend
                        .mcp_oauth_login(McpServerOauthLoginParams::new(request_name))
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != generation {
                    return;
                }
                match result {
                    Ok(response) => {
                        let opened = open_system_browser(&response.authorization_url);
                        app.status_line =
                            format!("MCP · {name} sign-in pending · {}", opened.summary()).into();
                    }
                    Err(error) => {
                        app.pending_mcp_oauth.remove(&name);
                        app.status_line = format!("MCP · {name} sign-in failed · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Flattened plugins for the Extensions panel.
    pub fn plugins(&self) -> &[PluginSummary] {
        &self.plugins
    }

    pub fn plugin_mutations_available(&self) -> bool {
        matches!(self.connection, UiConnection::Ready { .. })
            && self
                .backend
                .as_ref()
                .is_some_and(|backend| backend.capabilities().plugin_mutations)
    }

    pub fn plugin_mutation_id(&self) -> Option<&str> {
        self.plugin_mutation_in_progress.as_deref()
    }

    pub fn mutate_plugin(&mut self, plugin: PluginSummary, cx: &mut Context<Self>) {
        if self.plugin_mutation_in_progress.is_some() {
            self.status_line = "Plugins · another change is still in progress".into();
            cx.notify();
            return;
        }
        if plugin.availability != mitsuro_desktop_backend::PluginAvailability::Available
            || plugin.install_policy != mitsuro_desktop_backend::PluginInstallPolicy::Available
        {
            self.status_line = "Plugins · this plugin is managed by its marketplace policy".into();
            cx.notify();
            return;
        }
        let Some(backend) = self.live_backend() else {
            self.status_line =
                "Plugins · install and removal require a connected Codex app-server".into();
            cx.notify();
            return;
        };
        if !backend.capabilities().plugin_mutations {
            self.status_line =
                "Plugins · this backend exposes inventory but not install or removal".into();
            cx.notify();
            return;
        }

        let generation = self.backend_generation;
        let plugin_id = plugin.id.clone();
        let plugin_name = plugin.name.clone();
        let marketplace_path = plugin
            .extra
            .get("marketplacePath")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let remote_marketplace_name = plugin
            .extra
            .get("remoteMarketplaceName")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let display_name = plugin.display_name().to_owned();
        let uninstalling = plugin.installed;
        self.plugin_mutation_in_progress = Some(plugin_id.clone());
        self.status_line = format!(
            "Plugins · {} {display_name}…",
            if uninstalling {
                "removing"
            } else {
                "installing"
            }
        )
        .into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if uninstalling {
                        backend
                            .uninstall_plugin(PluginUninstallParams { plugin_id })
                            .await
                            .map(|_| None)
                    } else {
                        backend
                            .install_plugin(PluginInstallParams {
                                plugin_name,
                                marketplace_path,
                                remote_marketplace_name,
                            })
                            .await
                            .map(Some)
                    }
                    .map_err(|error| error.to_string())
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != generation {
                    return;
                }
                app.plugin_mutation_in_progress = None;
                match result {
                    Ok(Some(response)) => {
                        let auth_note = if response.apps_needing_auth.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " · {} app(s) require authentication",
                                response.apps_needing_auth.len()
                            )
                        };
                        app.status_line =
                            format!("Plugins · installed {display_name}{auth_note}").into();
                        app.kick_extensions_refresh(cx);
                    }
                    Ok(None) => {
                        app.status_line = format!("Plugins · removed {display_name}").into();
                        app.kick_extensions_refresh(cx);
                    }
                    Err(error) => {
                        app.status_line = format!(
                            "Plugins · could not {} {display_name} · {error}",
                            if uninstalling { "remove" } else { "install" }
                        )
                        .into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Environments for the Computer panel.
    pub fn environments(&self) -> &[EnvironmentSummary] {
        &self.environments
    }

    pub fn environment_id_input(&self) -> &Entity<InputState> {
        &self.environment_id_input
    }

    pub fn environment_url_input(&self) -> &Entity<InputState> {
        &self.environment_url_input
    }

    pub fn environment_add_available(&self) -> bool {
        matches!(self.connection, UiConnection::Ready { .. })
            && self
                .backend
                .as_ref()
                .is_some_and(|backend| backend.capabilities().environment_add)
    }

    pub fn environment_add_in_progress(&self) -> bool {
        self.environment_add_in_progress
    }

    pub fn add_environment(&mut self, cx: &mut Context<Self>) {
        if self.environment_add_in_progress {
            return;
        }
        let Some(backend) = self.live_backend() else {
            self.status_line =
                "Computer · environment registration requires Codex app-server".into();
            cx.notify();
            return;
        };
        if !backend.capabilities().environment_add {
            self.status_line =
                "Computer · this backend does not expose remote environment registration".into();
            cx.notify();
            return;
        }

        let environment_id = self.environment_id_input.read(cx).value().trim().to_owned();
        let exec_server_url = self
            .environment_url_input
            .read(cx)
            .value()
            .trim()
            .to_owned();
        if environment_id.is_empty() {
            self.status_line = "Computer · enter an environment id".into();
            cx.notify();
            return;
        }
        if !valid_exec_server_url(&exec_server_url) {
            self.status_line = "Computer · exec-server URL must be ws:// or wss://".into();
            cx.notify();
            return;
        }

        let params = EnvironmentAddParams::new(environment_id.clone(), exec_server_url);
        let summary = mitsuro_desktop_backend::registered_environment_summary(&params);
        let generation = self.backend_generation;
        self.environment_add_in_progress = true;
        self.status_line = format!("Computer · adding {environment_id}…").into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    backend
                        .environment_add(params)
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != generation {
                    return;
                }
                app.environment_add_in_progress = false;
                match result {
                    Ok(_) => {
                        if let Some(existing) = app
                            .environments
                            .iter_mut()
                            .find(|environment| environment.id == summary.id)
                        {
                            *existing = summary.clone();
                        } else {
                            app.environments.push(summary.clone());
                        }
                        app.selected_environment_id = Some(summary.id.clone());
                        app.environments_state = SurfaceDataState::Live;
                        app.status_line =
                            format!("Computer · added {} · checking status", summary.id).into();
                        app.refresh_selected_environment_detail(cx);
                    }
                    Err(error) => {
                        app.status_line =
                            format!("Computer · could not add {environment_id} · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn selected_environment_id(&self) -> Option<&str> {
        self.selected_environment_id.as_deref()
    }

    pub fn environment_status_detail(&self) -> Option<&EnvironmentStatusResponse> {
        self.environment_status_detail.as_ref()
    }

    pub fn environment_info_detail(&self) -> Option<&EnvironmentInfoResponse> {
        self.environment_info_detail.as_ref()
    }

    pub fn collaboration_modes(&self) -> &[CollaborationModeMask] {
        &self.collaboration_modes
    }

    pub fn work_mode_available(&self) -> bool {
        match self.active_backend_kind() {
            Some(BackendKind::MitsuroHttp) => true,
            Some(BackendKind::CodexStdio)
            | Some(BackendKind::CodexWebSocket)
            | Some(BackendKind::Fixture) => {
                self.collaboration_modes
                    .iter()
                    .any(|preset| preset.mode == Some(ModeKind::Plan))
                    && self
                        .collaboration_modes
                        .iter()
                        .any(|preset| preset.mode == Some(ModeKind::Default))
            }
            None => false,
        }
    }

    pub fn work_mode_label(&self) -> &'static str {
        if self.composer_plan_mode {
            "Plan"
        } else if self.active_backend_kind() == Some(BackendKind::MitsuroHttp) {
            "Build"
        } else {
            "Default"
        }
    }

    pub fn toggle_work_mode(&mut self, cx: &mut Context<Self>) {
        if !self.work_mode_available() {
            self.status_line = "The active backend did not advertise Plan/Default modes.".into();
            cx.notify();
            return;
        }
        self.composer_plan_mode = !self.composer_plan_mode;
        if let Some(backend) = self.active_backend_kind() {
            self.preferences
                .remember_plan_mode(backend, self.composer_plan_mode);
            self.save_preferences_best_effort();
        }
        self.status_line = format!("Work mode: {}", self.work_mode_label()).into();
        cx.notify();
    }

    fn selected_work_mode(&self) -> Option<ProductWorkMode> {
        match self.active_backend_kind()? {
            BackendKind::MitsuroHttp => Some(if self.composer_plan_mode {
                ProductWorkMode::MitsuroPlan
            } else {
                ProductWorkMode::MitsuroBuild
            }),
            BackendKind::CodexStdio | BackendKind::CodexWebSocket | BackendKind::Fixture => {
                let mode = if self.composer_plan_mode {
                    ModeKind::Plan
                } else {
                    ModeKind::Default
                };
                let preset = self
                    .collaboration_modes
                    .iter()
                    .find(|preset| preset.mode == Some(mode))?;
                let model = preset
                    .model
                    .as_deref()
                    .filter(|model| !model.trim().is_empty())
                    .map(str::to_owned)
                    .or_else(|| self.selected_model_slug())?;
                Some(ProductWorkMode::Codex {
                    mode,
                    model,
                    reasoning_effort: preset
                        .reasoning_effort
                        .clone()
                        .or_else(|| self.selected_reasoning_effort.clone()),
                })
            }
        }
    }

    pub fn select_environment(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_environment_id = Some(id);
        self.environment_status_detail = None;
        self.environment_info_detail = None;
        self.refresh_selected_environment_detail(cx);
        cx.notify();
    }

    /// Reload environment catalog + collaboration modes (fixture offline).
    pub fn refresh_environments(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let fixture = self.fixture.clone();
        let backend = self.backend.clone();
        let use_live = matches!(self.connection, UiConnection::Ready { .. }) && backend.is_some();
        let use_fixture = self.is_explicit_fixture();
        self.status_line = "Computer · refreshing environments…".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if use_live {
                        if let Some(backend) = backend {
                            // Neither live transport exposes environment/list. Keep the
                            // catalog empty and refresh only the independently typed modes.
                            let modes = match backend
                                .collaboration_mode_list(CollaborationModeListParams::default())
                                .await
                            {
                                Ok(r) => r.data,
                                Err(_) => Vec::new(),
                            };
                            return Ok::<_, String>((
                                Vec::new(),
                                modes,
                                "app-server",
                                SurfaceDataState::Unsupported,
                            ));
                        }
                    }
                    if !use_fixture {
                        return Err(
                            "environment catalog is unavailable for this backend state".into()
                        );
                    }
                    let fixture = fixture.unwrap_or_else(|| Arc::new(FixtureBackend::new()));
                    if !fixture.status().is_usable() {
                        fixture.connect().await.map_err(|e| e.to_string())?;
                    }
                    let catalog = fixture.environment_catalog();
                    let modes = fixture
                        .collaboration_mode_list(CollaborationModeListParams::default())
                        .await
                        .map_err(|e| e.to_string())?
                        .data;
                    Ok((catalog, modes, "fixture", SurfaceDataState::Fixture))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((envs, modes, label, state)) => {
                        app.environments = envs;
                        app.environments_state = state;
                        app.collaboration_modes = modes;
                        if app
                            .selected_environment_id
                            .as_ref()
                            .map(|id| !app.environments.iter().any(|e| &e.id == id))
                            .unwrap_or(true)
                        {
                            app.selected_environment_id =
                                app.environments.first().map(|e| e.id.clone());
                        }
                        let n = app.environments.len();
                        let connected =
                            app.environments.iter().filter(|e| e.is_connected()).count();
                        app.status_line =
                            format!("Computer · {label} · {n} env(s) · {connected} connected")
                                .into();
                    }
                    Err(message) => {
                        app.environments.clear();
                        app.collaboration_modes.clear();
                        app.selected_environment_id = None;
                        app.environments_state = SurfaceDataState::Error;
                        app.status_line = format!("Computer refresh failed · {message}").into();
                    }
                }
                app.environment_status_detail = None;
                app.environment_info_detail = None;
                app.refresh_selected_environment_detail(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Fetch `environment/status` + `environment/info` for the selected id.
    pub fn refresh_selected_environment_detail(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected_environment_id.clone() else {
            return;
        };
        let fixture = self.fixture.clone();
        let backend = self.backend.clone();
        let use_live = matches!(self.connection, UiConnection::Ready { .. }) && backend.is_some();
        let use_fixture = self.is_explicit_fixture();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if use_live {
                        if let Some(backend) = backend {
                            let status = backend
                                .environment_status(EnvironmentStatusParams::new(id.clone()))
                                .await
                                .ok();
                            let info = backend
                                .environment_info(EnvironmentInfoParams::new(id.clone()))
                                .await
                                .ok();
                            return Ok::<_, String>((status, info, "app-server"));
                        }
                    }
                    if !use_fixture {
                        return Err(
                            "environment detail is unavailable for this backend state".into()
                        );
                    }
                    let fixture = fixture.unwrap_or_else(|| Arc::new(FixtureBackend::new()));
                    if !fixture.status().is_usable() {
                        fixture.connect().await.map_err(|e| e.to_string())?;
                    }
                    let status = fixture
                        .environment_status(EnvironmentStatusParams::new(id.clone()))
                        .await
                        .ok();
                    let info = fixture
                        .environment_info(EnvironmentInfoParams::new(id))
                        .await
                        .ok();
                    Ok((status, info, "fixture"))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((status, info, _)) => {
                        app.environment_status_detail = status;
                        app.environment_info_detail = info;
                    }
                    Err(_) => {
                        // Fall back to catalog row fields.
                        if let Some(entry) = app
                            .selected_environment_id
                            .as_ref()
                            .and_then(|sid| app.environments.iter().find(|e| &e.id == sid))
                        {
                            app.environment_status_detail = Some(EnvironmentStatusResponse {
                                status: entry.status,
                                error: entry.error.clone(),
                            });
                            if let Some(shell) = entry.shell.clone() {
                                app.environment_info_detail = Some(EnvironmentInfoResponse {
                                    shell,
                                    cwd: entry.cwd.clone(),
                                });
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Refresh MCP + plugin + skills lists (live app-server when Ready, else fixture).
    pub fn refresh_extensions(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_extensions_data(true, cx);
    }

    /// Refresh extension state without replacing lifecycle status chrome.
    fn kick_extensions_refresh(&mut self, cx: &mut Context<Self>) {
        self.refresh_extensions_data(false, cx);
    }

    fn refresh_extensions_data(&mut self, announce: bool, cx: &mut Context<Self>) {
        let fixture = self.fixture.clone();
        let backend = self.live_backend();
        let use_live = backend.is_some();
        let use_fixture = self.is_explicit_fixture();
        let was_live = use_live;
        if announce {
            self.status_line = "Extensions · refreshing…".into();
            cx.notify();
        }

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if use_live {
                        if let Some(backend) = backend {
                            let mut errors = Vec::new();
                            let mcp = match backend.list_product_mcp_servers().await {
                                Ok(servers) => {
                                    servers.into_iter().map(mcp_status_from_product).collect()
                                }
                                Err(error) => {
                                    errors.push(format!("MCP: {error}"));
                                    Vec::new()
                                }
                            };
                            let plugins = match backend.list_product_extensions().await {
                                Ok(extensions) => extensions
                                    .into_iter()
                                    .map(plugin_summary_from_product)
                                    .collect(),
                                Err(error) => {
                                    errors.push(format!("plugins: {error}"));
                                    Vec::new()
                                }
                            };
                            let skills = match backend.list_product_skills().await {
                                Ok(skills) => skills
                                    .into_iter()
                                    .map(skill_metadata_from_product)
                                    .collect(),
                                Err(error) => {
                                    errors.push(format!("skills: {error}"));
                                    Vec::new()
                                }
                            };
                            return Ok::<_, String>((mcp, plugins, skills, "app-server", errors));
                        }
                    }
                    if !use_fixture {
                        return Err(
                            "extension catalog is unavailable for this backend state".into()
                        );
                    }
                    let fixture = fixture.unwrap_or_else(|| Arc::new(FixtureBackend::new()));
                    if !fixture.status().is_usable() {
                        fixture.connect().await.map_err(|e| e.to_string())?;
                    }
                    let mcp = fixture
                        .mcp_server_status_list(ListMcpServerStatusParams::default())
                        .await
                        .map_err(|e| e.to_string())?
                        .data;
                    let plugins = fixture
                        .plugin_list(PluginListParams::default())
                        .await
                        .map_err(|e| e.to_string())?
                        .marketplaces
                        .into_iter()
                        .flat_map(|m| m.plugins)
                        .collect::<Vec<_>>();
                    let skills = fixture
                        .skills_list(SkillsListParams::default())
                        .await
                        .map_err(|e| e.to_string())?
                        .data
                        .into_iter()
                        .flat_map(|e| e.skills)
                        .collect::<Vec<_>>();
                    Ok((mcp, plugins, skills, "fixture", Vec::new()))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((mcp, plugins, skills, label, errors)) => {
                        let mcp_empty = mcp.is_empty();
                        let plugins_empty = plugins.is_empty();
                        app.apply_mcp_servers(mcp);
                        app.apply_plugins(plugins);
                        app.apply_skills(skills);
                        app.extensions_state = if label == "fixture" {
                            SurfaceDataState::Fixture
                        } else if errors.is_empty() {
                            SurfaceDataState::Live
                        } else {
                            SurfaceDataState::Error
                        };
                        let empty_note = if !errors.is_empty() {
                            format!("app-server partial · {}", errors.join("; "))
                        } else if label == "app-server" && mcp_empty && plugins_empty {
                            "app-server · empty catalog".to_string()
                        } else {
                            label.to_string()
                        };
                        if announce {
                            app.status_line = format!(
                                "Extensions · {empty_note} · {} MCP · {} plugin(s) · {} skill(s)",
                                app.mcp_servers.len(),
                                app.plugins.len(),
                                app.skills.len()
                            )
                            .into();
                        }
                    }
                    Err(message) => {
                        app.apply_mcp_servers(Vec::new());
                        app.apply_plugins(Vec::new());
                        app.apply_skills(Vec::new());
                        app.extensions_state = if was_live {
                            SurfaceDataState::Error
                        } else {
                            SurfaceDataState::Fixture
                        };
                        if announce {
                            app.status_line =
                                format!("Extensions refresh failed · {message}").into();
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_config_snippet(&mut self, snippet: String) {
        if !snippet.trim().is_empty() {
            self.config_snippet = snippet.into();
        }
    }

    pub fn browser_session(&self) -> &BrowserSession {
        &self.browser
    }

    pub fn browser_url_input(&self) -> &Entity<InputState> {
        &self.browser_url_input
    }

    fn browser_host_kind_label(&self) -> String {
        #[cfg(feature = "browser-native")]
        {
            return self.native_host.host_kind_label();
        }
        #[cfg(not(feature = "browser-native"))]
        {
            self.browser_host.host_kind().to_string()
        }
    }

    fn bridge_fields(&self) -> (SharedString, SharedString, Option<SharedString>) {
        #[cfg(feature = "browser-native")]
        {
            let mode = SharedString::from(self.native_host.bridge_mode().label());
            let detail = SharedString::from(self.native_host.report().detail.clone());
            let kind = SharedString::from(self.native_host.host_kind_label());
            (mode, detail, Some(kind))
        }
        #[cfg(not(feature = "browser-native"))]
        {
            (
                SharedString::from("External"),
                SharedString::from(
                    "System browser owns page content; Mitsuro keeps URL history only",
                ),
                None,
            )
        }
    }

    fn sync_browser_session(&mut self) {
        let (bridge_mode, bridge_detail, host_kind_override) = self.bridge_fields();
        self.browser = BrowserSession::from_host(
            &self.browser_host,
            bridge_detail,
            bridge_mode,
            host_kind_override,
        );
    }

    fn sync_url_bar_from_host(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.browser_host.url().to_string();
        self.browser_url_input.update(cx, |state, cx| {
            state.set_value(url, window, cx);
        });
    }

    /// Probe GPUI raw window handle and optionally try wry child embed.
    pub fn browser_request_attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(feature = "browser-native")]
        {
            self.native_host.attach_after_window_open(window);
            self.sync_browser_session();
            let detail = self.native_host.report().detail.clone();
            self.status_line = format!("Atlas attach · {detail}").into();
            cx.notify();
        }
        #[cfg(not(feature = "browser-native"))]
        {
            let _ = window;
            self.status_line = "Atlas · mock host (browser-native off)".into();
            cx.notify();
        }
    }

    /// Navigate from the URL bar (Go / Enter). Updates mock history + optional bridge/WebView.
    pub fn browser_navigate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.browser_url_input.read(cx).value().to_string();
        if raw.trim().is_empty() {
            self.status_line = "Browser · empty URL".into();
            cx.notify();
            return;
        }
        // Ensure handle probe has run before first navigate on Atlas.
        #[cfg(feature = "browser-native")]
        {
            if !self.native_host.is_attached() {
                self.native_host.attach_after_window_open(window);
            }
        }
        self.browser_host.navigate(&raw);
        let url = self.browser_host.url().to_string();

        #[cfg(feature = "browser-native")]
        let nav_note = {
            let out = self.native_host.navigate(&url);
            out.summary
        };
        #[cfg(not(feature = "browser-native"))]
        let nav_note = "mock history".to_string();

        self.sync_browser_session();
        self.sync_url_bar_from_host(window, cx);
        self.status_line = format!("Navigated · {url} · {nav_note}").into();
        cx.notify();
    }

    /// Open current Atlas URL in the system browser (or Chromium --app sibling).
    pub fn browser_open_external(&mut self, cx: &mut Context<Self>) {
        let url = self.browser_host.url().to_string();
        #[cfg(feature = "browser-native")]
        let result = self.native_host.open_bridge(&url);
        #[cfg(not(feature = "browser-native"))]
        let result = open_system_browser(&url);

        self.sync_browser_session();
        self.status_line = format!("Open external · {} · {}", url, result.summary()).into();
        cx.notify();
    }

    /// Back navigation via host history.
    pub fn browser_go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.browser_host.go_back() {
            self.status_line = "Browser back · no history".into();
            cx.notify();
            return;
        }
        let url = self.browser_host.url().to_string();
        #[cfg(feature = "browser-native")]
        {
            let _ = self.native_host.navigate(&url);
        }
        self.sync_browser_session();
        self.sync_url_bar_from_host(window, cx);
        self.status_line = format!("Back · {url}").into();
        cx.notify();
    }

    /// Forward navigation via host history.
    pub fn browser_go_forward(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.browser_host.go_forward() {
            self.status_line = "Browser forward · no history".into();
            cx.notify();
            return;
        }
        let url = self.browser_host.url().to_string();
        #[cfg(feature = "browser-native")]
        {
            let _ = self.native_host.navigate(&url);
        }
        self.sync_browser_session();
        self.sync_url_bar_from_host(window, cx);
        self.status_line = format!("Forward · {url}").into();
        cx.notify();
    }

    pub fn threads(&self) -> &[DemoThread] {
        &self.threads
    }

    pub fn selected_thread_id(&self) -> Option<&str> {
        self.selected_thread.as_deref()
    }

    pub fn selected_thread(&self) -> Option<&DemoThread> {
        let id = self.selected_thread.as_ref()?;
        self.threads.iter().find(|t| &t.summary.id == id)
    }

    pub fn transcript_visible_limit(&self) -> usize {
        self.selected_thread
            .as_ref()
            .and_then(|id| self.transcript_visible_limits.get(id).copied())
            .unwrap_or(16)
    }

    pub fn show_earlier_transcript_messages(
        &mut self,
        total_messages: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(thread_id) = self.selected_thread.clone() else {
            return;
        };
        let visible = self
            .transcript_visible_limits
            .entry(thread_id)
            .or_insert(16);
        *visible = visible.saturating_add(16).min(total_messages);
        self.status_line = format!("Transcript · showing {} of {total_messages}", *visible).into();
        cx.notify();
    }

    pub fn transcript_message_is_expanded(&self, key: &str) -> bool {
        self.expanded_transcript_messages.contains(key)
    }

    pub fn transcript_scroll_handle(&self) -> &ScrollHandle {
        &self.transcript_scroll_handle
    }

    pub fn toggle_transcript_message_expanded(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.expanded_transcript_messages.remove(&key) {
            self.expanded_transcript_messages.insert(key);
        }
        cx.notify();
    }

    pub fn search_query(&self) -> &str {
        &self.search_query
    }

    /// Local sidebar filter: title, preview, cwd (case-insensitive substring).
    pub fn thread_matches_search(&self, summary: &ThreadSummary) -> bool {
        let filter = self.search_query.trim().to_lowercase();
        if filter.is_empty() {
            return true;
        }
        let title = summary.display_title().to_lowercase();
        let preview = summary.preview.as_deref().unwrap_or("").to_lowercase();
        let cwd = summary.cwd.as_deref().unwrap_or("").to_lowercase();
        let meta = demo::meta_line(summary).to_lowercase();
        title.contains(&filter)
            || preview.contains(&filter)
            || cwd.contains(&filter)
            || meta.contains(&filter)
    }

    pub fn model_label(&self) -> SharedString {
        self.selected_model()
            .map(|m| SharedString::from(m.label().to_string()))
            .unwrap_or_else(|| SharedString::from("No model"))
    }

    /// Account session for Settings Account section.
    pub fn account_session(&self) -> &AccountSession {
        &self.account
    }

    /// Human-readable account status: Signed out / Fixture demo / Ready.
    pub fn account_status_label(&self) -> SharedString {
        match &self.connection {
            UiConnection::Fixture | UiConnection::Demo => {
                if self.account.signed_in {
                    let email = self.account.email_display.as_deref().unwrap_or("demo");
                    let plan = self.account.plan_label.as_deref().unwrap_or("Pro");
                    format!("Fixture demo · {email} · {plan}").into()
                } else {
                    "Signed out · fixture".into()
                }
            }
            UiConnection::Connecting => "Connecting…".into(),
            UiConnection::Ready { has_auth: true, .. } => {
                let email = self
                    .account
                    .email_display
                    .as_deref()
                    .unwrap_or("authenticated");
                let plan = self.account.plan_label.as_deref().unwrap_or("plan unknown");
                format!("Ready · {email} · {plan}").into()
            }
            UiConnection::Ready {
                has_auth: false, ..
            } => "Signed out · no account (account/read)".into(),
            UiConnection::Error { message } => format!("Error · {message}").into(),
        }
    }

    /// Human-readable auth line for Settings (account/read when Ready).
    #[allow(dead_code)]
    pub fn auth_status_label(&self) -> SharedString {
        self.account_status_label()
    }

    /// Apply account snapshot from protocol responses.
    fn apply_account_snapshot(
        &mut self,
        account: Option<Account>,
        usage: GetAccountTokenUsageResponse,
        rate_limits: GetAccountRateLimitsResponse,
        source: &'static str,
        login_detail: Option<String>,
    ) {
        let signed_in = account.as_ref().is_some_and(|a| a.is_signed_in());
        let email_display = account.as_ref().and_then(|a| a.email_display());
        let plan_label = account
            .as_ref()
            .and_then(|a| a.plan_type())
            .map(|p: PlanType| p.label().to_string());
        let previous_detail = self.account.login_detail.clone();
        let pending_login_id = self.account.pending_login_id.clone();
        let pending_login_url = self.account.pending_login_url.clone();
        self.account = AccountSession {
            signed_in,
            email_display,
            plan_label,
            usage,
            rate_limits,
            login_detail: login_detail.or(previous_detail),
            pending_login_id,
            pending_login_url,
            source,
        };
        if let UiConnection::Ready { has_auth, .. } = &mut self.connection {
            *has_auth = signed_in;
        }
    }

    /// Refresh account + usage + rate limits (no Window required).
    fn kick_account_refresh(&mut self, cx: &mut Context<Self>) {
        let fixture = self.fixture.clone();
        let backend = self.backend.clone();
        let generation = self.backend_generation;
        let use_live = matches!(self.connection, UiConnection::Ready { .. }) && backend.is_some();
        let use_fixture = self.is_explicit_fixture();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if use_live {
                        if let Some(backend) = backend {
                            if backend.kind() == BackendKind::MitsuroHttp {
                                let empty = AccountSession::empty("mitsuro-http");
                                return Ok::<_, String>((
                                    None,
                                    empty.usage,
                                    empty.rate_limits,
                                    "mitsuro-http",
                                    SurfaceDataState::Unsupported,
                                ));
                            }
                            let acc = backend
                                .account_read(GetAccountParams::default())
                                .await
                                .map_err(|error| format!("account/read: {error}"))?;
                            let (usage, limits) = if acc.has_account() {
                                let usage = backend
                                    .account_usage_read()
                                    .await
                                    .map_err(|error| format!("account/usage/read: {error}"))?;
                                let limits = backend
                                    .account_rate_limits_read()
                                    .await
                                    .map_err(|error| format!("account/rateLimits/read: {error}"))?;
                                (usage, limits)
                            } else {
                                let empty = AccountSession::empty("app-server");
                                (empty.usage, empty.rate_limits)
                            };
                            return Ok::<_, String>((
                                acc.account,
                                usage,
                                limits,
                                "app-server",
                                SurfaceDataState::Live,
                            ));
                        }
                    }
                    if !use_fixture {
                        return Err("account data is unavailable for this backend state".into());
                    }
                    let fixture = fixture.unwrap_or_else(|| Arc::new(FixtureBackend::new()));
                    if !fixture.status().is_usable() {
                        fixture.connect().await.map_err(|e| e.to_string())?;
                    }
                    let acc = fixture
                        .account_read(GetAccountParams::default())
                        .await
                        .map_err(|e| e.to_string())?;
                    let usage = fixture
                        .account_usage_read()
                        .await
                        .map_err(|e| e.to_string())?;
                    let limits = fixture
                        .account_rate_limits_read()
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok((
                        acc.account,
                        usage,
                        limits,
                        "fixture",
                        SurfaceDataState::Fixture,
                    ))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != generation {
                    return;
                }
                match result {
                    Ok((account, usage, limits, source, state)) => {
                        app.apply_account_snapshot(account, usage, limits, source, None);
                        app.account_state = state;
                    }
                    Err(_) => {
                        let source = app
                            .active_backend_kind()
                            .map(BackendKind::id)
                            .unwrap_or("unavailable");
                        app.account = AccountSession::empty(source);
                        app.account_state = SurfaceDataState::Error;
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Refresh account + usage + rate limits (fixture or live; never paid models).
    pub fn refresh_account(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.status_line = "Account · refreshing…".into();
        cx.notify();
        let fixture = self.fixture.clone();
        let backend = self.backend.clone();
        let use_live = matches!(self.connection, UiConnection::Ready { .. }) && backend.is_some();
        let use_fixture = self.is_explicit_fixture();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if use_live {
                        if let Some(backend) = backend {
                            if backend.kind() == BackendKind::MitsuroHttp {
                                let empty = AccountSession::empty("mitsuro-http");
                                return Ok::<_, String>((
                                    None,
                                    empty.usage,
                                    empty.rate_limits,
                                    "mitsuro-http",
                                    SurfaceDataState::Unsupported,
                                ));
                            }
                            let acc = backend
                                .account_read(GetAccountParams::default())
                                .await
                                .map_err(|error| format!("account/read: {error}"))?;
                            let (usage, limits) = if acc.has_account() {
                                let usage = backend
                                    .account_usage_read()
                                    .await
                                    .map_err(|error| format!("account/usage/read: {error}"))?;
                                let limits = backend
                                    .account_rate_limits_read()
                                    .await
                                    .map_err(|error| format!("account/rateLimits/read: {error}"))?;
                                (usage, limits)
                            } else {
                                let empty = AccountSession::empty("app-server");
                                (empty.usage, empty.rate_limits)
                            };
                            return Ok::<_, String>((
                                acc.account,
                                usage,
                                limits,
                                "app-server",
                                SurfaceDataState::Live,
                            ));
                        }
                    }
                    if !use_fixture {
                        return Err("account data is unavailable for this backend state".into());
                    }
                    let fixture = fixture.unwrap_or_else(|| Arc::new(FixtureBackend::new()));
                    if !fixture.status().is_usable() {
                        fixture.connect().await.map_err(|e| e.to_string())?;
                    }
                    let acc = fixture
                        .account_read(GetAccountParams::default())
                        .await
                        .map_err(|e| e.to_string())?;
                    let usage = fixture
                        .account_usage_read()
                        .await
                        .map_err(|e| e.to_string())?;
                    let limits = fixture
                        .account_rate_limits_read()
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok((
                        acc.account,
                        usage,
                        limits,
                        "fixture",
                        SurfaceDataState::Fixture,
                    ))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((account, usage, limits, source, state)) => {
                        app.apply_account_snapshot(account, usage, limits, source, None);
                        app.account_state = state;
                        app.status_line = format!(
                            "Account · {} · {}",
                            source,
                            if app.account.signed_in {
                                "signed in"
                            } else {
                                "signed out"
                            }
                        )
                        .into();
                    }
                    Err(message) => {
                        let source = app
                            .active_backend_kind()
                            .map(BackendKind::id)
                            .unwrap_or("unavailable");
                        app.account = AccountSession::empty(source);
                        app.account_state = SurfaceDataState::Error;
                        app.status_line = format!("Account refresh failed · {message}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Start the real Codex browser login flow (or the explicit fixture flow).
    pub fn account_sign_in(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.account.pending_login_id.is_some() {
            self.account_open_sign_in(cx);
            return;
        }
        let fixture = self.fixture.clone();
        let backend = self.backend.clone();
        let use_live = matches!(self.connection, UiConnection::Ready { .. }) && backend.is_some();
        let use_fixture = self.is_explicit_fixture();
        self.status_line = "Account · starting sign-in…".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            enum SignInResult {
                Pending {
                    login_id: String,
                    url: String,
                    user_code: Option<String>,
                },
                Fixture(Box<FixtureSignIn>),
            }

            struct FixtureSignIn {
                account: Option<Account>,
                usage: GetAccountTokenUsageResponse,
                limits: GetAccountRateLimitsResponse,
                detail: String,
            }

            let result = cx
                .background_spawn(async move {
                    if use_live {
                        if let Some(backend) = backend {
                            if backend.kind() == BackendKind::MitsuroHttp {
                                return Err(
                                    "account login is not exposed by the Mitsuro server".into()
                                );
                            }
                            match backend
                                .account_login_start(LoginAccountParams::chatgpt())
                                .await
                            {
                                Ok(login) => {
                                    let login_id = login.login_id().ok_or_else(|| {
                                        "login/start response is missing loginId".to_owned()
                                    })?;
                                    let url = login.device_url().ok_or_else(|| {
                                        "login/start response is missing authUrl".to_owned()
                                    })?;
                                    return Ok(SignInResult::Pending {
                                        login_id: login_id.to_owned(),
                                        url: url.to_owned(),
                                        user_code: login.user_code().map(str::to_owned),
                                    });
                                }
                                Err(e) => {
                                    return Err(format!("login/start: {e}"));
                                }
                            }
                        }
                    }
                    if !use_fixture {
                        return Err("account login is unavailable for this backend state".into());
                    }
                    let fixture = fixture.unwrap_or_else(|| Arc::new(FixtureBackend::new()));
                    if !fixture.status().is_usable() {
                        fixture.connect().await.map_err(|e| e.to_string())?;
                    }
                    let login = fixture
                        .account_login_start(LoginAccountParams::device_code())
                        .await
                        .map_err(|e| e.to_string())?;
                    let detail = format!(
                        "stub · {} · code {}",
                        login
                            .device_url()
                            .unwrap_or(mitsuro_desktop_backend::FIXTURE_LOGIN_VERIFICATION_URL),
                        login
                            .user_code()
                            .unwrap_or(mitsuro_desktop_backend::FIXTURE_LOGIN_USER_CODE)
                    );
                    let acc = fixture
                        .account_read(GetAccountParams::default())
                        .await
                        .map_err(|e| e.to_string())?
                        .account;
                    let usage = fixture
                        .account_usage_read()
                        .await
                        .map_err(|e| e.to_string())?;
                    let limits = fixture
                        .account_rate_limits_read()
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok(SignInResult::Fixture(Box::new(FixtureSignIn {
                        account: acc,
                        usage,
                        limits,
                        detail,
                    })))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok(SignInResult::Pending {
                        login_id,
                        url,
                        user_code,
                    }) => {
                        let open_result = open_system_browser(&url);
                        let code = user_code
                            .map(|code| format!(" · code {code}"))
                            .unwrap_or_default();
                        app.account.pending_login_id = Some(login_id);
                        app.account.pending_login_url = Some(url.clone());
                        app.account.login_detail =
                            Some(format!("Waiting for browser sign-in · {url}{code}"));
                        app.account.source = "app-server";
                        app.account_state = SurfaceDataState::Live;
                        app.status_line =
                            format!("Account · sign-in pending · {}", open_result.summary()).into();
                    }
                    Ok(SignInResult::Fixture(fixture)) => {
                        app.apply_account_snapshot(
                            fixture.account,
                            fixture.usage,
                            fixture.limits,
                            "fixture",
                            Some(fixture.detail.clone()),
                        );
                        app.account_state = SurfaceDataState::Fixture;
                        app.status_line =
                            format!("Account · fixture signed in · {}", fixture.detail).into();
                    }
                    Err(message) => {
                        app.account.login_detail = Some(format!("Sign-in failed · {message}"));
                        app.status_line = format!("Account sign-in failed · {message}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
        let _ = window;
    }

    pub fn account_open_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(url) = self.account.pending_login_url.clone() else {
            self.status_line = "Account · no pending sign-in URL".into();
            cx.notify();
            return;
        };
        let result = open_system_browser(&url);
        self.status_line = format!("Account · {}", result.summary()).into();
        cx.notify();
    }

    pub fn account_cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        let Some(login_id) = self.account.pending_login_id.clone() else {
            self.status_line = "Account · no pending sign-in".into();
            cx.notify();
            return;
        };
        let Some(backend) = self.live_backend() else {
            self.status_line = "Account · cannot cancel while backend is unavailable".into();
            cx.notify();
            return;
        };
        if backend.kind() == BackendKind::MitsuroHttp {
            self.status_line = "Account login is not exposed by the Mitsuro server.".into();
            cx.notify();
            return;
        }
        let generation = self.backend_generation;
        self.status_line = "Account · canceling sign-in…".into();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    backend
                        .account_login_cancel(CancelLoginAccountParams::new(login_id))
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != generation {
                    return;
                }
                match result {
                    Ok(response) => {
                        app.account.pending_login_id = None;
                        app.account.pending_login_url = None;
                        app.account.login_detail = None;
                        app.status_line = match response.status {
                            CancelLoginAccountStatus::Canceled => {
                                "Account · sign-in canceled".into()
                            }
                            CancelLoginAccountStatus::NotFound => {
                                "Account · sign-in was already resolved".into()
                            }
                        };
                    }
                    Err(error) => {
                        app.status_line = format!("Account · cancel failed · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Sign out through the selected backend (fixture clears only fixture state).
    pub fn account_sign_out(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let fixture = self.fixture.clone();
        let backend = self.backend.clone();
        let use_live = matches!(self.connection, UiConnection::Ready { .. }) && backend.is_some();
        let use_fixture = self.is_explicit_fixture();
        self.status_line = "Account · sign out…".into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if use_live {
                        if let Some(backend) = backend {
                            if backend.kind() == BackendKind::MitsuroHttp {
                                return Err(
                                    "account logout is not exposed by the Mitsuro server".into()
                                );
                            }
                            backend
                                .account_logout()
                                .await
                                .map_err(|error| format!("account/logout: {error}"))?;
                            let acc = backend
                                .account_read(GetAccountParams::default())
                                .await
                                .ok()
                                .and_then(|r| r.account);
                            return Ok::<_, String>((acc, "app-server"));
                        }
                    }
                    if !use_fixture {
                        return Err("account logout is unavailable for this backend state".into());
                    }
                    let fixture = fixture.unwrap_or_else(|| Arc::new(FixtureBackend::new()));
                    if !fixture.status().is_usable() {
                        fixture.connect().await.map_err(|e| e.to_string())?;
                    }
                    fixture.account_logout().await.map_err(|e| e.to_string())?;
                    let acc = fixture
                        .account_read(GetAccountParams::default())
                        .await
                        .map_err(|e| e.to_string())?
                        .account;
                    Ok((acc, "fixture"))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((account, source)) => {
                        let empty = AccountSession::empty(source);
                        app.apply_account_snapshot(
                            account,
                            empty.usage,
                            empty.rate_limits,
                            source,
                            None,
                        );
                        app.account_state = if source == "fixture" {
                            SurfaceDataState::Fixture
                        } else {
                            SurfaceDataState::Live
                        };
                        app.account.login_detail = None;
                        app.account.pending_login_id = None;
                        app.account.pending_login_url = None;
                        app.status_line = format!("Account · signed out · {source}").into();
                    }
                    Err(message) => {
                        app.status_line = format!(
                            "Account sign-out failed · {message} · retained last server snapshot"
                        )
                        .into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Connection detail line for Settings.
    #[allow(dead_code)]
    pub fn connection_status_label(&self) -> SharedString {
        match &self.connection {
            UiConnection::Demo => "Demo chrome".into(),
            UiConnection::Fixture => "Fixture backend · sample-turn.jsonl · no paid API".into(),
            UiConnection::Connecting => "Connecting to codex app-server…".into(),
            UiConnection::Ready { detail, has_auth } => {
                let auth = if *has_auth { "auth ok" } else { "no auth" };
                format!("Ready · {detail} · {auth}").into()
            }
            UiConnection::Error { message } => format!("Error · {message}").into(),
        }
    }

    pub fn pending_approval(&self) -> Option<&PendingApproval> {
        self.pending_approval.as_ref()
    }

    pub fn pending_user_input(&self) -> Option<(&PendingUserInput, usize)> {
        self.pending_user_input
            .as_ref()
            .map(|pending| (pending, self.user_input_question_index))
    }

    pub fn pending_mcp_elicitation(&self) -> Option<(&PendingMcpElicitation, usize)> {
        self.pending_mcp_elicitation
            .as_ref()
            .map(|pending| (pending, self.mcp_form_field_index))
    }

    pub fn server_request_input(&self, secret: bool) -> &Entity<InputState> {
        if secret {
            &self.server_request_secret_input
        } else {
            &self.server_request_input
        }
    }

    fn clear_server_request_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.server_request_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.server_request_secret_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
    }

    fn send_codex_server_response(
        &mut self,
        request_id: mitsuro_desktop_backend::JsonRpcId,
        result: serde_json::Value,
        label: String,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = self.backend.clone() else {
            self.status_line = "Server response unavailable: no live backend.".into();
            cx.notify();
            return;
        };
        self.status_line = format!("{label} · sending…").into();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    let response_backend = Arc::clone(&backend);
                    backend.block_on(async move {
                        response_backend
                            .respond_to_server_request(request_id, result)
                            .await
                            .map_err(|error| error.to_string())
                    })
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                app.status_line = match outcome {
                    Ok(()) => format!("{label} · sent").into(),
                    Err(error) => format!("{label} failed · {error}").into(),
                };
                cx.notify();
            });
        })
        .detach();
    }

    pub fn answer_user_input_option(
        &mut self,
        answer: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.advance_user_input(vec![answer], window, cx);
    }

    pub fn submit_user_input_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let secret = self
            .pending_user_input()
            .and_then(|(pending, index)| pending.questions.get(index))
            .is_some_and(|question| question.is_secret);
        let answer = self
            .server_request_input(secret)
            .read(cx)
            .value()
            .to_string();
        self.advance_user_input(vec![answer], window, cx);
    }

    fn advance_user_input(
        &mut self,
        answers: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((question_id, question_count)) =
            self.pending_user_input.as_ref().and_then(|pending| {
                pending
                    .questions
                    .get(self.user_input_question_index)
                    .map(|question| (question.id.clone(), pending.questions.len()))
            })
        else {
            return;
        };
        self.user_input_answers.insert(question_id, answers);
        self.clear_server_request_inputs(window, cx);
        if self.user_input_question_index + 1 < question_count {
            self.user_input_question_index += 1;
            self.status_line = format!(
                "Input requested · question {} of {}",
                self.user_input_question_index + 1,
                question_count
            )
            .into();
            cx.notify();
            return;
        }
        let pending = self.pending_user_input.take().expect("pending checked");
        self.user_input_question_index = 0;
        let answers = std::mem::take(&mut self.user_input_answers);
        self.send_codex_server_response(
            pending.request_id,
            PendingUserInput::response(answers),
            "User input".to_owned(),
            cx,
        );
    }

    pub fn decline_user_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_user_input.take() else {
            return;
        };
        self.clear_server_request_inputs(window, cx);
        self.user_input_question_index = 0;
        self.user_input_answers.clear();
        let answers = pending
            .questions
            .iter()
            .map(|question| (question.id.clone(), Vec::new()))
            .collect();
        self.send_codex_server_response(
            pending.request_id,
            PendingUserInput::response(answers),
            "User input declined".to_owned(),
            cx,
        );
    }

    fn mcp_form_fields(pending: &PendingMcpElicitation) -> Vec<(String, serde_json::Value)> {
        let schema = match &pending.mode {
            McpElicitationMode::Form { requested_schema }
            | McpElicitationMode::OpenAiForm { requested_schema } => requested_schema,
            McpElicitationMode::Url { .. } => return Vec::new(),
        };
        schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .map(|properties| {
                properties
                    .iter()
                    .map(|(name, schema)| (name.clone(), schema.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn current_mcp_form_field(&self) -> Option<(String, serde_json::Value, usize)> {
        let pending = self.pending_mcp_elicitation.as_ref()?;
        let fields = Self::mcp_form_fields(pending);
        fields
            .get(self.mcp_form_field_index)
            .cloned()
            .map(|(name, schema)| (name, schema, fields.len()))
    }

    pub fn answer_mcp_form_option(
        &mut self,
        value: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.advance_mcp_form(value, window, cx);
    }

    pub fn submit_mcp_form_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((_, schema, _)) = self.current_mcp_form_field() else {
            return;
        };
        let raw = self
            .server_request_input(false)
            .read(cx)
            .value()
            .to_string();
        let value: Result<serde_json::Value, ()> =
            match schema.get("type").and_then(serde_json::Value::as_str) {
                Some("integer") => raw
                    .parse::<i64>()
                    .map(serde_json::Value::from)
                    .map_err(|_| ()),
                Some("number") => raw
                    .parse::<f64>()
                    .map(serde_json::Value::from)
                    .map_err(|_| ()),
                Some("boolean") => raw
                    .parse::<bool>()
                    .map(serde_json::Value::from)
                    .map_err(|_| ()),
                Some("array") => Ok(serde_json::Value::Array(
                    raw.split(',')
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                        .map(|value| serde_json::Value::String(value.to_owned()))
                        .collect(),
                )),
                _ => Ok(serde_json::Value::String(raw)),
            };
        match value {
            Ok(value) => self.advance_mcp_form(value, window, cx),
            Err(_) => {
                self.status_line = "MCP form · enter a value matching the requested type.".into();
                cx.notify();
            }
        }
    }

    fn advance_mcp_form(
        &mut self,
        value: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((name, _, count)) = self.current_mcp_form_field() else {
            return;
        };
        self.mcp_form_values.insert(name, value);
        self.clear_server_request_inputs(window, cx);
        if self.mcp_form_field_index + 1 < count {
            self.mcp_form_field_index += 1;
            cx.notify();
            return;
        }
        let pending = self
            .pending_mcp_elicitation
            .take()
            .expect("pending checked");
        self.mcp_form_field_index = 0;
        let values = std::mem::take(&mut self.mcp_form_values)
            .into_iter()
            .collect::<serde_json::Map<_, _>>();
        self.send_codex_server_response(
            pending.request_id,
            PendingMcpElicitation::accept(serde_json::Value::Object(values)),
            format!("MCP response · {}", pending.server_name),
            cx,
        );
    }

    pub fn decline_mcp_elicitation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_mcp_elicitation.take() else {
            return;
        };
        self.clear_server_request_inputs(window, cx);
        self.mcp_form_field_index = 0;
        self.mcp_form_values.clear();
        self.send_codex_server_response(
            pending.request_id,
            PendingMcpElicitation::decline(),
            format!("MCP request declined · {}", pending.server_name),
            cx,
        );
    }

    pub fn accept_empty_mcp_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_mcp_elicitation.take() else {
            return;
        };
        self.clear_server_request_inputs(window, cx);
        self.send_codex_server_response(
            pending.request_id,
            PendingMcpElicitation::accept(serde_json::json!({})),
            format!("MCP response · {}", pending.server_name),
            cx,
        );
    }

    pub fn open_mcp_elicitation_url(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(url) =
            self.pending_mcp_elicitation
                .as_ref()
                .and_then(|pending| match &pending.mode {
                    McpElicitationMode::Url { url, .. } => Some(url.clone()),
                    _ => None,
                })
        else {
            return;
        };
        self.browser_url_input.update(cx, |state, cx| {
            state.set_value(url, window, cx);
        });
        self.browser_navigate(window, cx);
        self.browser_open_external(cx);
    }

    pub fn accept_mcp_url_elicitation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_mcp_elicitation.take() else {
            return;
        };
        self.clear_server_request_inputs(window, cx);
        self.send_codex_server_response(
            pending.request_id,
            PendingMcpElicitation::accept(serde_json::json!({})),
            format!("MCP authorization accepted · {}", pending.server_name),
            cx,
        );
    }

    /// Approve or reject the current pending approval (fixture resume + live respond).
    pub fn resolve_pending_approval(&mut self, choice: ApprovalChoice, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_approval.take() else {
            self.status_line = "No pending approval.".into();
            cx.notify();
            return;
        };

        let label = match choice {
            ApprovalChoice::Approve => "approved",
            ApprovalChoice::Reject => "rejected",
            ApprovalChoice::Abort => "aborted",
        };
        self.status_line = format!(
            "Approval {label}: {}",
            pending.summary.chars().take(48).collect::<String>()
        )
        .into();

        // Progressive live path: unblock the turn loop; it writes respond_approval.
        if let Some(bridge) = self.live_approval_bridge.as_ref() {
            if bridge.submit(choice) {
                self.status_line = format!("Approval {label} · live turn continuing…").into();
                cx.notify();
                return;
            }
        }

        // Fallback live path (no bridge waiter): write JSON-RPC result directly.
        if let Some(backend) = self.backend.clone() {
            let pending_live = pending;
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|e| e.to_string())?;
                        rt.block_on(async {
                            backend
                                .respond_approval(&pending_live, choice)
                                .await
                                .map_err(|e| e.to_string())
                        })
                    })
                    .await;
            })
            .detach();
        }

        // Fixture path: resume remaining stream events after the approval pause.
        if let Some((thread_id, rest)) = self.fixture_resume.take() {
            self.continue_fixture_events(thread_id, rest, cx);
        } else {
            cx.notify();
        }
    }

    pub fn select_thread(&mut self, id: String, cx: &mut Context<Self>) {
        self.selected_thread = Some(id.clone());
        self.thread_menu_open = false;
        self.composer_access_menu_open = false;
        let backend_session_id = self
            .threads
            .iter()
            .find(|thread| thread.summary.id == id)
            .and_then(|thread| thread.backend_session_id.clone());
        if let Some(session_id) = backend_session_id {
            self.preferences.remember_session(session_id);
            self.save_preferences_best_effort();
        }
        // Keep Chat vs Codex mode aligned with the thread surface when possible.
        let msg_count = self
            .threads
            .iter()
            .find(|t| t.summary.id == id)
            .map(|t| t.messages.len());
        if let Some(t) = self.threads.iter().find(|t| t.summary.id == id) {
            match t.surface {
                ThreadSurface::Chat => {
                    self.selected_chat_thread = Some(id.clone());
                    if !matches!(self.active_mode, ProductMode::Chat) {
                        self.active_mode = ProductMode::Chat;
                    }
                }
                ThreadSurface::Codex => {
                    self.selected_codex_thread = Some(id.clone());
                    if !matches!(self.active_mode, ProductMode::Codex | ProductMode::Chat) {
                        self.active_mode = ProductMode::Codex;
                    }
                }
            }
            let n = msg_count.unwrap_or(0);
            self.status_line = if n > 0 {
                format!("thread · {n} msgs")
            } else {
                format!("{} · {}", t.surface.label(), t.summary.display_title())
            }
            .into();
        } else {
            self.status_line = "Thread selected.".into();
        }

        // Real server threads when Ready: always thread/read if local cache is empty
        // (list/bootstrap leaves messages empty until open).
        if is_app_server_thread_id(&id) {
            if let Some(backend) = self.live_backend() {
                let empty = self
                    .threads
                    .iter()
                    .find(|t| t.summary.id == id)
                    .map(|t| t.messages.is_empty())
                    .unwrap_or(true);
                if empty {
                    self.status_line = "thread/read…".into();
                    self.load_thread_messages(backend, id, cx);
                }
            }
        }
        self.transcript_scroll_handle.scroll_to_bottom();
        cx.notify();
    }

    /// Select thread and refresh composer placeholder (needs `Window`).
    pub fn select_thread_with_window(
        &mut self,
        id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.select_thread(id, cx);
        self.update_composer_placeholder(window, cx);
    }

    /// Load offline sample threads into the sidebar (dev/review). Off by default for BAR first paint.
    #[allow(dead_code)]
    pub fn toggle_samples(&mut self, cx: &mut Context<Self>) {
        if self.samples_loaded {
            // Keep user-created threads; drop demo ids.
            let demo_ids: std::collections::HashSet<_> = demo::demo_threads()
                .into_iter()
                .map(|t| t.summary.id)
                .collect();
            self.threads.retain(|t| !demo_ids.contains(&t.summary.id));
            self.samples_loaded = false;
            if self
                .selected_thread
                .as_ref()
                .is_some_and(|id| demo_ids.contains(id))
            {
                self.selected_thread = None;
            }
        } else {
            for t in demo::demo_threads() {
                if !self.threads.iter().any(|x| x.summary.id == t.summary.id) {
                    self.threads.push(t);
                }
            }
            self.samples_loaded = true;
        }
        cx.notify();
    }

    #[allow(dead_code)]
    pub fn samples_loaded(&self) -> bool {
        self.samples_loaded
    }

    pub fn mode_menu_open(&self) -> bool {
        self.mode_menu_open
    }

    pub fn toggle_mode_menu(&mut self, cx: &mut Context<Self>) {
        self.mode_menu_open = !self.mode_menu_open;
        if self.mode_menu_open {
            self.thread_menu_open = false;
        }
        cx.notify();
    }

    pub fn close_mode_menu(&mut self, cx: &mut Context<Self>) {
        if self.mode_menu_open {
            self.mode_menu_open = false;
            cx.notify();
        }
    }

    pub fn thread_menu_open(&self) -> bool {
        self.thread_menu_open
    }

    pub fn toggle_thread_menu(&mut self, cx: &mut Context<Self>) {
        self.thread_menu_open = !self.thread_menu_open;
        if self.thread_menu_open {
            self.mode_menu_open = false;
        }
        cx.notify();
    }

    #[allow(dead_code)]
    pub fn close_thread_menu(&mut self, cx: &mut Context<Self>) {
        if self.thread_menu_open {
            self.thread_menu_open = false;
            cx.notify();
        }
    }

    /// Switch Chat ↔ Codex from the sidebar mode pill (clears selection → home hero).
    pub fn switch_thread_surface(
        &mut self,
        surface: demo::ThreadSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.mode_menu_open = false;
        self.thread_menu_open = false;
        let mode = match surface {
            demo::ThreadSurface::Chat => ProductMode::Chat,
            demo::ThreadSurface::Codex => ProductMode::Codex,
        };
        // Clear selection so home hero shows (bar: mode switch lands on empty home).
        self.remember_thread_selection_for_mode(self.active_mode);
        self.selected_thread = None;
        self.set_mode(mode, window, cx);
        // set_mode may restore remembered selection — force home for switcher.
        self.selected_thread = None;
        self.update_composer_placeholder(window, cx);
        self.status_line = format!("Mode · {}", mode.label()).into();
        cx.notify();
    }

    #[allow(dead_code)]
    pub fn dismiss_usage_card(&mut self, cx: &mut Context<Self>) {
        self.dismiss_usage_card = true;
        cx.notify();
    }

    pub fn usage_card_visible(&self) -> bool {
        !self.dismiss_usage_card
            && self.is_calm_stage()
            && self.account.source == "app-server"
            && self.account.is_rate_limited_out()
    }

    /// Profile row label for sidebar footer / Settings.
    ///
    /// Fixture demo uses [`mitsuro_desktop_backend::FIXTURE_DEMO_DISPLAY_NAME`] ("Jacob Burgess").
    /// Override with `MITSURO_PROFILE_NAME` for capture / demos.
    pub fn profile_display_name(&self) -> SharedString {
        if let Ok(name) = std::env::var("MITSURO_PROFILE_NAME") {
            let name = name.trim();
            if !name.is_empty() {
                return SharedString::from(name.to_string());
            }
        }
        if self.account.source == "fixture" {
            return SharedString::from(mitsuro_desktop_backend::FIXTURE_DEMO_DISPLAY_NAME);
        }
        if self.account.signed_in {
            return SharedString::from("ChatGPT account");
        }
        if self.active_backend_kind() == Some(BackendKind::MitsuroHttp) {
            return SharedString::from("Mitsuro");
        }
        SharedString::from("Account")
    }

    /// Plan chip for profile footer (e.g. "Pro"). Empty when unknown.
    pub fn profile_plan_label(&self) -> Option<SharedString> {
        self.account
            .plan_label
            .as_ref()
            .map(|p| SharedString::from(p.clone()))
    }

    /// Initials for solid avatar chip (e.g. "JB" from Jacob Burgess).
    #[allow(dead_code)]
    pub fn profile_initials(&self) -> SharedString {
        SharedString::from(profile_initials_from_name(&self.profile_display_name()))
    }

    /// Return to home (Codex/Chat) with no thread selected.
    pub fn go_home(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = match self.active_thread_surface() {
            demo::ThreadSurface::Chat => ProductMode::Chat,
            demo::ThreadSurface::Codex => ProductMode::Codex,
        };
        self.selected_thread = None;
        self.mode_menu_open = false;
        self.thread_menu_open = false;
        self.set_mode(mode, window, cx);
        self.selected_thread = None;
        self.update_composer_placeholder(window, cx);
        cx.notify();
    }

    pub fn new_thread(&mut self, cx: &mut Context<Self>) {
        let surface = self.active_thread_surface();
        // New conversations stay local only until first Send. This is an
        // optimistic draft, not synthetic backend data: promotion creates the
        // real server session with the selected workspace/access contract.
        if (matches!(self.connection, UiConnection::Ready { .. }) && self.backend.is_some())
            || self.is_explicit_fixture()
        {
            self.new_thread_local(surface, cx);
        } else {
            self.status_line = "New thread is unavailable until a backend is ready.".into();
            cx.notify();
        }
    }

    /// Optimistic draft (local id only until first live Send).
    fn new_thread_local(&mut self, surface: ThreadSurface, cx: &mut Context<Self>) {
        let id = format!("local-{}", self.threads.len() + 1);
        let is_fixture = self.is_explicit_fixture();
        let (name, preview, cwd) = match surface {
            ThreadSurface::Chat => (
                "New chat".into(),
                if is_fixture {
                    "Offline fixture draft".into()
                } else {
                    "Draft".into()
                },
                None,
            ),
            ThreadSurface::Codex => (
                "New thread".into(),
                if is_fixture {
                    "Offline fixture draft".into()
                } else {
                    "Draft".into()
                },
                self.composer_default_workspace_dir.clone(),
            ),
        };
        let thread = DemoThread {
            backend_session_id: None,
            summary: ThreadSummary {
                id: id.clone(),
                name: Some(name),
                preview: Some(preview),
                cwd,
                created_at: None,
                updated_at: None,
                model_provider: Some(if is_fixture {
                    "fixture".into()
                } else {
                    self.active_backend_kind()
                        .map(|kind| kind.id().to_owned())
                        .unwrap_or_else(|| "draft".into())
                }),
                ephemeral: Some(true),
                is_pinned: Some(false),
                archived: Some(false),
                raw: None,
            },
            surface,
            messages: vec![],
        };
        self.threads.insert(0, thread);
        self.selected_thread = Some(id.clone());
        if let Some(mode) = self.composer_default_access_mode {
            self.composer_access_modes.insert(id.clone(), mode);
        }
        match surface {
            ThreadSurface::Chat => {
                self.selected_chat_thread = Some(id);
                self.active_mode = ProductMode::Chat;
            }
            ThreadSurface::Codex => {
                self.selected_codex_thread = Some(id);
                self.active_mode = ProductMode::Codex;
            }
        }
        self.status_line = if is_fixture {
            format!("Started an offline fixture {} draft.", surface.label()).into()
        } else {
            format!("Started a {} draft.", surface.label()).into()
        };
        cx.notify();
    }

    /// Whether the sidebar shows archived threads.
    #[allow(dead_code)]
    pub fn show_archived(&self) -> bool {
        self.show_archived
    }

    /// Toggle show/hide archived threads in the sidebar.
    #[allow(dead_code)]
    pub fn toggle_show_archived(&mut self, cx: &mut Context<Self>) {
        self.show_archived = !self.show_archived;
        self.status_line = if self.show_archived {
            "Showing archived threads.".into()
        } else {
            "Hiding archived threads.".into()
        };
        cx.notify();
    }

    /// Whether a turn is currently streaming (Send blocked / Stop visible).
    pub fn turn_in_progress(&self) -> bool {
        self.turn_in_progress
    }

    pub fn composer_attachments(&self) -> &[ComposerAttachment] {
        &self.composer_attachments
    }

    pub fn composer_add_menu_open(&self) -> bool {
        self.composer_add_menu_open
    }

    pub fn composer_model_menu_open(&self) -> bool {
        self.composer_model_menu_open
    }

    pub fn composer_reasoning_menu_open(&self) -> bool {
        self.composer_reasoning_menu_open
    }

    pub fn composer_model_search_input(&self) -> &Entity<InputState> {
        &self.composer_model_search_input
    }

    pub fn visible_composer_models(&self, query: &str) -> Vec<&ModelInfo> {
        self.models
            .iter()
            .filter(|model| !model.hidden && model_matches_query(model, query))
            .collect()
    }

    pub fn composer_reasoning_choices(&self) -> Vec<(String, String)> {
        let Some(model) = self.selected_model() else {
            return Vec::new();
        };
        model
            .supported_reasoning_efforts
            .iter()
            .filter_map(|option| {
                let effort = option.reasoning_effort.trim();
                (!effort.is_empty()).then(|| (effort.to_owned(), option.description.clone()))
            })
            .collect()
    }

    pub fn selected_reasoning_effort_is(&self, effort: &str) -> bool {
        self.selected_reasoning_effort() == Some(effort)
    }

    pub fn toggle_composer_model_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.models.iter().all(|model| model.hidden) {
            self.status_line = "No selectable models are available.".into();
            cx.notify();
            return;
        }
        self.composer_model_menu_open = !self.composer_model_menu_open;
        self.composer_reasoning_menu_open = false;
        self.composer_add_menu_open = false;
        self.composer_access_menu_open = false;
        if self.composer_model_menu_open {
            self.composer_model_search_input
                .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        }
        cx.notify();
    }

    pub fn toggle_composer_reasoning_menu(&mut self, cx: &mut Context<Self>) {
        if !self.has_reasoning_effort_control() {
            self.status_line =
                "The selected model does not advertise multiple reasoning levels.".into();
            cx.notify();
            return;
        }
        self.composer_reasoning_menu_open = !self.composer_reasoning_menu_open;
        self.composer_model_menu_open = false;
        self.composer_add_menu_open = false;
        self.composer_access_menu_open = false;
        cx.notify();
    }

    pub fn select_reasoning_effort(&mut self, effort: String, cx: &mut Context<Self>) {
        if !self
            .reasoning_options_for_selected_model()
            .iter()
            .any(|option| option == &effort)
        {
            self.status_line =
                "That reasoning level is not supported by the selected model.".into();
            cx.notify();
            return;
        }
        self.selected_reasoning_effort = Some(effort.clone());
        self.composer_reasoning_menu_open = false;
        self.remember_selected_reasoning();
        self.status_line = format!("Reasoning: {}", reasoning_effort_display_name(&effort)).into();
        cx.notify();
    }

    pub fn can_open_composer_add_menu(&self) -> bool {
        self.can_attach_images() || self.can_mention_files() || self.can_add_skills()
    }

    pub fn toggle_composer_add_menu(&mut self, cx: &mut Context<Self>) {
        if !self.can_open_composer_add_menu() {
            self.status_line = "No addable inputs are available for this backend and model.".into();
            cx.notify();
            return;
        }
        self.composer_add_menu_open = !self.composer_add_menu_open;
        if self.composer_add_menu_open {
            self.composer_access_menu_open = false;
            self.composer_model_menu_open = false;
            self.composer_reasoning_menu_open = false;
        }
        cx.notify();
    }

    pub fn show_composer_workspace_control(&self) -> bool {
        self.live_backend()
            .is_some_and(|backend| backend.capabilities().workspace_selection)
    }

    pub fn can_select_composer_workspace(&self) -> bool {
        !self.turn_in_progress
            && self.show_composer_workspace_control()
            && self
                .selected_thread()
                .is_none_or(|thread| thread.backend_session_id.is_none())
    }

    fn composer_workspace_dir(&self) -> Option<&str> {
        match self.selected_thread() {
            Some(thread) => thread.summary.cwd.as_deref(),
            None => self.composer_default_workspace_dir.as_deref(),
        }
    }

    pub fn composer_workspace_label(&self) -> SharedString {
        let Some(path) = self.composer_workspace_dir() else {
            return "Choose project".into();
        };
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(path)
            .to_owned()
            .into()
    }

    pub fn select_composer_workspace(&mut self, cx: &mut Context<Self>) {
        if !self.can_select_composer_workspace() {
            self.status_line = if self
                .selected_thread()
                .is_some_and(|thread| thread.backend_session_id.is_some())
            {
                "The workspace is fixed for this server thread. Start a new thread to change it."
                    .into()
            } else {
                "Project selection is unavailable for the current backend state.".into()
            };
            cx.notify();
            return;
        }
        self.composer_access_menu_open = false;
        self.composer_model_menu_open = false;
        self.composer_reasoning_menu_open = false;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose project folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            let _ = this.update(cx, |app, cx| {
                match selected {
                    Ok(Ok(Some(paths))) => {
                        let path = paths.into_iter().next();
                        let canonical = path
                            .as_deref()
                            .and_then(|path| std::fs::canonicalize(path).ok())
                            .filter(|path| path.is_dir());
                        if let Some(path) = canonical {
                            let raw_path = path.display().to_string();
                            app.composer_default_workspace_dir = Some(raw_path.clone());
                            if let Some(thread_id) = app.selected_thread.clone() {
                                if let Some(thread) = app.threads.iter_mut().find(|thread| {
                                    thread.summary.id == thread_id
                                        && thread.backend_session_id.is_none()
                                }) {
                                    thread.summary.cwd = Some(raw_path.clone());
                                }
                            }
                            app.status_line = format!("Project · {raw_path}").into();
                        } else {
                            app.status_line =
                                "Project selection rejected · choose an existing folder.".into();
                        }
                    }
                    Ok(Ok(None)) | Err(_) => {
                        app.status_line = "Project selection canceled.".into();
                    }
                    Ok(Err(error)) => {
                        app.status_line = format!("Project picker failed · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn show_composer_access_control(&self) -> bool {
        self.live_backend()
            .is_some_and(|backend| backend.capabilities().access_modes)
    }

    pub fn composer_access_menu_open(&self) -> bool {
        self.composer_access_menu_open
    }

    fn composer_access_mode(&self) -> Option<ProductAccessMode> {
        match self.selected_thread.as_ref() {
            Some(id) => self.composer_access_modes.get(id).copied(),
            None => self.composer_default_access_mode,
        }
    }

    pub fn composer_access_label(&self) -> &'static str {
        match self.composer_access_mode() {
            Some(ProductAccessMode::CodexReadOnly) => "Read-only",
            Some(ProductAccessMode::CodexAuto) => "Auto",
            Some(ProductAccessMode::CodexFullAccess) => "Full access",
            Some(ProductAccessMode::MitsuroSupervised) => "Supervised",
            Some(ProductAccessMode::MitsuroAutonomous) => "Autonomous",
            None => "Default access",
        }
    }

    pub fn composer_access_mode_is(&self, mode: ProductAccessMode) -> bool {
        self.composer_access_mode() == Some(mode)
    }

    pub fn composer_access_choices(&self) -> Vec<(ProductAccessMode, &'static str, &'static str)> {
        match self.active_backend_kind() {
            Some(BackendKind::CodexStdio | BackendKind::CodexWebSocket) => vec![
                (
                    ProductAccessMode::CodexReadOnly,
                    "Read-only",
                    "Ask before actions; do not write files",
                ),
                (
                    ProductAccessMode::CodexAuto,
                    "Auto",
                    "Write in the workspace; ask when needed",
                ),
                (
                    ProductAccessMode::CodexFullAccess,
                    "Full access",
                    "Run without sandbox or approval prompts",
                ),
            ],
            Some(BackendKind::MitsuroHttp) => vec![
                (
                    ProductAccessMode::MitsuroSupervised,
                    "Supervised",
                    "Require approval for governed tools",
                ),
                (
                    ProductAccessMode::MitsuroAutonomous,
                    "Autonomous",
                    "Use Mitsuro's autonomous permission mode",
                ),
            ],
            _ => Vec::new(),
        }
    }

    pub fn toggle_composer_access_menu(&mut self, cx: &mut Context<Self>) {
        if self.turn_in_progress || !self.show_composer_access_control() {
            self.status_line =
                "Access selection is unavailable for the current backend state.".into();
            cx.notify();
            return;
        }
        self.composer_access_menu_open = !self.composer_access_menu_open;
        if self.composer_access_menu_open {
            self.composer_add_menu_open = false;
            self.composer_model_menu_open = false;
            self.composer_reasoning_menu_open = false;
        }
        cx.notify();
    }

    pub fn select_composer_access_mode(&mut self, mode: ProductAccessMode, cx: &mut Context<Self>) {
        if !self
            .composer_access_choices()
            .iter()
            .any(|(candidate, _, _)| *candidate == mode)
        {
            self.status_line = "That access mode does not belong to the active backend.".into();
            cx.notify();
            return;
        }
        if let Some(thread_id) = self.selected_thread.clone() {
            self.composer_access_modes.insert(thread_id, mode);
        } else {
            self.composer_default_access_mode = Some(mode);
        }
        self.composer_access_menu_open = false;
        self.status_line = format!("Access · {}", self.composer_access_label()).into();
        cx.notify();
    }

    pub fn can_attach_images(&self) -> bool {
        !self.turn_in_progress
            && self.selected_model_supports("image")
            && self
                .live_backend()
                .is_some_and(|backend| backend.capabilities().image_attachments)
    }

    pub fn can_attach_audio(&self) -> bool {
        !self.turn_in_progress
            && self.selected_model_supports("audio")
            && self
                .live_backend()
                .is_some_and(|backend| backend.capabilities().audio_attachments)
    }

    pub fn can_mention_files(&self) -> bool {
        !self.turn_in_progress
            && self
                .live_backend()
                .is_some_and(|backend| backend.capabilities().mention_inputs)
    }

    pub fn can_add_skills(&self) -> bool {
        !self.turn_in_progress
            && self.skills.iter().any(|skill| skill.enabled)
            && self
                .live_backend()
                .is_some_and(|backend| backend.capabilities().skill_inputs)
    }

    pub fn enabled_composer_skills(&self) -> impl Iterator<Item = &SkillMetadata> {
        self.skills.iter().filter(|skill| skill.enabled)
    }

    fn binary_attachment_count(&self) -> usize {
        self.composer_attachments
            .iter()
            .filter(|attachment| {
                matches!(
                    attachment.kind,
                    ComposerAttachmentKind::Image | ComposerAttachmentKind::Audio
                )
            })
            .count()
    }

    fn reference_attachment_count(&self) -> usize {
        self.composer_attachments
            .iter()
            .filter(|attachment| {
                matches!(
                    attachment.kind,
                    ComposerAttachmentKind::Skill | ComposerAttachmentKind::Mention
                )
            })
            .count()
    }

    fn selected_model_supports(&self, modality: &str) -> bool {
        self.selected_model().is_some_and(|model| {
            model
                .input_modalities
                .iter()
                .any(|candidate| candidate == modality)
        })
    }

    pub fn select_composer_images(&mut self, cx: &mut Context<Self>) {
        if !self.can_attach_images() {
            self.status_line =
                "Image attachments are unavailable for the current backend state.".into();
            cx.notify();
            return;
        }
        self.composer_add_menu_open = false;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach images".into()),
        });
        cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            let _ = this.update(cx, |app, cx| {
                match selected {
                    Ok(Ok(Some(paths))) => {
                        let mut rejected = 0usize;
                        for path in paths {
                            if app.binary_attachment_count() >= 4 {
                                rejected += 1;
                                continue;
                            }
                            let supported = path
                                .extension()
                                .and_then(|extension| extension.to_str())
                                .is_some_and(|extension| {
                                    matches!(
                                        extension.to_ascii_lowercase().as_str(),
                                        "png" | "jpg" | "jpeg" | "webp" | "gif"
                                    )
                                });
                            let size_ok = std::fs::metadata(&path)
                                .map(|metadata| metadata.is_file() && metadata.len() <= 20 * 1024 * 1024)
                                .unwrap_or(false);
                            let Some(raw_path) = path.to_str() else {
                                rejected += 1;
                                continue;
                            };
                            if !supported || !size_ok || !Path::new(raw_path).is_absolute() {
                                rejected += 1;
                                continue;
                            }
                            if app
                                .composer_attachments
                                .iter()
                                .any(|attachment| attachment.path == raw_path)
                            {
                                continue;
                            }
                            let name = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("image")
                                .to_owned();
                            app.composer_attachments.push(ComposerAttachment {
                                path: raw_path.to_owned(),
                                name,
                                kind: ComposerAttachmentKind::Image,
                            });
                        }
                        app.status_line = if rejected == 0 {
                            format!(
                                "Attached {} image(s).",
                                app.composer_attachments.len()
                            )
                            .into()
                        } else {
                            format!(
                                "Attached {} image(s) · rejected {rejected} unsupported, oversized, or excess file(s).",
                                app.composer_attachments.len()
                            )
                            .into()
                        };
                    }
                    Ok(Ok(None)) | Err(_) => {
                        app.status_line = "Image attachment canceled.".into();
                    }
                    Ok(Err(error)) => {
                        app.status_line = format!("Image picker failed · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn select_composer_audio(&mut self, cx: &mut Context<Self>) {
        if !self.can_attach_audio() {
            self.status_line =
                "Audio attachments are unavailable for this backend or selected model.".into();
            cx.notify();
            return;
        }
        self.composer_add_menu_open = false;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach audio".into()),
        });
        cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            let _ = this.update(cx, |app, cx| {
                match selected {
                    Ok(Ok(Some(paths))) => {
                        let mut rejected = 0usize;
                        for path in paths {
                            if app.binary_attachment_count() >= 4 {
                                rejected += 1;
                                continue;
                            }
                            let supported = path
                                .extension()
                                .and_then(|extension| extension.to_str())
                                .is_some_and(|extension| {
                                    matches!(
                                        extension.to_ascii_lowercase().as_str(),
                                        "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus"
                                    )
                                });
                            let size_ok = std::fs::metadata(&path)
                                .map(|metadata| {
                                    metadata.is_file() && metadata.len() <= 20 * 1024 * 1024
                                })
                                .unwrap_or(false);
                            let Some(raw_path) = path.to_str() else {
                                rejected += 1;
                                continue;
                            };
                            if !supported || !size_ok || !Path::new(raw_path).is_absolute() {
                                rejected += 1;
                                continue;
                            }
                            if app
                                .composer_attachments
                                .iter()
                                .any(|attachment| attachment.path == raw_path)
                            {
                                continue;
                            }
                            let name = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("audio")
                                .to_owned();
                            app.composer_attachments.push(ComposerAttachment {
                                path: raw_path.to_owned(),
                                name,
                                kind: ComposerAttachmentKind::Audio,
                            });
                        }
                        app.status_line = if rejected == 0 {
                            format!("Attached {} file(s).", app.composer_attachments.len()).into()
                        } else {
                            format!(
                                "Attached {} file(s) · rejected {rejected} unsupported, oversized, or excess file(s).",
                                app.composer_attachments.len()
                            )
                            .into()
                        };
                    }
                    Ok(Ok(None)) | Err(_) => {
                        app.status_line = "Audio attachment canceled.".into();
                    }
                    Ok(Err(error)) => {
                        app.status_line = format!("Audio picker failed · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn select_composer_mention(&mut self, cx: &mut Context<Self>) {
        if !self.can_mention_files() {
            self.status_line = "File mentions are unavailable for this backend.".into();
            cx.notify();
            return;
        }
        self.composer_add_menu_open = false;
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Mention files".into()),
        });
        cx.spawn(async move |this, cx| {
            let selected = receiver.await;
            let _ = this.update(cx, |app, cx| {
                match selected {
                    Ok(Ok(Some(paths))) => {
                        let mut rejected = 0usize;
                        for path in paths {
                            if app.reference_attachment_count() >= 8 {
                                rejected += 1;
                                continue;
                            }
                            let Some(raw_path) = path.to_str() else {
                                rejected += 1;
                                continue;
                            };
                            let valid = path.is_absolute()
                                && std::fs::metadata(&path)
                                    .is_ok_and(|metadata| metadata.is_file());
                            if !valid {
                                rejected += 1;
                                continue;
                            }
                            if app
                                .composer_attachments
                                .iter()
                                .any(|attachment| attachment.path == raw_path)
                            {
                                continue;
                            }
                            let name = path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("file")
                                .to_owned();
                            app.composer_attachments.push(ComposerAttachment {
                                path: raw_path.to_owned(),
                                name,
                                kind: ComposerAttachmentKind::Mention,
                            });
                        }
                        app.status_line = if rejected == 0 {
                            "File mention(s) added.".into()
                        } else {
                            format!("File mentions added · rejected {rejected} invalid or excess file(s).")
                                .into()
                        };
                    }
                    Ok(Ok(None)) | Err(_) => {
                        app.status_line = "File mention canceled.".into();
                    }
                    Ok(Err(error)) => {
                        app.status_line = format!("File mention picker failed · {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn add_composer_skill(&mut self, name: String, cx: &mut Context<Self>) {
        if !self.can_add_skills() || self.reference_attachment_count() >= 8 {
            self.status_line = "No additional skill references are available.".into();
            cx.notify();
            return;
        }
        let Some(skill) = self
            .skills
            .iter()
            .find(|skill| skill.enabled && skill.name == name)
        else {
            self.status_line = "The selected skill is no longer available.".into();
            cx.notify();
            return;
        };
        if !self.composer_attachments.iter().any(|attachment| {
            attachment.kind == ComposerAttachmentKind::Skill && attachment.path == skill.path
        }) {
            self.composer_attachments.push(ComposerAttachment {
                path: skill.path.clone(),
                name: skill.name.clone(),
                kind: ComposerAttachmentKind::Skill,
            });
        }
        self.composer_add_menu_open = false;
        self.status_line = format!("Skill added · {}", skill.name).into();
        cx.notify();
    }

    pub fn remove_composer_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.composer_attachments.len() {
            self.composer_attachments.remove(index);
            self.status_line = "Attachment removed.".into();
            cx.notify();
        }
    }

    pub fn can_steer_active_turn(&self) -> bool {
        self.turn_in_progress
            && self
                .backend
                .as_ref()
                .is_some_and(|backend| backend.capabilities().steering)
            && self.active_turn_thread_id.is_some()
            && self.active_turn_id.is_some()
    }

    /// Visible threads for the sidebar (surface + search + archived filter).
    pub fn visible_threads(&self) -> Vec<DemoThread> {
        let surface = self.active_thread_surface();
        self.threads
            .iter()
            .filter(|t| {
                if t.surface != surface {
                    return false;
                }
                let archived = t.summary.archived.unwrap_or(false);
                if !self.show_archived && archived {
                    return false;
                }
                self.thread_matches_search(&t.summary)
            })
            .cloned()
            .collect()
    }

    pub fn can_compact_selected_thread(&self) -> bool {
        !self.turn_in_progress
            && self
                .backend
                .as_ref()
                .is_some_and(|backend| backend.capabilities().manual_compaction)
            && self
                .selected_thread
                .as_ref()
                .and_then(|id| self.live_session_id(id))
                .is_some()
    }

    pub fn can_review_selected_thread(&self) -> bool {
        !self.turn_in_progress
            && self
                .backend
                .as_ref()
                .is_some_and(|backend| backend.capabilities().review)
            && self
                .selected_thread
                .as_ref()
                .and_then(|id| self.live_session_id(id))
                .is_some()
    }

    pub fn review_selected_thread(&mut self, cx: &mut Context<Self>) {
        self.thread_menu_open = false;
        if self.turn_in_progress {
            self.status_line = "Review unavailable · wait for the active turn to finish.".into();
            cx.notify();
            return;
        }
        let Some(backend) = self.live_backend() else {
            self.status_line = "Review unavailable · backend is not ready.".into();
            cx.notify();
            return;
        };
        if !backend.capabilities().review {
            self.status_line = "Review is not supported by the selected backend.".into();
            cx.notify();
            return;
        }
        let Some(thread_id) = self.selected_thread.clone() else {
            self.status_line = "Review · no thread selected".into();
            cx.notify();
            return;
        };
        let Some(session_id) = self.live_session_id(&thread_id) else {
            self.status_line = "Review · live session identity is missing".into();
            cx.notify();
            return;
        };

        self.turn_in_progress = true;
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.active_turn_thread_id = Some(thread_id.clone());
        self.status_line = "Reviewing uncommitted changes…".into();
        self.start_live_review(thread_id, session_id, cx);
        cx.notify();
    }

    pub fn compact_selected_thread(&mut self, cx: &mut Context<Self>) {
        self.thread_menu_open = false;
        if self.turn_in_progress {
            self.status_line = "Compact unavailable · wait for the active turn to finish.".into();
            cx.notify();
            return;
        }
        let Some(backend) = self.live_backend() else {
            self.status_line = "Compact unavailable · backend is not ready.".into();
            cx.notify();
            return;
        };
        if !backend.capabilities().manual_compaction {
            self.status_line = "Compact is not supported by the selected backend.".into();
            cx.notify();
            return;
        }
        let Some(thread_id) = self.selected_thread.clone() else {
            self.status_line = "Compact · no thread selected".into();
            cx.notify();
            return;
        };
        let Some(session_id) = self.live_session_id(&thread_id) else {
            self.status_line = "Compact · live session identity is missing".into();
            cx.notify();
            return;
        };
        let backend_generation = self.backend_generation;
        self.status_line = "Compacting thread…".into();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    backend
                        .compact_session(&session_id)
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != backend_generation {
                    return;
                }
                app.status_line = match result {
                    Ok(()) => "Compaction started.".into(),
                    Err(error) => format!("Compact failed · {error}").into(),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Archive the selected thread (or toggle unarchive when already archived).
    pub fn archive_selected_thread(&mut self, cx: &mut Context<Self>) {
        self.thread_menu_open = false;
        if self.live_backend().is_none() && !self.is_explicit_fixture() {
            self.status_line = "Archive is unavailable until a backend is ready.".into();
            cx.notify();
            return;
        }
        if self
            .live_backend()
            .is_some_and(|backend| !backend.capabilities().archive)
        {
            self.status_line = "Archive is not supported by the selected backend.".into();
            cx.notify();
            return;
        }
        let Some(id) = self.selected_thread.clone() else {
            self.status_line = "Archive · no thread selected".into();
            cx.notify();
            return;
        };
        let is_archived = self
            .threads
            .iter()
            .find(|t| t.summary.id == id)
            .and_then(|t| t.summary.archived)
            .unwrap_or(false);

        if is_archived {
            self.unarchive_thread_id(id, cx);
        } else {
            self.archive_thread_id(id, cx);
        }
    }

    fn archive_thread_id(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(t) = self.threads.iter_mut().find(|t| t.summary.id == id) {
            t.summary.archived = Some(true);
        }
        self.status_line = "thread/archive…".into();
        // Deselect if not showing archived
        if !self.show_archived && self.selected_thread.as_deref() == Some(id.as_str()) {
            self.selected_thread = self
                .threads
                .iter()
                .find(|t| !t.summary.archived.unwrap_or(false))
                .map(|t| t.summary.id.clone());
        }

        // Live app-server when Ready + real server id (mirror thread_name_set).
        if is_app_server_thread_id(&id) {
            if let Some(backend) = self.live_backend() {
                let tid = id;
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .map_err(|e| e.to_string())?;
                            rt.block_on(async {
                                backend
                                    .thread_archive(ThreadArchiveParams::new(tid.clone()))
                                    .await
                                    .map_err(|e| e.to_string())?;
                                Ok::<_, String>(tid)
                            })
                        })
                        .await;
                    let _ = this.update(cx, |app, cx| {
                        match result {
                            Ok(_tid) => {
                                app.status_line = "thread/archive · done".into();
                            }
                            Err(e) => {
                                app.status_line = format!("thread/archive failed · {e}").into();
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
                return;
            }
        }
        if let Some(fixture) = self.fixture.clone().filter(|_| self.is_explicit_fixture()) {
            let tid = id;
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        fixture
                            .thread_archive(ThreadArchiveParams::new(tid))
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
            })
            .detach();
        }
        self.status_line = "thread/archive · local".into();
        cx.notify();
    }

    fn unarchive_thread_id(&mut self, id: String, cx: &mut Context<Self>) {
        if let Some(t) = self.threads.iter_mut().find(|t| t.summary.id == id) {
            t.summary.archived = Some(false);
        }
        self.status_line = "thread/unarchive…".into();

        if is_app_server_thread_id(&id) {
            if let Some(backend) = self.live_backend() {
                let tid = id;
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .map_err(|e| e.to_string())?;
                            rt.block_on(async {
                                backend
                                    .thread_unarchive(ThreadUnarchiveParams::new(tid.clone()))
                                    .await
                                    .map_err(|e| e.to_string())?;
                                Ok::<_, String>(tid)
                            })
                        })
                        .await;
                    let _ = this.update(cx, |app, cx| {
                        match result {
                            Ok(_tid) => {
                                app.status_line = "thread/unarchive · done".into();
                            }
                            Err(e) => {
                                app.status_line = format!("thread/unarchive failed · {e}").into();
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
                return;
            }
        }
        if let Some(fixture) = self.fixture.clone().filter(|_| self.is_explicit_fixture()) {
            let tid = id;
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        fixture
                            .thread_unarchive(ThreadUnarchiveParams::new(tid))
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
            })
            .detach();
        }
        self.status_line = "thread/unarchive · local".into();
        cx.notify();
    }

    /// Delete the selected thread (backend + local list).
    pub fn delete_selected_thread(&mut self, cx: &mut Context<Self>) {
        self.thread_menu_open = false;
        if self.live_backend().is_none() && !self.is_explicit_fixture() {
            self.status_line = "Delete is unavailable until a backend is ready.".into();
            cx.notify();
            return;
        }
        let Some(id) = self.selected_thread.clone() else {
            self.status_line = "Delete · no thread selected".into();
            cx.notify();
            return;
        };
        let live_session_id = self.live_session_id(&id);
        self.threads.retain(|t| t.summary.id != id);
        self.selected_thread = self.threads.first().map(|t| t.summary.id.clone());
        self.status_line = "thread/delete…".into();

        if let Some(session_id) = live_session_id {
            if let Some(backend) = self.live_backend() {
                let tid = id.clone();
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .map_err(|e| e.to_string())?;
                            rt.block_on(async {
                                backend
                                    .delete_session(&session_id)
                                    .await
                                    .map_err(|e| e.to_string())?;
                                Ok::<_, String>(tid)
                            })
                        })
                        .await;
                    let _ = this.update(cx, |app, cx| {
                        match result {
                            Ok(_tid) => {
                                app.status_line = "thread/delete · done".into();
                            }
                            Err(e) => {
                                app.status_line = format!("thread/delete failed · {e}").into();
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
                return;
            }
        }
        if let Some(fixture) = self.fixture.clone().filter(|_| self.is_explicit_fixture()) {
            let tid = id;
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        fixture
                            .thread_delete(ThreadDeleteParams::new(tid))
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
            })
            .detach();
        }
        self.status_line = "thread/delete · local".into();
        cx.notify();
    }

    /// Fork the selected thread into a new local (and backend) thread.
    pub fn fork_selected_thread(&mut self, cx: &mut Context<Self>) {
        self.thread_menu_open = false;
        if self.live_backend().is_none() && !self.is_explicit_fixture() {
            self.status_line = "Fork is unavailable until a backend is ready.".into();
            cx.notify();
            return;
        }
        if self
            .live_backend()
            .is_some_and(|backend| !backend.capabilities().fork)
        {
            self.status_line = "Fork is not supported by the selected backend.".into();
            cx.notify();
            return;
        }
        let Some(id) = self.selected_thread.clone() else {
            self.status_line = "Fork · no thread selected".into();
            cx.notify();
            return;
        };
        let Some(source) = self.threads.iter().find(|t| t.summary.id == id).cloned() else {
            self.status_line = "Fork · thread not found".into();
            cx.notify();
            return;
        };

        // Optimistic local fork; backend may replace id when Ready/Fixture.
        let fork_id = format!("fork-{}", self.threads.len() + 1);
        let mut forked = source;
        forked.summary.id = fork_id.clone();
        forked.backend_session_id = None;
        forked.summary.archived = Some(false);
        let base = forked
            .summary
            .name
            .clone()
            .unwrap_or_else(|| "Thread".into());
        forked.summary.name = Some(format!("{base} (fork)"));
        self.threads.insert(0, forked);
        self.selected_thread = Some(fork_id.clone());
        self.status_line = "thread/fork…".into();

        // Live thread/fork when Ready + real source id.
        if is_app_server_thread_id(&id) {
            if let Some(backend) = self.live_backend() {
                let tid = id.clone();
                let source_id = id;
                let local_id = fork_id;
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_spawn(async move {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .map_err(|e| e.to_string())?;
                            rt.block_on(async {
                                backend
                                    .thread_fork(ThreadForkParams::new(tid))
                                    .await
                                    .map_err(|e| e.to_string())
                            })
                        })
                        .await;
                    let _ = this.update(cx, |app, cx| {
                        match result {
                            Ok(resp) => {
                                let summary = resp.summary();
                                let backend_session_id = app.live_backend().map(|backend| {
                                    BackendSessionId::new(backend.kind(), summary.id.clone())
                                });
                                if let Some(t) =
                                    app.threads.iter_mut().find(|t| t.summary.id == local_id)
                                {
                                    t.summary = summary.clone();
                                    t.backend_session_id = backend_session_id;
                                }
                                let forked_id = summary.id.clone();
                                app.selected_thread = Some(summary.id);
                                app.status_line = "thread/fork · done".into();
                                // Pull turns for the new server thread if still empty.
                                if let Some(backend) = app.live_backend() {
                                    let empty = app
                                        .threads
                                        .iter()
                                        .find(|t| t.summary.id == forked_id)
                                        .map(|t| t.messages.is_empty())
                                        .unwrap_or(true);
                                    if empty {
                                        app.status_line = "thread/read…".into();
                                        app.load_thread_messages(backend, forked_id, cx);
                                    }
                                }
                            }
                            Err(e) => {
                                app.threads.retain(|thread| thread.summary.id != local_id);
                                app.selected_thread = Some(source_id.clone());
                                app.status_line = format!("thread/fork failed · {e}").into();
                            }
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
                return;
            }
        }
        if self.is_explicit_fixture() {
            let Some(fixture) = self.fixture.clone() else {
                return;
            };
            let tid = id;
            let local_id = fork_id;
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_spawn(async move {
                        fixture
                            .thread_fork(ThreadForkParams::new(tid))
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
                let _ = this.update(cx, |app, cx| {
                    if let Ok(resp) = result {
                        let summary = resp.summary();
                        if let Some(t) = app.threads.iter_mut().find(|t| t.summary.id == local_id) {
                            // Preserve messages; update summary/id from fixture.
                            let messages = std::mem::take(&mut t.messages);
                            t.summary = summary.clone();
                            t.messages = messages;
                        }
                        app.selected_thread = Some(summary.id);
                        app.status_line = "thread/fork · fixture".into();
                    } else {
                        app.status_line = format!("thread/fork · local {local_id}").into();
                    }
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
            return;
        }
        self.status_line = format!("thread/fork · local {fork_id}").into();
        cx.notify();
    }

    /// Stop / interrupt the in-progress turn (fixture cancel or live `turn/interrupt`).
    pub fn interrupt_turn(&mut self, cx: &mut Context<Self>) {
        if !self.turn_in_progress {
            self.status_line = "No turn in progress.".into();
            cx.notify();
            return;
        }

        // Cancel fixture stream replay.
        if let Some(flag) = &self.turn_cancel {
            flag.store(true, Ordering::SeqCst);
        }
        // Drop pending fixture resume so approvals don't continue after stop.
        self.fixture_resume = None;
        // Unblock live approval waiters with Abort so the runner can wind down.
        if let Some(bridge) = self.live_approval_bridge.take() {
            let _ = bridge.submit(ApprovalChoice::Abort);
        }

        let thread_id = self.active_turn_thread_id.clone();
        let turn_id = self.active_turn_id.clone();
        let live_session_id = thread_id.as_deref().and_then(|id| self.live_session_id(id));

        // Live path: call turn/interrupt when we know thread + turn ids.
        if let (Some(backend), Some(session_id), Some(turn)) =
            (self.backend.clone(), live_session_id, turn_id.clone())
        {
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .map_err(|e| e.to_string())?;
                        rt.block_on(async {
                            backend
                                .interrupt_session(&session_id, turn)
                                .await
                                .map_err(|e| e.to_string())
                        })
                    })
                    .await;
            })
            .detach();
        } else if let (Some(fixture), Some(tid), Some(turn)) = (
            self.fixture.clone().filter(|_| self.is_explicit_fixture()),
            thread_id.clone(),
            turn_id,
        ) {
            // Fixture: no-op success path for parity.
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        fixture
                            .turn_interrupt(TurnInterruptParams::new(tid, turn))
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
            })
            .detach();
        }

        // Immediately clear streaming UI state.
        if let Some(id) = thread_id {
            if let Some(thread) = self.threads.iter_mut().find(|t| t.summary.id == id) {
                for m in &mut thread.messages {
                    m.streaming = false;
                }
            }
        }
        self.turn_in_progress = false;
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.active_turn_thread_id = None;
        self.active_turn_id = None;
        self.turn_cancel = None;
        self.pending_approval = None;
        self.pending_user_input = None;
        self.user_input_answers.clear();
        self.pending_mcp_elicitation = None;
        self.mcp_form_values.clear();
        self.status_line = "Turn interrupted.".into();
        cx.notify();
    }

    /// Select a model by id (Settings list / chip).
    #[allow(dead_code)]
    pub fn select_model(&mut self, id: String, cx: &mut Context<Self>) {
        if self.models.iter().any(|m| m.id == id) {
            self.selected_model_id = Some(id);
            self.composer_model_menu_open = false;
            self.restore_reasoning_for_selected_model();
            self.restore_speed_for_selected_model();
            self.remember_selected_model();
            let label = self.model_label();
            self.status_line = format!("Model: {label}").into();
            cx.notify();
        }
    }

    /// Fill the composer with a suggestion chip prompt (empty-state affordance).
    #[allow(dead_code)]
    pub fn fill_composer(
        &mut self,
        input: &Entity<InputState>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owned = text.to_string();
        input.update(cx, |state, cx| {
            state.set_value(owned, window, cx);
        });
        self.status_line = "Prompt loaded into composer.".into();
        cx.notify();
    }

    pub fn submit_composer(
        &mut self,
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = input.read(cx).value().to_string();
        let trimmed = text.trim();
        if trimmed.is_empty() && self.composer_attachments.is_empty() {
            self.status_line = "Composer is empty.".into();
            cx.notify();
            return;
        }

        if self.turn_in_progress {
            if !self.composer_attachments.is_empty() {
                self.status_line = "Attachments cannot steer an active turn.".into();
                cx.notify();
                return;
            }
            self.submit_live_steer(input, trimmed.to_owned(), window, cx);
            return;
        }

        let mode = self.resolve_send_mode();
        if matches!(mode, SendMode::Unavailable) {
            self.status_line = match &self.connection {
                UiConnection::Connecting => "Send unavailable · backend is connecting.".into(),
                UiConnection::Error { message } => {
                    format!("Send unavailable · backend error: {message}").into()
                }
                UiConnection::Ready {
                    has_auth: false, ..
                } => "Send unavailable · backend is not authenticated.".into(),
                _ => "Send unavailable · select a connected backend.".into(),
            };
            cx.notify();
            return;
        }

        // Ensure we have a selected thread; prefer real app-server id (not local-*).
        if self.selected_thread.is_none() {
            // Kick async thread/start when Ready; fall through to local then promote on send.
            if matches!(self.connection, UiConnection::Ready { .. }) && self.backend.is_some() {
                // Synchronous local placeholder; promote_to_server before live turn if still local-*.
                self.new_thread_local(self.active_thread_surface(), cx);
            } else {
                self.new_thread(cx);
            }
        }
        let thread_id = self.selected_thread.clone().unwrap_or_default();
        if thread_id.is_empty() {
            self.status_line = "No thread available.".into();
            cx.notify();
            return;
        }

        let attachments = self
            .composer_attachments
            .iter()
            .map(|attachment| match attachment.kind {
                ComposerAttachmentKind::Image => ProductAttachment::LocalImage {
                    path: attachment.path.clone(),
                },
                ComposerAttachmentKind::Audio => ProductAttachment::LocalAudio {
                    path: attachment.path.clone(),
                },
                ComposerAttachmentKind::Skill => ProductAttachment::Skill {
                    name: attachment.name.clone(),
                    path: attachment.path.clone(),
                },
                ComposerAttachmentKind::Mention => ProductAttachment::Mention {
                    name: attachment.name.clone(),
                    path: attachment.path.clone(),
                },
            })
            .collect::<Vec<_>>();
        let attachment_names = self
            .composer_attachments
            .iter()
            .map(|attachment| attachment.name.clone())
            .collect::<Vec<_>>();
        let demo_images = self
            .composer_attachments
            .iter()
            .filter(|attachment| attachment.kind == ComposerAttachmentKind::Image)
            .map(|attachment| DemoImageAttachment {
                label: attachment.name.clone(),
                source: DemoImageSource::LocalPath(attachment.path.clone()),
            })
            .collect::<Vec<_>>();
        let demo_audio = self
            .composer_attachments
            .iter()
            .filter(|attachment| attachment.kind == ComposerAttachmentKind::Audio)
            .map(|attachment| DemoAudioAttachment {
                label: attachment.name.clone(),
                source: DemoAudioSource::LocalPath(attachment.path.clone()),
            })
            .collect::<Vec<_>>();
        let demo_references = self
            .composer_attachments
            .iter()
            .filter_map(|attachment| {
                let kind = match attachment.kind {
                    ComposerAttachmentKind::Skill => DemoReferenceKind::Skill,
                    ComposerAttachmentKind::Mention => DemoReferenceKind::Mention,
                    ComposerAttachmentKind::Image | ComposerAttachmentKind::Audio => return None,
                };
                Some(DemoReferenceAttachment {
                    kind,
                    name: attachment.name.clone(),
                    path: attachment.path.clone(),
                })
            })
            .collect::<Vec<_>>();
        let visible_user_text = if attachment_names.is_empty() {
            trimmed.to_owned()
        } else if trimmed.is_empty() {
            format!("Attachments · {}", attachment_names.join(", "))
        } else {
            format!("{trimmed}\n\nAttachments · {}", attachment_names.join(", "))
        };

        // Append user bubble immediately; auto-name from first message when still default.
        let mut auto_name: Option<String> = None;
        if let Some(thread) = self.threads.iter_mut().find(|t| t.summary.id == thread_id) {
            thread.messages.push(DemoMessage::user_with_attachments(
                trimmed,
                demo_images,
                demo_audio,
                demo_references,
            ));
            thread.summary.preview = Some(visible_user_text.chars().take(64).collect());
            let is_default_name = thread
                .summary
                .name
                .as_deref()
                .map(|n| n == "New thread" || n == "New chat" || n == "New fixture thread")
                .unwrap_or(true);
            if is_default_name {
                let name: String = visible_user_text.chars().take(48).collect();
                thread.summary.name = Some(name.clone());
                auto_name = Some(name);
            }
        }

        // Best-effort `thread/name/set` when renaming from first message (live or fixture).
        if let Some(name) = auto_name {
            self.set_thread_name_best_effort(&thread_id, name, cx);
        }

        input.update(cx, |state, cx| {
            state.set_value("", window, cx);
        });
        self.composer_attachments.clear();
        self.composer_add_menu_open = false;
        self.composer_access_menu_open = false;
        self.composer_model_menu_open = false;
        self.composer_reasoning_menu_open = false;

        let model_slug = self.selected_model_slug();
        let reasoning_effort = self.selected_reasoning_effort.clone();
        let speed_mode = self.selected_speed_mode();
        let work_mode = self.selected_work_mode();
        let working_dir = self.composer_workspace_dir().map(ToOwned::to_owned);
        let access_mode = self.composer_access_mode();
        if matches!(mode, SendMode::Live) && self.account.is_rate_limited_out() {
            // Still attempt live (server is source of truth) but surface the probe.
            self.status_line =
                "Live Send · rate limit 100% / no credits — server may refuse.".into();
        }
        self.turn_in_progress = true;
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.active_turn_thread_id = Some(thread_id.clone());
        match mode {
            SendMode::Live => {
                let model_note = model_slug
                    .as_deref()
                    .map(|m| format!(" · model={m}"))
                    .unwrap_or_default();
                // If UI still holds a local-* id, promote via thread/start then turn/start.
                if thread_id.starts_with("local-") {
                    self.status_line =
                        format!("Promoting local thread → app-server{model_note}…").into();
                    self.promote_local_then_live_turn(
                        thread_id,
                        trimmed.to_string(),
                        model_slug,
                        reasoning_effort,
                        speed_mode,
                        work_mode,
                        working_dir,
                        access_mode,
                        attachments,
                        cx,
                    );
                } else {
                    self.status_line = format!("Live turn/start{model_note}…").into();
                    self.start_live_turn(
                        thread_id,
                        trimmed.to_string(),
                        model_slug,
                        reasoning_effort,
                        speed_mode,
                        work_mode,
                        working_dir,
                        access_mode,
                        attachments,
                        cx,
                    );
                }
            }
            SendMode::Fixture => {
                self.status_line = "Streaming fixture turn…".into();
                self.start_fixture_turn(thread_id, cx);
            }
            SendMode::Unavailable => unreachable!("unavailable sends return before mutation"),
        }
        cx.notify();
    }

    fn submit_live_steer(
        &mut self,
        input: &Entity<InputState>,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(backend) = self.live_backend() else {
            self.status_line = "Steer unavailable · backend is not ready.".into();
            cx.notify();
            return;
        };
        let Some(thread_id) = self.active_turn_thread_id.clone() else {
            self.status_line = "Steer unavailable · active thread is unknown.".into();
            cx.notify();
            return;
        };
        let Some(expected_turn_id) = self.active_turn_id.clone() else {
            self.status_line = "Steer unavailable · active turn is not initialized yet.".into();
            cx.notify();
            return;
        };
        let Some(session_id) = self.live_session_id(&thread_id) else {
            self.status_line = "Steer unavailable · active session identity is missing.".into();
            cx.notify();
            return;
        };

        if let Some(thread) = self
            .threads
            .iter_mut()
            .find(|thread| thread.summary.id == thread_id)
        {
            thread.messages.push(DemoMessage::user(&text));
            thread.summary.preview = Some(text.chars().take(64).collect());
        }
        input.update(cx, |state, cx| state.set_value("", window, cx));
        self.status_line = "Steering active turn…".into();
        let backend_generation = self.backend_generation;
        let turn_generation = self.turn_generation;
        let request = ProductSteer {
            session_id,
            expected_turn_id,
            text,
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    backend
                        .steer_session(request)
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != backend_generation
                    || app.turn_generation != turn_generation
                {
                    return;
                }
                app.status_line = match result {
                    Ok(turn_id) => format!("Steered active turn · {turn_id}").into(),
                    Err(error) => {
                        if let Some(thread) = app
                            .threads
                            .iter_mut()
                            .find(|thread| thread.summary.id == thread_id)
                        {
                            thread.messages.push(DemoMessage::error(format!(
                                "Steering was not accepted: {error}"
                            )));
                        }
                        format!("Steer failed · {error}").into()
                    }
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Replace a `local-*` thread with a real app-server thread, then start live turn.
    fn promote_local_then_live_turn(
        &mut self,
        local_id: String,
        text: String,
        model: Option<String>,
        reasoning_effort: Option<String>,
        speed_mode: Option<ProductSpeedMode>,
        work_mode: Option<ProductWorkMode>,
        working_dir: Option<String>,
        access_mode: Option<ProductAccessMode>,
        attachments: Vec<ProductAttachment>,
        cx: &mut Context<Self>,
    ) {
        let turn_generation = self.turn_generation;
        let Some(backend) = self.backend.clone() else {
            self.turn_in_progress = false;
            self.active_turn_thread_id = None;
            self.status_line = "Session creation failed · no connected backend.".into();
            cx.notify();
            return;
        };
        let cwd = working_dir.or_else(|| {
            self.threads
                .iter()
                .find(|t| t.summary.id == local_id)
                .and_then(|t| t.summary.cwd.clone())
        });
        let model_for_start = model.clone();
        let speed_for_start = speed_mode.clone();
        cx.spawn(async move |this, cx| {
            let create_backend = Arc::clone(&backend);
            let result = cx
                .background_spawn(async move {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|e| e.to_string())?;
                    rt.block_on(async {
                        create_backend
                            .create_session(CreateSession {
                                working_dir: cwd,
                                model: model_for_start,
                                ephemeral: false,
                                access_mode,
                                speed_mode: speed_for_start,
                            })
                            .await
                            .map_err(|e| e.to_string())
                    })
                })
                .await;
            let _ = this.update(cx, |app, cx| match result {
                Ok(session) => {
                    if app.turn_generation != turn_generation {
                        delete_session_best_effort(backend, session.id, cx);
                        return;
                    }
                    let backend_session_id = session.id.clone();
                    let summary = thread_summary_from_session(session);
                    let new_id = summary.id.clone();
                    // Migrate local thread → server id (preserve user bubble).
                    if let Some(idx) = app.threads.iter().position(|t| t.summary.id == local_id) {
                        let mut t = app.threads.remove(idx);
                        t.summary = summary;
                        t.backend_session_id = Some(backend_session_id);
                        // Keep messages (user already appended).
                        app.threads.insert(0, t);
                    }
                    app.selected_thread = Some(new_id.clone());
                    app.active_turn_thread_id = Some(new_id.clone());
                    if let Some(mode) = app.composer_access_modes.remove(&local_id) {
                        app.composer_access_modes.insert(new_id.clone(), mode);
                    }
                    match app.active_thread_surface() {
                        ThreadSurface::Chat => app.selected_chat_thread = Some(new_id.clone()),
                        ThreadSurface::Codex => app.selected_codex_thread = Some(new_id.clone()),
                    }
                    app.status_line = format!("Live turn/start on {new_id}…").into();
                    app.start_live_turn(
                        new_id,
                        text,
                        model,
                        reasoning_effort,
                        speed_mode,
                        work_mode,
                        app.composer_workspace_dir().map(ToOwned::to_owned),
                        access_mode,
                        attachments,
                        cx,
                    );
                    cx.notify();
                }
                Err(e) => {
                    if app.turn_generation != turn_generation {
                        return;
                    }
                    if let Some(thread) = app
                        .threads
                        .iter_mut()
                        .find(|thread| thread.summary.id == local_id)
                    {
                        thread.messages.push(DemoMessage::error(format!(
                            "Could not create a server session: {e}"
                        )));
                    }
                    app.turn_in_progress = false;
                    app.turn_cancel = None;
                    app.active_turn_thread_id = None;
                    app.active_turn_id = None;
                    app.status_line = format!("Session creation failed: {e}").into();
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Live Codex backend only while connection is Ready.
    fn live_backend(&self) -> Option<Arc<DesktopBackend>> {
        if matches!(self.connection, UiConnection::Ready { .. }) {
            self.backend.clone()
        } else {
            None
        }
    }

    fn preferred_backend_selection(&self) -> mitsuro_desktop_backend::Result<BackendSelection> {
        if std::env::var_os("MITSURO_BACKEND").is_some() {
            return BackendSelection::from_env();
        }
        Ok(match self.preferences.selected_backend {
            Some(mitsuro_desktop_backend::BackendKind::CodexStdio) => BackendSelection::CodexStdio,
            Some(mitsuro_desktop_backend::BackendKind::CodexWebSocket) => {
                BackendSelection::CodexWebSocket
            }
            Some(mitsuro_desktop_backend::BackendKind::Fixture) => BackendSelection::Fixture,
            Some(mitsuro_desktop_backend::BackendKind::MitsuroHttp) | None => {
                BackendSelection::MitsuroHttp
            }
        })
    }

    fn save_preferences_best_effort(&self) {
        if let Err(error) = self.preferences.save_default() {
            eprintln!("[mitsuro] desktop preference save failed: {error}");
        }
    }

    fn remember_selected_model(&mut self) {
        let Some(kind) = self.active_backend_kind() else {
            return;
        };
        let Some(model_id) = self.selected_model_id.clone() else {
            return;
        };
        self.preferences.remember_model(kind, model_id);
        self.save_preferences_best_effort();
    }

    fn live_session_id(&self, ui_id: &str) -> Option<BackendSessionId> {
        self.threads
            .iter()
            .find(|thread| thread.summary.id == ui_id)
            .and_then(|thread| thread.backend_session_id.clone())
    }

    /// Best-effort `thread/name/set` (live app-server or fixture). UI already updated.
    fn set_thread_name_best_effort(
        &mut self,
        thread_id: &str,
        name: String,
        cx: &mut Context<Self>,
    ) {
        let tid = thread_id.to_string();
        if let Some(session_id) = self.live_session_id(&tid) {
            if let Some(backend) = self.live_backend() {
                cx.spawn(async move |_this, cx| {
                    let _ = cx
                        .background_spawn(async move {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .map_err(|e| e.to_string())?;
                            rt.block_on(async {
                                backend
                                    .rename_session(&session_id, name)
                                    .await
                                    .map_err(|e| e.to_string())
                            })
                        })
                        .await;
                })
                .detach();
                return;
            }
        }
        if let Some(fixture) = self.fixture.clone().filter(|_| self.is_explicit_fixture()) {
            cx.spawn(async move |_this, cx| {
                let _ = cx
                    .background_spawn(async move {
                        fixture
                            .thread_name_set(ThreadSetNameParams::new(tid, name))
                            .await
                            .map_err(|e| e.to_string())
                    })
                    .await;
            })
            .detach();
        }
    }

    fn resolve_send_mode(&self) -> SendMode {
        // Fixture replay is an explicit development mode. A user pressing Send on a
        // Ready/authenticated product backend starts a real turn on that backend.
        if std::env::var_os("MITSURO_NO_LIVE_TURN").is_some() {
            return SendMode::Unavailable;
        }
        let force_fixture = std::env::var_os("MITSURO_FORCE_FIXTURE").is_some();
        decide_send_mode(
            &self.connection,
            self.active_backend_kind(),
            self.backend.is_some(),
            force_fixture,
        )
    }

    fn start_fixture_turn(&mut self, thread_id: String, cx: &mut Context<Self>) {
        let turn_generation = self.turn_generation;
        self.active_turn_thread_id = Some(thread_id.clone());
        let delay = self
            .fixture
            .as_ref()
            .map(|f| f.stream_delay())
            .unwrap_or(Duration::from_millis(35));
        let cancel = Arc::new(AtomicBool::new(false));
        self.turn_cancel = Some(Arc::clone(&cancel));
        // Fixture sample uses turn ids from JSONL; track a stable id for interrupt RPC.
        self.active_turn_id = Some("turn-fixture-stream".into());

        cx.spawn(async move |this, cx| {
            let events = match load_sample_turn_events() {
                Ok(e) => e,
                Err(e) => {
                    let _ = this.update(cx, |app, cx| {
                        if !app.is_current_turn(turn_generation, &thread_id) {
                            return;
                        }
                        app.turn_in_progress = false;
                        app.turn_cancel = None;
                        if let Some(thread) = app
                            .threads
                            .iter_mut()
                            .find(|thread| thread.summary.id == thread_id)
                        {
                            thread.messages.push(DemoMessage::error(format!(
                                "Could not load the fixture turn: {e}"
                            )));
                        }
                        app.active_turn_thread_id = None;
                        app.active_turn_id = None;
                        app.status_line = format!("Fixture load error: {e}").into();
                        cx.notify();
                    });
                    return;
                }
            };
            // Rewrite thread_id on events so they bind to the active UI thread.
            let events: Vec<TurnStreamEvent> = events
                .into_iter()
                .map(|ev| rebind_thread_id(ev, &thread_id))
                .collect();

            replay_fixture_events(this, cx, thread_id, turn_generation, events, delay, cancel)
                .await;
        })
        .detach();
    }

    /// Resume fixture stream after the user answers an approval prompt.
    fn continue_fixture_events(
        &mut self,
        thread_id: String,
        events: Vec<TurnStreamEvent>,
        cx: &mut Context<Self>,
    ) {
        let delay = self
            .fixture
            .as_ref()
            .map(|f| f.stream_delay())
            .unwrap_or(Duration::from_millis(35));
        self.turn_in_progress = true;
        let turn_generation = self.turn_generation;
        let cancel = self
            .turn_cancel
            .clone()
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        self.turn_cancel = Some(Arc::clone(&cancel));
        cx.spawn(async move |this, cx| {
            replay_fixture_events(this, cx, thread_id, turn_generation, events, delay, cancel)
                .await;
        })
        .detach();
    }

    fn start_live_turn(
        &mut self,
        thread_id: String,
        text: String,
        model: Option<String>,
        reasoning_effort: Option<String>,
        speed_mode: Option<ProductSpeedMode>,
        work_mode: Option<ProductWorkMode>,
        working_dir: Option<String>,
        access_mode: Option<ProductAccessMode>,
        attachments: Vec<ProductAttachment>,
        cx: &mut Context<Self>,
    ) {
        let turn_generation = self.turn_generation;
        self.active_turn_thread_id = Some(thread_id.clone());
        let Some(backend) = self.backend.clone() else {
            self.turn_in_progress = false;
            self.active_turn_thread_id = None;
            self.status_line = "Live turn failed · backend disconnected.".into();
            cx.notify();
            return;
        };
        let Some(session_id) = self.live_session_id(&thread_id) else {
            self.turn_in_progress = false;
            self.active_turn_thread_id = None;
            self.status_line =
                "Live turn refused: the selected thread has no backend-qualified session id."
                    .into();
            cx.notify();
            return;
        };

        // Progressive path: apply events as they arrive; mid-stream approvals
        // surface ApprovalBar and block the turn loop until the user answers.
        let bridge = Arc::new(LiveApprovalBridge::new());
        self.live_approval_bridge = Some(Arc::clone(&bridge));

        cx.spawn(async move |this, cx| {
            /// Messages from the progressive live-turn producer thread.
            enum LiveMsg {
                Event(Box<TurnStreamEvent>),
                Finished(Result<mitsuro_desktop_backend::LiveTurnOutcome, String>),
            }

            let (msg_tx, msg_rx) = std::sync::mpsc::channel::<LiveMsg>();
            let msg_rx = Arc::new(std::sync::Mutex::new(msg_rx));

            // Producer: progressive live turn on a dedicated multi-thread runtime.
            // Selected model is forwarded as TurnStartParams.model (wire `model`).
            let _producer = cx.background_spawn({
                let backend = Arc::clone(&backend);
                let bridge = Arc::clone(&bridge);
                let text = text.clone();
                let model = model.clone();
                let reasoning_effort = reasoning_effort.clone();
                let speed_mode = speed_mode.clone();
                let work_mode = work_mode.clone();
                let attachments = attachments.clone();
                let msg_tx = msg_tx;
                async move {
                    let (event_tx, event_rx) = std::sync::mpsc::channel::<TurnStreamEvent>();
                    // Forward turn events onto the UI message channel.
                    let forward_tx = msg_tx.clone();
                    let forwarder = std::thread::spawn(move || {
                        while let Ok(ev) = event_rx.recv() {
                            if forward_tx.send(LiveMsg::Event(Box::new(ev))).is_err() {
                                break;
                            }
                        }
                    });

                    let result = backend
                        .run_product_turn_with_bridge_blocking(
                            ProductTurn {
                                session_id,
                                text,
                                model,
                                reasoning_effort,
                                speed_mode,
                                work_mode,
                                working_dir,
                                access_mode,
                                attachments,
                            },
                            event_tx,
                            bridge,
                            DEFAULT_LIVE_TURN_TIMEOUT,
                        )
                        .map_err(|e| e.to_string());

                    // Dropping event_tx (inside run_live… after return) ends forwarder once drained.
                    let _ = forwarder.join();
                    let _ = msg_tx.send(LiveMsg::Finished(result));
                }
            });

            // Consumer: apply each event to the UI as soon as it is produced.
            let mut saw_completed = false;
            let mut outcome: Option<Result<mitsuro_desktop_backend::LiveTurnOutcome, String>> =
                None;
            loop {
                let rx = Arc::clone(&msg_rx);
                let next = cx
                    .background_spawn(async move {
                        let guard = rx.lock().unwrap_or_else(|e| e.into_inner());
                        guard.recv()
                    })
                    .await;

                match next {
                    Ok(LiveMsg::Event(ev)) => {
                        let ev = *ev;
                        let done = matches!(ev, TurnStreamEvent::TurnCompleted { .. });
                        let is_approval = matches!(ev, TurnStreamEvent::ApprovalRequested(_));
                        let _ = this.update(cx, |app, cx| {
                            if !app.is_current_turn(turn_generation, &thread_id) {
                                return;
                            }
                            app.apply_stream_event(&thread_id, ev);
                            if is_approval {
                                app.turn_in_progress = true;
                                app.status_line = "Waiting for approval (live)…".into();
                            }
                            cx.notify();
                        });
                        if done {
                            saw_completed = true;
                        }
                    }
                    Ok(LiveMsg::Finished(result)) => {
                        outcome = Some(result);
                        break;
                    }
                    Err(_) => break,
                }
            }

            let outcome = outcome.unwrap_or_else(|| Err("live turn channel closed".into()));
            let _ = this.update(cx, |app, cx| {
                if !app.is_current_turn(turn_generation, &thread_id) {
                    return;
                }
                app.live_approval_bridge = None;
                match &outcome {
                    Ok(o) if o.completed || saw_completed => {
                        if app.pending_approval.is_none() {
                            app.turn_in_progress = false;
                            app.active_turn_thread_id = None;
                            app.active_turn_id = None;
                            app.turn_cancel = None;
                            app.status_line = "Live turn complete.".into();
                        }
                    }
                    Ok(_) => {
                        if app.pending_approval.is_none() {
                            app.turn_in_progress = false;
                            app.active_turn_thread_id = None;
                            app.active_turn_id = None;
                            app.turn_cancel = None;
                            app.status_line = "Live turn ended (timeout or closed).".into();
                        }
                    }
                    Err(e) => {
                        if let Some(thread) = app
                            .threads
                            .iter_mut()
                            .find(|candidate| candidate.summary.id == thread_id)
                        {
                            thread
                                .messages
                                .push(DemoMessage::error(format!("Live turn failed: {e}")));
                        }
                        app.turn_in_progress = false;
                        app.active_turn_thread_id = None;
                        app.active_turn_id = None;
                        app.turn_cancel = None;
                        app.status_line = format!("Live turn failed: {e}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_live_review(
        &mut self,
        thread_id: String,
        session_id: BackendSessionId,
        cx: &mut Context<Self>,
    ) {
        let turn_generation = self.turn_generation;
        let Some(backend) = self.backend.clone() else {
            self.turn_in_progress = false;
            self.active_turn_thread_id = None;
            self.status_line = "Review failed · backend disconnected.".into();
            cx.notify();
            return;
        };

        // Review is a real Codex turn. Subscribe before review/start and use the
        // normal approval bridge so streamed items and prompts remain interactive.
        let bridge = Arc::new(LiveApprovalBridge::new());
        self.live_approval_bridge = Some(Arc::clone(&bridge));

        cx.spawn(async move |this, cx| {
            enum LiveReviewMsg {
                Event(Box<TurnStreamEvent>),
                Finished(Result<mitsuro_desktop_backend::LiveReviewOutcome, String>),
            }

            let (msg_tx, msg_rx) = std::sync::mpsc::channel::<LiveReviewMsg>();
            let msg_rx = Arc::new(std::sync::Mutex::new(msg_rx));

            let _producer = cx.background_spawn({
                let backend = Arc::clone(&backend);
                let bridge = Arc::clone(&bridge);
                let msg_tx = msg_tx;
                async move {
                    let (event_tx, event_rx) = std::sync::mpsc::channel::<TurnStreamEvent>();
                    let forward_tx = msg_tx.clone();
                    let forwarder = std::thread::spawn(move || {
                        while let Ok(event) = event_rx.recv() {
                            if forward_tx
                                .send(LiveReviewMsg::Event(Box::new(event)))
                                .is_err()
                            {
                                break;
                            }
                        }
                    });

                    let result = backend
                        .run_product_review_with_bridge_blocking(
                            ProductReview {
                                session_id,
                                target: ProductReviewTarget::UncommittedChanges,
                                // The menu action reviews in the selected thread. The
                                // backend adapter also follows detached review ids for
                                // future surfaces that explicitly request them.
                                detached: false,
                            },
                            event_tx,
                            bridge,
                            DEFAULT_LIVE_TURN_TIMEOUT,
                        )
                        .map_err(|error| error.to_string());

                    let _ = forwarder.join();
                    let _ = msg_tx.send(LiveReviewMsg::Finished(result));
                }
            });

            let mut saw_completed = false;
            let mut outcome: Option<Result<mitsuro_desktop_backend::LiveReviewOutcome, String>> =
                None;
            loop {
                let rx = Arc::clone(&msg_rx);
                let next = cx
                    .background_spawn(async move {
                        let guard = rx.lock().unwrap_or_else(|error| error.into_inner());
                        guard.recv()
                    })
                    .await;

                match next {
                    Ok(LiveReviewMsg::Event(event)) => {
                        let event = *event;
                        let done = matches!(event, TurnStreamEvent::TurnCompleted { .. });
                        let is_approval = matches!(event, TurnStreamEvent::ApprovalRequested(_));
                        let _ = this.update(cx, |app, cx| {
                            if !app.is_current_turn(turn_generation, &thread_id) {
                                return;
                            }
                            app.apply_stream_event(&thread_id, event);
                            if is_approval {
                                app.turn_in_progress = true;
                                app.status_line = "Waiting for review approval (live)…".into();
                            }
                            cx.notify();
                        });
                        if done {
                            saw_completed = true;
                        }
                    }
                    Ok(LiveReviewMsg::Finished(result)) => {
                        outcome = Some(result);
                        break;
                    }
                    Err(_) => break,
                }
            }

            let outcome = outcome.unwrap_or_else(|| Err("live review channel closed".into()));
            let _ = this.update(cx, |app, cx| {
                if !app.is_current_turn(turn_generation, &thread_id) {
                    return;
                }
                app.live_approval_bridge = None;
                match &outcome {
                    Ok(review) if review.stream.completed || saw_completed => {
                        if app.pending_approval.is_none() {
                            app.turn_in_progress = false;
                            app.active_turn_thread_id = None;
                            app.active_turn_id = None;
                            app.turn_cancel = None;
                            app.status_line = "Review complete.".into();
                        }
                    }
                    Ok(_) => {
                        if app.pending_approval.is_none() {
                            app.turn_in_progress = false;
                            app.active_turn_thread_id = None;
                            app.active_turn_id = None;
                            app.turn_cancel = None;
                            app.status_line = "Review ended (timeout or closed).".into();
                        }
                    }
                    Err(error) => {
                        if let Some(thread) = app
                            .threads
                            .iter_mut()
                            .find(|candidate| candidate.summary.id == thread_id)
                        {
                            thread
                                .messages
                                .push(DemoMessage::error(format!("Review failed: {error}")));
                        }
                        app.turn_in_progress = false;
                        app.active_turn_thread_id = None;
                        app.active_turn_id = None;
                        app.turn_cancel = None;
                        app.status_line = format!("Review failed: {error}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn is_current_turn(&self, generation: u64, thread_id: &str) -> bool {
        turn_update_is_current(
            self.turn_generation,
            self.active_turn_thread_id.as_deref(),
            generation,
            thread_id,
        )
    }

    fn apply_stream_event(&mut self, thread_id: &str, event: TurnStreamEvent) {
        // Codex lifecycle notifications are owned by the application-lifetime
        // subscriber. The turn stream still carries them for non-GPUI clients,
        // but applying both subscriptions here would duplicate activity rows.
        if matches!(event, TurnStreamEvent::Lifecycle(_))
            && self.active_backend_kind() == Some(BackendKind::CodexStdio)
        {
            return;
        }
        let Some(idx) = self.threads.iter().position(|t| t.summary.id == thread_id) else {
            return;
        };

        if let TurnStreamEvent::DelegationEvent { event, .. } = &event {
            self.delegations
                .entry(thread_id.to_owned())
                .or_default()
                .apply_event(event);
        }

        let mut status_update: Option<String> = None;

        // Capture turn id before borrowing the thread mutably.
        if let TurnStreamEvent::TurnStarted { turn_id, .. } = &event {
            self.active_turn_id = Some(turn_id.clone());
            status_update = Some(format!("Turn {turn_id} started…"));
        }

        {
            let thread = &mut self.threads[idx];
            match event {
                TurnStreamEvent::TurnStarted { .. } => {
                    // status_update set above
                }
                TurnStreamEvent::ItemStarted {
                    item_id,
                    kind,
                    item,
                    ..
                } => match kind {
                    mitsuro_desktop_backend::ItemKind::AgentMessage => {
                        let exists = thread
                            .messages
                            .iter()
                            .any(|m| m.item_id.as_deref() == Some(item_id.as_str()));
                        if !exists {
                            thread
                                .messages
                                .push(DemoMessage::streaming_assistant(item_id));
                        }
                    }
                    mitsuro_desktop_backend::ItemKind::Reasoning => {
                        thread
                            .messages
                            .push(DemoMessage::reasoning(String::new(), Some(item_id)));
                        if let Some(m) = thread.messages.last_mut() {
                            m.streaming = true;
                        }
                    }
                    mitsuro_desktop_backend::ItemKind::Plan => {
                        thread
                            .messages
                            .push(DemoMessage::plan(String::new(), Some(item_id)));
                        if let Some(m) = thread.messages.last_mut() {
                            m.streaming = true;
                        }
                    }
                    mitsuro_desktop_backend::ItemKind::CommandExecution => {
                        let fields = item
                            .as_ref()
                            .map(command_execution_fields)
                            .unwrap_or_default();
                        let mut m = DemoMessage::command_execution(
                            fields.command,
                            fields.cwd,
                            fields.status,
                            fields.output,
                            Some(item_id),
                        );
                        m.streaming = true;
                        thread.messages.push(m);
                    }
                    mitsuro_desktop_backend::ItemKind::FileChange => {
                        let fields = item.as_ref().map(file_change_fields).unwrap_or_default();
                        let mut m = DemoMessage::file_change(
                            fields.paths_summary,
                            fields.patch_preview,
                            fields.status,
                            Some(item_id),
                        );
                        m.streaming = true;
                        thread.messages.push(m);
                    }
                    mitsuro_desktop_backend::ItemKind::UserMessage => {}
                    _ => {
                        let exists = thread
                            .messages
                            .iter()
                            .any(|message| message.item_id.as_deref() == Some(item_id.as_str()));
                        if !exists {
                            let mut message = activity_message(&kind, item_id, item.as_ref());
                            message.streaming = true;
                            thread.messages.push(message);
                        }
                    }
                },
                TurnStreamEvent::AgentMessageDelta { item_id, delta, .. } => {
                    if let Some(msg) = find_message_mut(&mut thread.messages, &item_id) {
                        msg.text_mut().push_str(&delta);
                        msg.streaming = true;
                    } else {
                        let mut m = DemoMessage::streaming_assistant(item_id);
                        m.set_text(delta);
                        thread.messages.push(m);
                    }
                }
                TurnStreamEvent::ReasoningTextDelta { item_id, delta, .. }
                | TurnStreamEvent::ReasoningSummaryDelta { item_id, delta, .. } => {
                    if let Some(msg) = find_message_mut(&mut thread.messages, &item_id) {
                        msg.text_mut().push_str(&delta);
                        msg.streaming = true;
                    } else {
                        let mut m = DemoMessage::reasoning(delta, Some(item_id));
                        m.streaming = true;
                        thread.messages.push(m);
                    }
                }
                TurnStreamEvent::PlanDelta { item_id, delta, .. } => {
                    if let Some(msg) = find_message_mut(&mut thread.messages, &item_id) {
                        msg.text_mut().push_str(&delta);
                        msg.streaming = true;
                    } else {
                        let mut m = DemoMessage::plan(delta, Some(item_id));
                        m.streaming = true;
                        thread.messages.push(m);
                    }
                }
                TurnStreamEvent::CommandExecutionOutputDelta { item_id, delta, .. } => {
                    if let Some(msg) = find_message_mut(&mut thread.messages, &item_id) {
                        if let DemoMessageKind::CommandExecution { output, .. } = &mut msg.kind {
                            output.push_str(&delta);
                        } else {
                            msg.text_mut().push_str(&delta);
                        }
                        msg.streaming = true;
                    } else {
                        let mut m = DemoMessage::command_execution(
                            "",
                            "",
                            "inProgress",
                            delta,
                            Some(item_id),
                        );
                        m.streaming = true;
                        thread.messages.push(m);
                    }
                }
                TurnStreamEvent::FileChangeOutputDelta { item_id, delta, .. } => {
                    if let Some(msg) = find_message_mut(&mut thread.messages, &item_id) {
                        if let DemoMessageKind::FileChange { patch_preview, .. } = &mut msg.kind {
                            // Legacy textual output; append below any structured patch.
                            if !patch_preview.is_empty() && !patch_preview.ends_with('\n') {
                                patch_preview.push('\n');
                            }
                            patch_preview.push_str(&delta);
                        } else {
                            msg.text_mut().push_str(&delta);
                        }
                        msg.streaming = true;
                    } else {
                        let mut m =
                            DemoMessage::file_change("", delta, "inProgress", Some(item_id));
                        m.streaming = true;
                        thread.messages.push(m);
                    }
                }
                TurnStreamEvent::FileChangePatchUpdated {
                    item_id, changes, ..
                } => {
                    let (paths, patch) = summarize_file_changes(Some(&changes));
                    if let Some(msg) = find_message_mut(&mut thread.messages, &item_id) {
                        if let DemoMessageKind::FileChange {
                            paths_summary,
                            patch_preview,
                            ..
                        } = &mut msg.kind
                        {
                            if !paths.is_empty() {
                                *paths_summary = paths;
                            }
                            if !patch.is_empty() {
                                *patch_preview = patch;
                            }
                        }
                        msg.streaming = true;
                    } else {
                        let mut m =
                            DemoMessage::file_change(paths, patch, "inProgress", Some(item_id));
                        m.streaming = true;
                        thread.messages.push(m);
                    }
                }
                TurnStreamEvent::ItemCompleted {
                    item_id,
                    text,
                    kind,
                    item,
                    ..
                } => {
                    if let Some(msg) = find_message_mut(&mut thread.messages, &item_id) {
                        match kind {
                            mitsuro_desktop_backend::ItemKind::CommandExecution => {
                                if let Some(raw) = item.as_ref() {
                                    let fields = command_execution_fields(raw);
                                    if let DemoMessageKind::CommandExecution {
                                        command,
                                        cwd,
                                        status,
                                        output,
                                    } = &mut msg.kind
                                    {
                                        if !fields.command.is_empty() {
                                            *command = fields.command;
                                        }
                                        if !fields.cwd.is_empty() {
                                            *cwd = fields.cwd;
                                        }
                                        *status = fields.status;
                                        if !fields.output.is_empty() {
                                            *output = fields.output;
                                        } else if let Some(t) = text {
                                            if !t.is_empty() {
                                                *output = t;
                                            }
                                        }
                                    }
                                }
                            }
                            mitsuro_desktop_backend::ItemKind::FileChange => {
                                if let Some(raw) = item.as_ref() {
                                    let fields = file_change_fields(raw);
                                    if let DemoMessageKind::FileChange {
                                        paths_summary,
                                        patch_preview,
                                        status,
                                    } = &mut msg.kind
                                    {
                                        if !fields.paths_summary.is_empty() {
                                            *paths_summary = fields.paths_summary;
                                        }
                                        if !fields.patch_preview.is_empty() {
                                            *patch_preview = fields.patch_preview;
                                        } else if let Some(t) = text {
                                            if !t.is_empty() {
                                                *patch_preview = t;
                                            }
                                        }
                                        *status = fields.status;
                                    }
                                }
                            }
                            _ if matches!(&msg.kind, DemoMessageKind::Activity { .. }) => {
                                let fields = item.as_ref().map(activity_item_fields).unwrap_or(
                                    ActivityFields {
                                        kind: kind.as_str().to_owned(),
                                        title: activity_title(kind.as_str()),
                                        summary: text.clone().unwrap_or_default(),
                                        status: String::new(),
                                    },
                                );
                                if let DemoMessageKind::Activity {
                                    kind,
                                    title,
                                    body,
                                    status,
                                } = &mut msg.kind
                                {
                                    *kind = fields.kind;
                                    *title = fields.title;
                                    *body = fields.summary;
                                    *status = fields.status;
                                }
                            }
                            _ => {
                                if let Some(final_text) = text {
                                    if !final_text.is_empty() {
                                        msg.set_text(final_text);
                                    }
                                }
                            }
                        }
                        msg.streaming = false;
                    } else if let Some(final_text) = text {
                        if !final_text.is_empty()
                            || matches!(
                                kind,
                                mitsuro_desktop_backend::ItemKind::CommandExecution
                                    | mitsuro_desktop_backend::ItemKind::FileChange
                            )
                        {
                            match kind {
                                mitsuro_desktop_backend::ItemKind::AgentMessage => {
                                    thread.messages.push(DemoMessage::assistant(final_text));
                                }
                                mitsuro_desktop_backend::ItemKind::Reasoning => {
                                    thread
                                        .messages
                                        .push(DemoMessage::reasoning(final_text, Some(item_id)));
                                }
                                mitsuro_desktop_backend::ItemKind::Plan => {
                                    thread
                                        .messages
                                        .push(DemoMessage::plan(final_text, Some(item_id)));
                                }
                                mitsuro_desktop_backend::ItemKind::CommandExecution => {
                                    let fields = item
                                        .as_ref()
                                        .map(command_execution_fields)
                                        .unwrap_or_default();
                                    let output = if fields.output.is_empty() {
                                        final_text
                                    } else {
                                        fields.output
                                    };
                                    thread.messages.push(DemoMessage::command_execution(
                                        fields.command,
                                        fields.cwd,
                                        fields.status,
                                        output,
                                        Some(item_id),
                                    ));
                                }
                                mitsuro_desktop_backend::ItemKind::FileChange => {
                                    let fields =
                                        item.as_ref().map(file_change_fields).unwrap_or_default();
                                    let patch = if fields.patch_preview.is_empty() {
                                        final_text
                                    } else {
                                        fields.patch_preview
                                    };
                                    thread.messages.push(DemoMessage::file_change(
                                        fields.paths_summary,
                                        patch,
                                        fields.status,
                                        Some(item_id),
                                    ));
                                }
                                mitsuro_desktop_backend::ItemKind::UserMessage => {}
                                _ => {
                                    thread.messages.push(activity_message(
                                        &kind,
                                        item_id,
                                        item.as_ref(),
                                    ));
                                }
                            }
                        }
                    } else if matches!(
                        kind,
                        mitsuro_desktop_backend::ItemKind::CommandExecution
                            | mitsuro_desktop_backend::ItemKind::FileChange
                    ) {
                        // Completed with empty text — still materialize structured block from item.
                        match kind {
                            mitsuro_desktop_backend::ItemKind::CommandExecution => {
                                let fields = item
                                    .as_ref()
                                    .map(command_execution_fields)
                                    .unwrap_or_default();
                                thread.messages.push(DemoMessage::command_execution(
                                    fields.command,
                                    fields.cwd,
                                    fields.status,
                                    fields.output,
                                    Some(item_id),
                                ));
                            }
                            mitsuro_desktop_backend::ItemKind::FileChange => {
                                let fields =
                                    item.as_ref().map(file_change_fields).unwrap_or_default();
                                thread.messages.push(DemoMessage::file_change(
                                    fields.paths_summary,
                                    fields.patch_preview,
                                    fields.status,
                                    Some(item_id),
                                ));
                            }
                            _ => {}
                        }
                    } else if !matches!(kind, mitsuro_desktop_backend::ItemKind::UserMessage) {
                        thread
                            .messages
                            .push(activity_message(&kind, item_id, item.as_ref()));
                    }
                }
                TurnStreamEvent::TurnCompleted { status, .. } => {
                    for m in &mut thread.messages {
                        m.streaming = false;
                    }
                    let label = status.unwrap_or_else(|| "completed".into());
                    let normalized = label.to_ascii_lowercase();
                    if normalized.contains("fail") || normalized.contains("error") {
                        thread.messages.push(DemoMessage::error(format!(
                            "The turn ended with status: {label}"
                        )));
                    }
                    status_update = Some(format!("Turn {label}."));
                }
                TurnStreamEvent::ApprovalRequested(pending) => {
                    self.pending_user_input = None;
                    self.pending_mcp_elicitation = None;
                    self.pending_approval = Some(pending.clone());
                    status_update = Some(format!(
                        "Approval required: {}",
                        pending.summary.chars().take(56).collect::<String>()
                    ));
                }
                TurnStreamEvent::UserInputRequested(pending) => {
                    self.pending_approval = None;
                    self.pending_mcp_elicitation = None;
                    self.pending_user_input = Some(pending.clone());
                    self.user_input_question_index = 0;
                    self.user_input_answers.clear();
                    status_update = Some(format!(
                        "Input requested · {} question{}",
                        pending.questions.len(),
                        if pending.questions.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    ));
                }
                TurnStreamEvent::McpElicitationRequested(pending) => {
                    self.pending_approval = None;
                    self.pending_user_input = None;
                    self.pending_mcp_elicitation = Some(pending.clone());
                    self.mcp_form_field_index = 0;
                    self.mcp_form_values.clear();
                    status_update = Some(format!(
                        "MCP request · {} · {}",
                        pending.server_name, pending.message
                    ));
                }
                // Process notifications (P10): surface in status; full terminal UI later.
                TurnStreamEvent::ProcessOutputDelta {
                    process_handle,
                    stream,
                    delta,
                    ..
                } => {
                    let preview: String = delta.chars().take(48).collect();
                    status_update =
                        Some(format!("Process {process_handle} · {stream:?}: {preview}"));
                }
                TurnStreamEvent::ProcessExited {
                    process_handle,
                    exit_code,
                    ..
                } => {
                    status_update = Some(format!(
                        "Process {process_handle} exited · code {exit_code}"
                    ));
                }
                TurnStreamEvent::DelegatedProgress { progress, .. } => {
                    status_update = Some(match progress.current_action.as_deref() {
                        Some(detail) => format!(
                            "{} · {} · {}",
                            progress.agent_name,
                            progress.status.label(),
                            detail
                        ),
                        None => {
                            format!("{} · {}", progress.agent_name, progress.status.label())
                        }
                    });
                }
                TurnStreamEvent::DelegationEvent { event, .. } => {
                    status_update = Some(format!(
                        "Delegation {} · {}",
                        event.group_id,
                        event.kind.label()
                    ));
                }
                TurnStreamEvent::Lifecycle(event) => {
                    if event.method == "serverRequest/resolved" {
                        self.pending_approval = None;
                        self.pending_user_input = None;
                        self.user_input_answers.clear();
                        self.pending_mcp_elicitation = None;
                        self.mcp_form_values.clear();
                    }
                    match event.method.as_str() {
                        "thread/name/updated" => {
                            if let Some(name) = event
                                .params
                                .as_ref()
                                .and_then(|params| params.get("name"))
                                .and_then(serde_json::Value::as_str)
                            {
                                thread.summary.name = Some(name.to_owned());
                            }
                        }
                        "thread/archived" | "thread/deleted" | "thread/closed" => {
                            thread.summary.archived = Some(true);
                        }
                        "thread/unarchived" => {
                            thread.summary.archived = Some(false);
                        }
                        _ => {}
                    }

                    if event.is_transcript_activity() {
                        let body = if event.detail.is_empty() {
                            event.method.clone()
                        } else {
                            event.detail.clone()
                        };
                        if matches!(
                            event.severity,
                            mitsuro_desktop_backend::NotificationSeverity::Error
                        ) {
                            thread.messages.push(DemoMessage::error(body));
                        } else {
                            thread.messages.push(DemoMessage::activity(
                                event.method.clone(),
                                event.title.clone(),
                                body,
                                format!("{:?}", event.severity).to_ascii_lowercase(),
                                event.item_id.clone(),
                            ));
                        }
                    }

                    if event.is_transcript_activity()
                        || matches!(
                            event.family,
                            mitsuro_desktop_backend::NotificationFamily::Account
                                | mitsuro_desktop_backend::NotificationFamily::RemoteControl
                        )
                    {
                        status_update = Some(if event.detail.is_empty() {
                            event.title
                        } else {
                            format!("{} · {}", event.title, event.detail)
                        });
                    }
                }
                TurnStreamEvent::Other { .. } => {}
            }
        }

        if let Some(line) = status_update {
            self.status_line = line.into();
        }
        if self.selected_thread.as_deref() == Some(thread_id) {
            self.transcript_scroll_handle.scroll_to_bottom();
        }
    }

    fn start_backend_lifecycle_listener(
        &mut self,
        backend: &Arc<DesktopBackend>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(mut events) = backend.subscribe_lifecycle_events() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let current = this
                    .update(cx, |app, cx| {
                        if app.backend_generation != generation {
                            return false;
                        }
                        app.apply_backend_lifecycle_event(event, cx);
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    break;
                }
            }
        })
        .detach();
    }

    fn apply_backend_lifecycle_event(
        &mut self,
        event: LifecycleNotification,
        cx: &mut Context<Self>,
    ) {
        if let Some(realtime) = RealtimeEvent::from_lifecycle(&event) {
            self.apply_realtime_event(realtime);
            cx.notify();
            return;
        }
        if event.method == "serverRequest/resolved" {
            self.pending_approval = None;
            self.pending_user_input = None;
            self.user_input_answers.clear();
            self.pending_mcp_elicitation = None;
            self.mcp_form_values.clear();
        }
        if event.method == "mcpServer/oauthLogin/completed" {
            if let Some(completion) = event
                .params
                .clone()
                .and_then(|params| serde_json::from_value(params).ok())
            {
                let completion: McpServerOauthLoginCompleted = completion;
                self.pending_mcp_oauth.remove(&completion.name);
                self.status_line = if completion.success {
                    format!("MCP · {} signed in", completion.name).into()
                } else {
                    format!(
                        "MCP · {} sign-in failed · {}",
                        completion.name,
                        completion.error.as_deref().unwrap_or("unknown error")
                    )
                    .into()
                };
            }
        }

        let login_completion = account_login_completion(&event);
        if let Some(completion) = login_completion.as_ref() {
            let matches_pending = self.account.pending_login_id.as_deref().is_none()
                || completion.login_id.is_none()
                || self.account.pending_login_id.as_deref() == completion.login_id.as_deref();
            if matches_pending {
                self.account.pending_login_id = None;
                self.account.pending_login_url = None;
                self.account.login_detail = if completion.success {
                    None
                } else {
                    Some(
                        completion
                            .error
                            .clone()
                            .unwrap_or_else(|| "Sign-in did not complete".to_owned()),
                    )
                };
            }
        }

        let thread_idx = event.thread_id.as_ref().and_then(|thread_id| {
            self.threads.iter().position(|thread| {
                thread.summary.id == *thread_id
                    || thread
                        .backend_session_id
                        .as_ref()
                        .is_some_and(|session| session.raw == *thread_id)
            })
        });
        if let Some(idx) = thread_idx {
            let thread = &mut self.threads[idx];
            match event.method.as_str() {
                "thread/name/updated" => {
                    if let Some(name) = event
                        .params
                        .as_ref()
                        .and_then(|params| params.get("name"))
                        .and_then(serde_json::Value::as_str)
                    {
                        thread.summary.name = Some(name.to_owned());
                    }
                }
                "thread/archived" | "thread/deleted" | "thread/closed" => {
                    thread.summary.archived = Some(true);
                }
                "thread/unarchived" => thread.summary.archived = Some(false),
                _ => {}
            }

            if event.is_transcript_activity() {
                let body = if event.detail.is_empty() {
                    event.method.clone()
                } else {
                    event.detail.clone()
                };
                if matches!(
                    event.severity,
                    mitsuro_desktop_backend::NotificationSeverity::Error
                ) {
                    thread.messages.push(DemoMessage::error(body));
                } else {
                    thread.messages.push(DemoMessage::activity(
                        event.method.clone(),
                        event.title.clone(),
                        body,
                        format!("{:?}", event.severity).to_ascii_lowercase(),
                        event.item_id.clone(),
                    ));
                }
            }
        }

        if event.is_transcript_activity()
            || matches!(
                event.family,
                mitsuro_desktop_backend::NotificationFamily::Account
                    | mitsuro_desktop_backend::NotificationFamily::RemoteControl
            )
        {
            self.status_line = if event.detail.is_empty() {
                event.title.clone().into()
            } else {
                format!("{} · {}", event.title, event.detail).into()
            };
        }

        if matches!(
            event.family,
            mitsuro_desktop_backend::NotificationFamily::Account
        ) && login_completion
            .as_ref()
            .is_none_or(|result| result.success)
            && matches!(self.connection, UiConnection::Ready { .. })
        {
            self.kick_account_refresh(cx);
        }
        if matches!(
            event.method.as_str(),
            "skills/changed"
                | "mcpServer/oauthLogin/completed"
                | "mcpServer/startupStatus/updated"
                | "app/list/updated"
        ) && matches!(self.connection, UiConnection::Ready { .. })
        {
            self.kick_extensions_refresh(cx);
        }
        if event.method == "thread/started" && matches!(self.connection, UiConnection::Ready { .. })
        {
            self.kick_thread_list_refresh(cx);
        }
        if event.method == "fs/changed" && self.active_mode == ProductMode::Files {
            self.files_refresh_directory_data(cx);
        }

        if self.selected_thread.as_deref() == event.thread_id.as_deref() {
            self.transcript_scroll_handle.scroll_to_bottom();
        }
    }

    fn apply_realtime_event(&mut self, event: RealtimeEvent) {
        match event {
            RealtimeEvent::Started { thread_id, .. } => {
                if let Some(runtime) = self
                    .realtime_voice_runtime
                    .as_mut()
                    .filter(|runtime| runtime.session_id.raw == thread_id)
                {
                    runtime.phase = RealtimeVoicePhase::Active;
                    self.status_line = "Voice chat active · listening".into();
                }
            }
            RealtimeEvent::TranscriptDelta {
                thread_id, delta, ..
            } => {
                if self
                    .realtime_voice_runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.session_id.raw == thread_id)
                {
                    let preview: String = delta.chars().take(48).collect();
                    self.status_line = format!("Voice transcript · {preview}").into();
                }
            }
            RealtimeEvent::TranscriptDone {
                thread_id,
                role,
                text,
            } => {
                if text.trim().is_empty() {
                    return;
                }
                let ui_thread_id = self
                    .realtime_voice_runtime
                    .as_ref()
                    .filter(|runtime| runtime.session_id.raw == thread_id)
                    .map(|runtime| runtime.ui_thread_id.clone())
                    .or_else(|| {
                        self.threads
                            .iter()
                            .find(|thread| {
                                thread
                                    .backend_session_id
                                    .as_ref()
                                    .is_some_and(|session| session.raw == thread_id)
                            })
                            .map(|thread| thread.summary.id.clone())
                    });
                if let Some(thread) = ui_thread_id.and_then(|ui_id| {
                    self.threads
                        .iter_mut()
                        .find(|thread| thread.summary.id == ui_id)
                }) {
                    if role.eq_ignore_ascii_case("user") {
                        thread.messages.push(DemoMessage::user(&text));
                        thread.summary.preview = Some(text.chars().take(64).collect());
                    } else {
                        thread.messages.push(DemoMessage::assistant(&text));
                    }
                }
                self.status_line = format!("Voice transcript · {role}").into();
                self.transcript_scroll_handle.scroll_to_bottom();
            }
            RealtimeEvent::OutputAudio { thread_id, audio } => {
                let Some(runtime) = self
                    .realtime_voice_runtime
                    .as_mut()
                    .filter(|runtime| runtime.session_id.raw == thread_id)
                else {
                    return;
                };
                let bytes = match base64::engine::general_purpose::STANDARD.decode(&audio.data) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        self.status_line =
                            format!("Voice playback data was invalid · {error}").into();
                        return;
                    }
                };
                let needs_player = runtime.playback.as_ref().is_none_or(|playback| {
                    playback.sample_rate != audio.sample_rate
                        || playback.channels != audio.num_channels
                });
                if needs_player {
                    match start_pipewire_playback(audio.sample_rate, audio.num_channels) {
                        Ok(audio_tx) => {
                            runtime.playback = Some(RealtimePlayback {
                                sample_rate: audio.sample_rate,
                                channels: audio.num_channels,
                                audio_tx,
                            });
                        }
                        Err(error) => {
                            self.status_line = error.into();
                            return;
                        }
                    }
                }
                if let Some(playback) = runtime.playback.as_ref() {
                    if playback.audio_tx.send(bytes).is_err() {
                        runtime.playback = None;
                        self.status_line = "Voice playback stream ended unexpectedly.".into();
                    }
                }
            }
            RealtimeEvent::Error { thread_id, message } => {
                let ui_thread_id = self
                    .realtime_voice_runtime
                    .as_ref()
                    .filter(|runtime| runtime.session_id.raw == thread_id)
                    .map(|runtime| runtime.ui_thread_id.clone());
                if let Some(runtime) = self.realtime_voice_runtime.take() {
                    runtime.capture_stop.store(true, Ordering::SeqCst);
                }
                if let Some(thread) = ui_thread_id.and_then(|ui_id| {
                    self.threads
                        .iter_mut()
                        .find(|thread| thread.summary.id == ui_id)
                }) {
                    thread
                        .messages
                        .push(DemoMessage::error(format!("Voice chat failed: {message}")));
                }
                self.status_line = format!("Voice chat error · {message}").into();
            }
            RealtimeEvent::Closed { thread_id, reason } => {
                if self
                    .realtime_voice_runtime
                    .as_ref()
                    .is_some_and(|runtime| runtime.session_id.raw == thread_id)
                {
                    if let Some(runtime) = self.realtime_voice_runtime.take() {
                        runtime.capture_stop.store(true, Ordering::SeqCst);
                    }
                    self.status_line = reason
                        .filter(|reason| !reason.is_empty())
                        .map(|reason| format!("Voice chat closed · {reason}"))
                        .unwrap_or_else(|| "Voice chat ended.".to_owned())
                        .into();
                }
            }
            RealtimeEvent::Sdp { .. } | RealtimeEvent::ItemAdded { .. } => {}
        }
    }

    fn kick_thread_list_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(backend) = self.live_backend() else {
            return;
        };
        let generation = self.backend_generation;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    backend
                        .list_sessions(100)
                        .await
                        .map_err(|error| error.to_string())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != generation {
                    return;
                }
                if let Ok(sessions) = result {
                    for session in sessions {
                        let raw_id = session.id.raw.clone();
                        let backend_session_id = session.id.clone();
                        let summary = thread_summary_from_session(session);
                        if let Some(thread) = app
                            .threads
                            .iter_mut()
                            .find(|thread| thread.summary.id == raw_id)
                        {
                            thread.backend_session_id = Some(backend_session_id);
                            thread.summary = summary;
                        } else {
                            app.threads.insert(
                                0,
                                DemoThread {
                                    backend_session_id: Some(backend_session_id),
                                    summary,
                                    surface: ThreadSurface::Codex,
                                    messages: Vec::new(),
                                },
                            );
                        }
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn bootstrap_fixture(&mut self, cx: &mut Context<Self>) {
        let fixture = self
            .fixture
            .clone()
            .unwrap_or_else(|| Arc::new(FixtureBackend::new()));
        self.fixture = Some(Arc::clone(&fixture));
        self.connection = UiConnection::Fixture;
        // Quiet status — connection chip carries "Local"; no fixture pill wall.
        self.status_line = SharedString::from("");
        self.realtime_voices = None;
        self.realtime_voices_state = SurfaceDataState::Unsupported;
        self.realtime_voices = None;
        self.realtime_voices_state = SurfaceDataState::Unsupported;

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    fixture.connect().await.map_err(|e| e.to_string())?;
                    let list = fixture
                        .thread_list(ThreadListParams::default())
                        .await
                        .map_err(|e| e.to_string())?;
                    let models = fixture
                        .model_list(ModelListParams::default())
                        .await
                        .map_err(|e| e.to_string())?;
                    let config = fixture
                        .config_read(ConfigReadParams {
                            include_layers: Some(true),
                            ..Default::default()
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                    let skills = fixture
                        .skills_list(SkillsListParams::default())
                        .await
                        .map_err(|e| e.to_string())?;
                    let mcp = fixture
                        .mcp_server_status_list(ListMcpServerStatusParams::default())
                        .await
                        .map_err(|e| e.to_string())?;
                    let plugins = fixture
                        .plugin_list(PluginListParams::default())
                        .await
                        .map_err(|e| e.to_string())?;
                    let account = fixture
                        .account_read(GetAccountParams::default())
                        .await
                        .map_err(|e| e.to_string())?;
                    let usage = fixture
                        .account_usage_read()
                        .await
                        .map_err(|e| e.to_string())?;
                    let rate_limits = fixture
                        .account_rate_limits_read()
                        .await
                        .map_err(|e| e.to_string())?;
                    Ok::<_, String>((
                        list.threads(),
                        models.data,
                        config.settings_snippet(),
                        skills
                            .data
                            .into_iter()
                            .flat_map(|e| e.skills)
                            .collect::<Vec<_>>(),
                        mcp.data,
                        plugins
                            .marketplaces
                            .into_iter()
                            .flat_map(|m| m.plugins)
                            .collect::<Vec<_>>(),
                        account.account,
                        usage,
                        rate_limits,
                    ))
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                if let Ok((
                    remote,
                    models,
                    config_snip,
                    skills,
                    mcp,
                    plugins,
                    account,
                    usage,
                    rate_limits,
                )) = result
                {
                    // Keep first paint calm: do not dump fixture/remote threads into the sidebar.
                    // Threads appear when the user creates one or loads samples.
                    let _ = remote;
                    app.apply_models(models);
                    app.apply_config_snippet(config_snip);
                    app.apply_skills(skills);
                    app.apply_mcp_servers(mcp);
                    app.apply_plugins(plugins);
                    app.apply_account_snapshot(account, usage, rate_limits, "fixture", None);
                    app.extensions_state = SurfaceDataState::Fixture;
                    app.account_state = SurfaceDataState::Fixture;
                } else {
                    // Seed demo catalog even if connect path failed.
                    app.apply_models(fixture_demo_models());
                    app.apply_config_snippet(fixture_demo_config().settings_snippet());
                    app.apply_skills(
                        fixture_demo_skills()
                            .data
                            .into_iter()
                            .flat_map(|e| e.skills)
                            .collect(),
                    );
                    app.apply_mcp_servers(fixture_demo_mcp_servers().data);
                    app.apply_plugins(
                        fixture_demo_plugins()
                            .marketplaces
                            .into_iter()
                            .flat_map(|m| m.plugins)
                            .collect(),
                    );
                    app.account = AccountSession::fixture_demo();
                    app.extensions_state = SurfaceDataState::Fixture;
                    app.account_state = SurfaceDataState::Fixture;
                }
                app.goals = demo::demo_goals();
                app.selected_goal = app.goals.first().map(|goal| goal.id.clone());
                app.environments = fixture_demo_environments();
                app.environments_state = SurfaceDataState::Fixture;
                app.selected_environment_id = app
                    .environments
                    .first()
                    .map(|environment| environment.id.clone());
                app.collaboration_modes = fixture_demo_collaboration_modes().data;
                app.files.cwd = FIXTURE_PROJECT_ROOT.into();
                app.connection = UiConnection::Fixture;
                // Single quiet status; counts live in Settings, not title chrome.
                app.status_line = SharedString::from("");
                cx.notify();
            });
        })
        .detach();
    }

    fn clear_live_backend_state(&mut self, kind: BackendKind) {
        if let Some(cancel) = self.turn_cancel.take() {
            cancel.store(true, Ordering::SeqCst);
        }
        if let Some(bridge) = self.live_approval_bridge.take() {
            let _ = bridge.submit(ApprovalChoice::Abort);
        }
        self.turn_generation = self.turn_generation.wrapping_add(1);
        self.threads.clear();
        self.expanded_transcript_messages.clear();
        self.selected_thread = None;
        self.selected_chat_thread = None;
        self.selected_codex_thread = None;
        self.models.clear();
        self.selected_model_id = None;
        self.selected_reasoning_effort = None;
        self.selected_fast_mode = false;
        self.config_snippet = SharedString::from("");
        self.skills.clear();
        self.mcp_servers.clear();
        self.pending_mcp_oauth.clear();
        self.plugins.clear();
        self.extensions_state = SurfaceDataState::Loading;
        self.plugin_mutation_in_progress = None;
        self.expanded_plugin_sections.clear();
        self.goals.clear();
        self.selected_goal = None;
        self.goals_are_live_hive = false;
        self.hive_snapshot = None;
        self.environments.clear();
        self.environment_add_in_progress = false;
        self.environments_state = SurfaceDataState::Loading;
        self.selected_environment_id = None;
        self.environment_status_detail = None;
        self.environment_info_detail = None;
        self.collaboration_modes.clear();
        self.composer_plan_mode = false;
        self.realtime_voices = None;
        self.realtime_voices_state = if kind == BackendKind::MitsuroHttp {
            SurfaceDataState::Unsupported
        } else {
            SurfaceDataState::Loading
        };
        if let Some(runtime) = self.realtime_voice_runtime.take() {
            runtime.capture_stop.store(true, Ordering::SeqCst);
        }
        self.realtime_voice_generation = self.realtime_voice_generation.wrapping_add(1);
        self.scheduled_tasks = None;
        self.background_processes.clear();
        self.terminal = TerminalSession::idle(kind.id());
        self.files = FilesSession::new(kind.id());
        self.pending_approval = None;
        self.pending_user_input = None;
        self.user_input_answers.clear();
        self.pending_mcp_elicitation = None;
        self.mcp_form_values.clear();
        self.fixture_resume = None;
        self.active_turn_thread_id = None;
        self.active_turn_id = None;
        self.turn_in_progress = false;
        self.composer_attachments.clear();
        self.composer_add_menu_open = false;
        self.composer_model_menu_open = false;
        self.composer_reasoning_menu_open = false;
        self.composer_default_workspace_dir = std::env::current_dir()
            .ok()
            .map(|path| path.display().to_string());
        self.composer_default_access_mode = None;
        self.composer_access_modes.clear();
        self.composer_access_menu_open = false;
        self.account = AccountSession::empty(kind.id());
        self.account_state = SurfaceDataState::Loading;
    }

    fn bootstrap_backend(&mut self, cx: &mut Context<Self>) {
        let selection = match self.preferred_backend_selection() {
            Ok(selection) => selection,
            Err(error) => {
                self.clear_live_backend_state(BackendKind::MitsuroHttp);
                self.connection = UiConnection::Error {
                    message: error.to_string(),
                };
                self.status_line = format!("Backend configuration error: {error}").into();
                return;
            }
        };
        self.connect_backend_selection(selection, cx);
    }

    fn connect_backend_selection(&mut self, selection: BackendSelection, cx: &mut Context<Self>) {
        if matches!(selection, BackendSelection::Fixture) {
            let previous_backend = self.backend.take();
            self.backend_generation = self.backend_generation.wrapping_add(1);
            self.clear_live_backend_state(BackendKind::Fixture);
            self.connection = UiConnection::Fixture;
            self.status_line = "Fixture backend selected explicitly.".into();
            self.bootstrap_fixture(cx);
            disconnect_backend_best_effort(previous_backend, cx);
            return;
        }
        if matches!(selection, BackendSelection::CodexWebSocket) {
            let previous_backend = self.backend.take();
            self.backend_generation = self.backend_generation.wrapping_add(1);
            self.clear_live_backend_state(BackendKind::CodexWebSocket);
            self.connection = UiConnection::Error {
                message: "codex-ws is not implemented".to_owned(),
            };
            self.status_line =
                "codex-ws is not implemented yet; use codex-stdio or mitsuro-http.".into();
            disconnect_backend_best_effort(previous_backend, cx);
            cx.notify();
            return;
        }
        let backend = match selection {
            BackendSelection::CodexStdio => DesktopBackend::codex_stdio(),
            BackendSelection::Auto | BackendSelection::MitsuroHttp => {
                match DesktopBackend::mitsuro_from_env() {
                    Ok(backend) => backend,
                    Err(error) => {
                        self.clear_live_backend_state(BackendKind::MitsuroHttp);
                        self.connection = UiConnection::Error {
                            message: error.to_string(),
                        };
                        self.status_line =
                            format!("Mitsuro backend configuration error: {error}").into();
                        cx.notify();
                        return;
                    }
                }
            }
            BackendSelection::CodexWebSocket | BackendSelection::Fixture => unreachable!(),
        };
        let backend = Arc::new(backend);
        let previous_backend = self.backend.take();
        self.backend_generation = self.backend_generation.wrapping_add(1);
        let generation = self.backend_generation;
        let backend_label = backend.kind().id();
        self.backend = Some(Arc::clone(&backend));
        self.start_backend_lifecycle_listener(&backend, generation, cx);
        self.clear_live_backend_state(backend.kind());
        self.connection = UiConnection::Connecting;
        self.status_line = format!("Connecting to {backend_label}…").into();
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    if let Some(previous) = previous_backend {
                        let runner = Arc::clone(&previous);
                        let _ = previous.block_on(async move { runner.disconnect().await });
                    }
                    connect_list_auth_and_models(backend)
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                if app.backend_generation != generation {
                    return;
                }
                match result {
                    Ok(bootstrap) => {
                        let BackendBootstrap {
                            init,
                            sessions: remote,
                            has_auth,
                            models,
                            collaboration_modes,
                            realtime_voices,
                            config_snip,
                            skills,
                            mcp,
                            plugins,
                            processes,
                            hive,
                            schedules,
                        } = bootstrap;
                        eprintln!(
                            "[mitsuro] Connected backend={} os={} threads={} auth={} models={}",
                            app.backend
                                .as_ref()
                                .map(|backend| backend.kind().id())
                                .unwrap_or("none"),
                            init.platform_os,
                            remote.len(),
                            has_auth,
                            models.len()
                        );
                        app.connection = UiConnection::Ready {
                            detail: format!("{} · {}", init.platform_os, init.user_agent),
                            has_auth,
                        };
                        if let Some(backend_kind) =
                            app.backend.as_ref().map(|backend| backend.kind())
                        {
                            app.preferences.remember_backend(backend_kind);
                            app.save_preferences_best_effort();
                        }
                        app.threads = remote
                            .into_iter()
                            .map(|session| DemoThread {
                                backend_session_id: Some(session.id.clone()),
                                summary: thread_summary_from_session(session),
                                surface: ThreadSurface::Codex,
                                messages: vec![],
                            })
                            .collect();
                        // Default: leave selected_thread None → calm home hero.
                        // Override via MITSURO_START_THREAD / START_MODE=thread-open.
                        app.apply_models(
                            models.into_iter().map(model_info_from_product).collect(),
                        );
                        app.collaboration_modes = collaboration_modes;
                        match realtime_voices {
                            Ok(Some(voices)) => app.apply_realtime_voices(voices),
                            Ok(None) => {
                                app.realtime_voices = None;
                                app.realtime_voices_state = SurfaceDataState::Unsupported;
                            }
                            Err(_) => {
                                app.realtime_voices = None;
                                app.realtime_voices_state = SurfaceDataState::Error;
                            }
                        }
                        if let Some(backend) = app.active_backend_kind() {
                            app.composer_plan_mode = app.preferences.plan_mode_for(backend);
                        }
                        if let Some(snip) = config_snip {
                            app.apply_config_snippet(snip);
                        }
                        app.apply_skills(skills);
                        app.apply_mcp_servers(mcp);
                        app.apply_plugins(plugins);
                        app.extensions_state = SurfaceDataState::Live;
                        // Neither live transport exposes environment/list. Do not invent rows.
                        app.environments.clear();
                        app.environments_state = SurfaceDataState::Unsupported;
                        app.selected_environment_id = None;
                        let is_mitsuro = app
                            .backend
                            .as_ref()
                            .is_some_and(|backend| backend.kind() == BackendKind::MitsuroHttp);
                        if is_mitsuro {
                            app.terminal.backend_label = "mitsuro-http · read-only catalog".into();
                            match processes {
                                Some(processes) => {
                                    app.terminal.output = process_catalog_text(&processes).into();
                                    app.background_processes = processes;
                                }
                                None => {
                                    app.terminal.output = "Mitsuro background-process catalog is unavailable.\nInteractive terminal spawning is not exposed by this backend.".into();
                                    app.background_processes.clear();
                                }
                            }
                            app.goals_are_live_hive = true;
                            match hive {
                                Some(hive) => {
                                    app.goals = hive_goals_from_snapshot(&hive);
                                    app.selected_goal =
                                        app.goals.first().map(|goal| goal.id.clone());
                                    app.hive_snapshot = Some(hive);
                                }
                                None => {
                                    app.goals.clear();
                                    app.selected_goal = None;
                                    app.hive_snapshot = None;
                                }
                            }
                            // Some(empty) intentionally keeps the live, mutation-disabled schedule
                            // surface instead of silently falling back to fixture suggestions.
                            app.scheduled_tasks = Some(schedules.unwrap_or_default());
                        } else {
                            app.background_processes.clear();
                            app.goals.clear();
                            app.selected_goal = None;
                            app.goals_are_live_hive = false;
                            app.hive_snapshot = None;
                            app.scheduled_tasks = None;
                        }
                        let auth_note = if has_auth { "auth" } else { "no auth" };
                        // Short chrome: Connected · N threads · auth (counts for models/skills live in Settings).
                        app.status_line =
                            format!("Connected · {} threads · {auth_note}", app.threads.len(),)
                                .into();
                        if app.active_mode == ProductMode::Files {
                            app.files.cwd = app.preferred_workspace_cwd().into();
                            app.files.backend_label = app.files_backend_label();
                            app.files_refresh_directory_data(cx);
                        }
                        // Best-effort account snapshot from app-server (needs Window for public API —
                        // use internal spawn path via refresh_account with a no-op when possible).
                        app.kick_account_refresh(cx);
                        // Defer open-thread so first Connected paint (recents + chrome) settles
                        // before thread/read materializes bubbles (GNOME hang detector).
                        if app.pending_start_thread.is_some() {
                            cx.spawn(async move |this, cx| {
                                let _ = cx
                                    .background_spawn(async {
                                        std::thread::sleep(std::time::Duration::from_millis(600));
                                    })
                                    .await;
                                let _ = this.update(cx, |app, cx| {
                                    app.apply_pending_start_thread(cx);
                                });
                            })
                            .detach();
                        }
                    }
                    Err(message) => {
                        eprintln!("[mitsuro] backend connect failed: {message}");
                        app.connection = UiConnection::Error {
                            message: message.clone(),
                        };
                        app.account = AccountSession::empty("unavailable");
                        app.account_state = SurfaceDataState::Error;
                        app.extensions_state = SurfaceDataState::Error;
                        app.environments_state = SurfaceDataState::Error;
                        app.status_line = format!("Backend unavailable · {message}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Apply `pending_start_thread` after Recents are populated (bootstrap).
    ///
    /// Resolves:
    /// - `@first` → first non-archived thread
    /// - exact id match
    /// - case-insensitive title / preview substring
    fn apply_pending_start_thread(&mut self, cx: &mut Context<Self>) {
        let Some(want) = self.pending_start_thread.take() else {
            return;
        };
        if self.threads.is_empty() {
            // Put back so a later list refresh could retry (rare).
            self.pending_start_thread = Some(want);
            return;
        }

        let want_trim = want.trim();
        let qualified = BackendSessionId::parse_qualified(want_trim).ok();
        let resolved = if want_trim == "@first" || want_trim.is_empty() {
            self.threads
                .iter()
                .find(|t| t.summary.archived != Some(true))
                .or_else(|| self.threads.first())
                .map(|t| t.summary.id.clone())
        } else if let Some(t) = qualified.as_ref().and_then(|session| {
            self.threads
                .iter()
                .find(|thread| thread.backend_session_id.as_ref() == Some(session))
        }) {
            Some(t.summary.id.clone())
        } else if let Some(t) = self.threads.iter().find(|t| t.summary.id == want_trim) {
            Some(t.summary.id.clone())
        } else {
            let needle = want_trim.to_ascii_lowercase();
            self.threads
                .iter()
                .find(|t| {
                    t.summary
                        .display_title()
                        .to_ascii_lowercase()
                        .contains(&needle)
                        || t.summary
                            .name
                            .as_ref()
                            .map(|n| n.to_ascii_lowercase().contains(&needle))
                            .unwrap_or(false)
                        || t.summary
                            .preview
                            .as_ref()
                            .map(|p| p.to_ascii_lowercase().contains(&needle))
                            .unwrap_or(false)
                })
                .map(|t| t.summary.id.clone())
        };

        match resolved {
            Some(id) => {
                eprintln!("[mitsuro] open-thread select → {id} (from {want_trim})");
                self.select_thread(id, cx);
            }
            None => {
                self.status_line = format!(
                    "START_THREAD {want_trim:?} not in Recents ({} threads).",
                    self.threads.len()
                )
                .into();
            }
        }
    }

    /// Load transcript for a server thread via `thread/read` (includeTurns).
    /// Used when selecting an empty cached server thread while Ready.
    fn load_thread_messages(
        &mut self,
        backend: Arc<DesktopBackend>,
        thread_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self.live_session_id(&thread_id) else {
            self.status_line =
                "Session read refused: the thread has no backend-qualified identity.".into();
            cx.notify();
            return;
        };
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    // Keep transcript work off the UI thread, but preserve the
                    // canonical history instead of silently dropping and
                    // truncating nearly all server messages.
                    let tid = thread_id.clone();
                    let b = Arc::clone(&backend);
                    let prepared = backend.block_on(async move {
                        match b.read_session(&session_id).await {
                            Ok(conversation) => {
                                let delegation = conversation.delegation;
                                let delegation_status = delegation_hydration_status(&delegation);
                                let msgs = conversation.messages;
                                let seen = msgs.len();
                                eprintln!(
                                    "[mitsuro] thread/read ok id={} tail={} scanned={}",
                                    tid,
                                    msgs.len(),
                                    seen
                                );
                                let n_chat = msgs.len();
                                let ui: Vec<DemoMessage> = msgs
                                    .into_iter()
                                    .map(|m| {
                                        let mut msg = match m.role {
                                            MessageRole::User => {
                                                DemoMessage::user_with_attachments(
                                                    m.body,
                                                    demo_image_attachments(m.images),
                                                    demo_audio_attachments(m.audio),
                                                    demo_reference_attachments(m.references),
                                                )
                                            }
                                            MessageRole::Assistant => {
                                                DemoMessage::assistant(m.body)
                                            }
                                            MessageRole::Activity => {
                                                let fields = m.activity.unwrap_or(ActivityFields {
                                                    kind: "activity".to_owned(),
                                                    title: "Activity".to_owned(),
                                                    summary: m.body,
                                                    status: String::new(),
                                                });
                                                DemoMessage::activity(
                                                    fields.kind,
                                                    fields.title,
                                                    fields.summary,
                                                    fields.status,
                                                    m.item_id.clone(),
                                                )
                                            }
                                            MessageRole::Reasoning => {
                                                DemoMessage::reasoning(m.body, m.item_id.clone())
                                            }
                                            MessageRole::Plan => {
                                                DemoMessage::plan(m.body, m.item_id.clone())
                                            }
                                            MessageRole::CommandExecution => {
                                                let fields = m.command.unwrap_or_default();
                                                DemoMessage::command_execution(
                                                    fields.command,
                                                    fields.cwd,
                                                    fields.status,
                                                    fields.output,
                                                    m.item_id.clone(),
                                                )
                                            }
                                            MessageRole::FileChange => {
                                                let fields = m.file_change.unwrap_or_default();
                                                DemoMessage::file_change(
                                                    fields.paths_summary,
                                                    fields.patch_preview,
                                                    fields.status,
                                                    m.item_id.clone(),
                                                )
                                            }
                                        };
                                        if msg.item_id.is_none() {
                                            msg.item_id = m.item_id;
                                        }
                                        msg
                                    })
                                    .collect();
                                eprintln!(
                                    "[mitsuro] thread/read prepared id={} scanned={} ui={}",
                                    tid,
                                    seen,
                                    ui.len()
                                );
                                Ok::<_, String>((
                                    tid,
                                    seen.max(n_chat),
                                    ui,
                                    delegation,
                                    delegation_status,
                                ))
                            }
                            Err(e) => {
                                eprintln!("[mitsuro] thread/read failed id={tid}: {e}");
                                Err(e.to_string())
                            }
                        }
                    });
                    // Let "Loading thread…" paint before attaching bubbles.
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    prepared
                })
                .await;

            let _ = this.update(cx, |app, cx| {
                match result {
                    Ok((tid, n_in, ui_msgs, delegation, delegation_status)) => {
                        if let Some(thread) = app.threads.iter_mut().find(|t| t.summary.id == tid) {
                            thread.messages = ui_msgs;
                            app.delegations.insert(tid.clone(), delegation);
                            app.selected_thread = Some(tid.clone());
                            app.transcript_scroll_handle.scroll_to_bottom();
                            app.selected_codex_thread = Some(tid.clone());
                            if !matches!(app.active_mode, ProductMode::Codex | ProductMode::Chat) {
                                app.active_mode = ProductMode::Codex;
                            }
                            eprintln!(
                                "[mitsuro] thread/read applied id={} server={} ui={}",
                                tid,
                                n_in,
                                thread.messages.len()
                            );
                            let transcript_status = format!(
                                "thread/read · {} msgs (of {n_in})",
                                thread.messages.len(),
                            );
                            app.status_line = delegation_status
                                .map(|status| format!("{transcript_status} · {status}"))
                                .unwrap_or(transcript_status)
                                .into();
                        } else {
                            eprintln!(
                                "[mitsuro] thread/read MISSING sidebar thread id={tid} n={n_in}"
                            );
                            app.status_line = "thread/read · thread missing in sidebar".into();
                        }
                    }
                    Err(e) => {
                        app.status_line = format!("thread/read failed · {e}").into();
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn sync_search_from_input(&mut self, cx: &Context<Self>) {
        let value = self.search_input.read(cx).value().to_string();
        if value != self.search_query {
            self.search_query = value;
        }
    }
}

/// True when `id` is expected to exist on codex app-server (not a local/demo placeholder).
fn is_app_server_thread_id(id: &str) -> bool {
    !(id.starts_with("local-")
        || id.starts_with("fork-")
        || id.starts_with("demo-")
        || id.starts_with("chat-")
        || id.starts_with("goal-")
        || id.starts_with("fixture-"))
}

fn thread_summary_from_session(session: SessionSummary) -> ThreadSummary {
    ThreadSummary {
        id: session.id.raw,
        name: session.title,
        preview: session.preview,
        cwd: session.working_dir,
        created_at: None,
        updated_at: session.updated_at,
        model_provider: session.model_provider,
        ephemeral: Some(session.ephemeral),
        is_pinned: Some(false),
        archived: Some(session.archived),
        raw: None,
    }
}

fn model_info_from_product(model: ProductModel) -> ModelInfo {
    let default_service_tier = match &model.default_speed_mode {
        ProductSpeedMode::CodexServiceTier(tier) => Some(tier.clone()),
        ProductSpeedMode::CodexStandard
        | ProductSpeedMode::MitsuroStandard
        | ProductSpeedMode::MitsuroFast => None,
    };
    ModelInfo {
        id: model.id,
        model: model.model,
        display_name: model.display_name,
        description: model.description,
        hidden: model.hidden,
        is_default: model.is_default,
        default_reasoning_effort: model.default_reasoning_effort,
        supported_reasoning_efforts: model
            .supported_reasoning_efforts
            .into_iter()
            .map(|effort| ReasoningEffortOption {
                reasoning_effort: effort.effort,
                description: effort.description,
            })
            .collect(),
        service_tiers: model
            .speed_options
            .into_iter()
            .filter_map(|option| {
                let id = match option.mode {
                    ProductSpeedMode::CodexServiceTier(tier) => tier,
                    ProductSpeedMode::MitsuroFast => "priority".to_owned(),
                    ProductSpeedMode::CodexStandard | ProductSpeedMode::MitsuroStandard => {
                        return None;
                    }
                };
                Some(ModelServiceTier {
                    id,
                    name: option.label,
                    description: option.description,
                })
            })
            .collect(),
        default_service_tier,
        input_modalities: model.input_modalities,
        upgrade: model.upgrade,
    }
}

pub(crate) fn reasoning_effort_display_name(effort: &str) -> String {
    match effort.trim().to_ascii_lowercase().as_str() {
        "none" | "off" => "Off".to_owned(),
        "xhigh" | "x-high" => "XHigh".to_owned(),
        "ultra" => "Ultra".to_owned(),
        "" => "Default".to_owned(),
        value => {
            let mut chars = value.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_else(|| "Default".to_owned())
        }
    }
}

fn model_matches_query(model: &ModelInfo, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    query.is_empty()
        || model.label().to_ascii_lowercase().contains(&query)
        || model.id.to_ascii_lowercase().contains(&query)
        || model.model.to_ascii_lowercase().contains(&query)
        || model.description.to_ascii_lowercase().contains(&query)
}

fn valid_exec_server_url(value: &str) -> bool {
    let target = value
        .strip_prefix("ws://")
        .or_else(|| value.strip_prefix("wss://"));
    target.is_some_and(|target| !target.is_empty() && !target.chars().any(char::is_whitespace))
}

fn file_match_from_product(file: ProductFileMatch) -> FuzzyFileSearchResult {
    FuzzyFileSearchResult {
        root: file.root,
        path: file.path,
        match_type: if file.is_directory {
            mitsuro_desktop_backend::FuzzyFileSearchMatchType::Directory
        } else {
            mitsuro_desktop_backend::FuzzyFileSearchMatchType::File
        },
        file_name: file.file_name,
        score: file.score,
        indices: (!file.indices.is_empty()).then_some(file.indices),
    }
}

fn skill_metadata_from_product(skill: ProductSkill) -> SkillMetadata {
    SkillMetadata {
        name: skill.name,
        description: skill.description,
        enabled: skill.enabled,
        path: skill.path,
        scope: skill.scope,
        short_description: skill.short_description,
    }
}

fn mcp_status_from_product(server: ProductMcpServer) -> McpServerStatus {
    let auth_status = if server.status.contains("auth required") {
        McpAuthStatus::NotLoggedIn
    } else {
        McpAuthStatus::Unsupported
    };
    McpServerStatus {
        name: server.name.clone(),
        server_info: Some(McpServerInfo {
            name: server.name,
            version: String::new(),
            title: server.title,
            description: None,
            website_url: None,
        }),
        tools: server
            .tool_names
            .into_iter()
            .map(|name| (name, serde_json::Value::Null))
            .collect(),
        resources: Vec::new(),
        resource_templates: Vec::new(),
        auth_status,
    }
}

fn plugin_summary_from_product(extension: ProductExtension) -> PluginSummary {
    let mut extra = serde_json::Map::new();
    if let Some(path) = extension.marketplace_path {
        extra.insert("marketplacePath".to_owned(), path.into());
    }
    if let Some(name) = extension.remote_marketplace_name {
        extra.insert("remoteMarketplaceName".to_owned(), name.into());
    }
    PluginSummary {
        id: extension.id,
        name: extension.name,
        source: PluginSource::Remote,
        installed: extension.installed,
        enabled: extension.enabled,
        install_policy: extension.install_policy,
        auth_policy: extension.auth_policy,
        availability: extension.availability,
        version: extension.version,
        local_version: None,
        remote_plugin_id: None,
        interface: Some(PluginInterface {
            display_name: Some(extension.display_name),
            short_description: extension.description,
            long_description: None,
            developer_name: Some(extension.source),
            category: extension.category,
            capabilities: extension.capabilities,
        }),
        keywords: Vec::new(),
        extra,
    }
}

fn process_catalog_text(processes: &[ProductProcess]) -> String {
    if processes.is_empty() {
        return "Mitsuro background-process catalog is empty.\nInteractive terminal spawning is not exposed by this backend."
            .to_owned();
    }
    let mut output = String::from(
        "Mitsuro background processes (read-only)\nInteractive terminal spawning is not exposed by this backend.\n\n",
    );
    for process in processes {
        output.push_str(&format!(
            "{}  pid={}  {}  {}\n    {}\n",
            process.status,
            process
                .pid
                .map_or_else(|| "—".to_owned(), |value| value.to_string()),
            process.id,
            process.command,
            process.working_dir
        ));
    }
    output
}

fn hive_goals_from_snapshot(snapshot: &ProductHiveSnapshot) -> Vec<DemoGoal> {
    snapshot
        .runs
        .iter()
        .map(|run| {
            let status = match run.agent_state.as_str() {
                "paused" | "sleeping" | "scheduled" | "waiting" => DemoGoalStatus::Paused,
                "failed" | "blocked" => DemoGoalStatus::Blocked,
                "complete" | "completed" | "succeeded" => DemoGoalStatus::Complete,
                _ => DemoGoalStatus::Active,
            };
            let mut plan_items = Vec::new();
            for (label, count, done) in [
                ("Completed tasks", run.completed_tasks, true),
                ("In-progress tasks", run.in_progress_tasks, false),
                ("Pending tasks", run.pending_tasks, false),
                ("Blocked tasks", run.blocked_tasks, false),
                ("Failed tasks", run.failed_tasks, false),
            ] {
                if count > 0 {
                    plan_items.push(DemoPlanItem {
                        id: format!(
                            "{}-{}",
                            run.session_id,
                            label.to_lowercase().replace(' ', "-")
                        ),
                        title: format!("{label}: {count}"),
                        done,
                    });
                }
            }
            if plan_items.is_empty() {
                plan_items.push(DemoPlanItem {
                    id: format!("{}-idle", run.session_id),
                    title: "No queued tasks".to_owned(),
                    done: true,
                });
            }
            DemoGoal {
                id: run.session_id.clone(),
                objective: run
                    .diagnostic_summary
                    .clone()
                    .filter(|summary| !summary.is_empty())
                    .unwrap_or_else(|| run.title.clone()),
                status,
                plan_items,
                thread_id: Some(run.session_id.clone()),
                updated_at: None,
            }
        })
        .collect()
}

/// Normalize thread `cwd` fields (`file:///path` or absolute path) for fs/* roots.
fn path_from_cwd_field(cwd: &str) -> String {
    let s = cwd.trim();
    if let Some(rest) = s.strip_prefix("file://") {
        if rest.starts_with('/') {
            return rest.to_string();
        }
        // file://host/path → treat path portion as absolute when possible
        if let Some(idx) = rest.find('/') {
            return rest[idx..].to_string();
        }
        return rest.to_string();
    }
    s.to_string()
}

fn activity_title(kind: &str) -> String {
    activity_item_fields(&serde_json::json!({ "type": kind })).title
}

fn activity_message(
    kind: &mitsuro_desktop_backend::ItemKind,
    item_id: String,
    item: Option<&serde_json::Value>,
) -> DemoMessage {
    let fields = item
        .map(activity_item_fields)
        .unwrap_or_else(|| ActivityFields {
            kind: kind.as_str().to_owned(),
            title: activity_title(kind.as_str()),
            summary: String::new(),
            status: String::new(),
        });
    DemoMessage::activity(
        fields.kind,
        fields.title,
        fields.summary,
        fields.status,
        Some(item_id),
    )
}

fn find_message_mut<'a>(
    messages: &'a mut [DemoMessage],
    item_id: &str,
) -> Option<&'a mut DemoMessage> {
    messages
        .iter_mut()
        .rev()
        .find(|m| m.item_id.as_deref() == Some(item_id))
}

fn delegation_hydration_status(projection: &SessionDelegationProjection) -> Option<String> {
    if projection.groups.is_empty() {
        return None;
    }
    let (active_groups, active_tasks) = projection.active_counts();
    let latest = projection
        .latest_task()
        .map(|task| format!("latest {} {}", task.key, task.status.label()))
        .or_else(|| {
            projection
                .groups
                .iter()
                .max_by(|left, right| left.updated_at.cmp(&right.updated_at))
                .map(|group| format!("latest group {}", group.status.label()))
        });
    Some(match latest {
        Some(latest) => {
            format!("{active_groups} active groups · {active_tasks} active tasks · {latest}")
        }
        None => format!("{active_groups} active groups · {active_tasks} active tasks"),
    })
}

/// Minimal whitespace split for `echo hello` style argv (no shell quoting).
fn shell_split_simple(cmd: &str) -> Vec<String> {
    cmd.split_whitespace().map(str::to_string).collect()
}

fn rebind_thread_id(event: TurnStreamEvent, thread_id: &str) -> TurnStreamEvent {
    match event {
        TurnStreamEvent::TurnStarted { turn_id, turn, .. } => TurnStreamEvent::TurnStarted {
            thread_id: thread_id.into(),
            turn_id,
            turn,
        },
        TurnStreamEvent::TurnCompleted {
            turn_id,
            status,
            turn,
            ..
        } => TurnStreamEvent::TurnCompleted {
            thread_id: thread_id.into(),
            turn_id,
            status,
            turn,
        },
        TurnStreamEvent::ItemStarted {
            turn_id,
            item_id,
            kind,
            item,
            ..
        } => TurnStreamEvent::ItemStarted {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            kind,
            item,
        },
        TurnStreamEvent::ItemCompleted {
            turn_id,
            item_id,
            kind,
            text,
            item,
            ..
        } => TurnStreamEvent::ItemCompleted {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            kind,
            text,
            item,
        },
        TurnStreamEvent::AgentMessageDelta {
            turn_id,
            item_id,
            delta,
            ..
        } => TurnStreamEvent::AgentMessageDelta {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            delta,
        },
        TurnStreamEvent::ReasoningTextDelta {
            turn_id,
            item_id,
            content_index,
            delta,
            ..
        } => TurnStreamEvent::ReasoningTextDelta {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            content_index,
            delta,
        },
        TurnStreamEvent::ReasoningSummaryDelta {
            turn_id,
            item_id,
            summary_index,
            delta,
            ..
        } => TurnStreamEvent::ReasoningSummaryDelta {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            summary_index,
            delta,
        },
        TurnStreamEvent::PlanDelta {
            turn_id,
            item_id,
            delta,
            ..
        } => TurnStreamEvent::PlanDelta {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            delta,
        },
        TurnStreamEvent::CommandExecutionOutputDelta {
            turn_id,
            item_id,
            delta,
            ..
        } => TurnStreamEvent::CommandExecutionOutputDelta {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            delta,
        },
        TurnStreamEvent::FileChangeOutputDelta {
            turn_id,
            item_id,
            delta,
            ..
        } => TurnStreamEvent::FileChangeOutputDelta {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            delta,
        },
        TurnStreamEvent::FileChangePatchUpdated {
            turn_id,
            item_id,
            changes,
            ..
        } => TurnStreamEvent::FileChangePatchUpdated {
            thread_id: thread_id.into(),
            turn_id,
            item_id,
            changes,
        },
        TurnStreamEvent::ApprovalRequested(mut pending) => {
            pending.thread_id = Some(thread_id.into());
            TurnStreamEvent::ApprovalRequested(pending)
        }
        TurnStreamEvent::DelegatedProgress { progress, .. } => TurnStreamEvent::DelegatedProgress {
            thread_id: thread_id.into(),
            progress,
        },
        TurnStreamEvent::DelegationEvent { event, .. } => TurnStreamEvent::DelegationEvent {
            thread_id: thread_id.into(),
            event,
        },
        other => other,
    }
}

fn turn_update_is_current(
    active_generation: u64,
    active_thread_id: Option<&str>,
    candidate_generation: u64,
    candidate_thread_id: &str,
) -> bool {
    active_generation == candidate_generation && active_thread_id == Some(candidate_thread_id)
}

fn command_available(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|directory| directory.join(name).is_file())
    })
}

fn stream_pipewire_microphone(
    backend: Arc<DesktopBackend>,
    session_id: BackendSessionId,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    const SAMPLE_RATE: u32 = 24_000;
    const CHANNELS: u16 = 1;
    // 100 ms of signed 16-bit mono PCM.
    const CHUNK_BYTES: usize = 4_800;

    let mut child = Command::new("pw-record")
        .args([
            "--raw",
            "--format",
            "s16",
            "--rate",
            "24000",
            "--channels",
            "1",
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("could not launch pw-record: {error}"))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "pw-record did not expose an audio stream".to_owned())?;
    let mut buffer = vec![0_u8; CHUNK_BYTES];
    let result = loop {
        if stop.load(Ordering::SeqCst) {
            break Ok(());
        }
        let count = match stdout.read_exact(&mut buffer) {
            Ok(()) => CHUNK_BYTES,
            Err(error) => break Err(format!("could not read microphone audio: {error}")),
        };
        if stop.load(Ordering::SeqCst) {
            break Ok(());
        }
        let params = ThreadRealtimeAppendAudioParams {
            thread_id: session_id.raw.clone(),
            audio: ThreadRealtimeAudioChunk {
                data: base64::engine::general_purpose::STANDARD.encode(&buffer[..count]),
                num_channels: CHANNELS,
                sample_rate: SAMPLE_RATE,
                samples_per_channel: Some((count / 2) as u32),
                item_id: None,
            },
        };
        let runtime = Arc::clone(&backend);
        let runner = Arc::clone(&backend);
        let request_session = session_id.clone();
        if let Err(error) = runtime
            .block_on(async move { runner.realtime_append_audio(&request_session, params).await })
        {
            break Err(format!("realtime audio append failed: {error}"));
        }
    };
    let _ = child.kill();
    let _ = child.wait();
    result
}

fn start_pipewire_playback(
    sample_rate: u32,
    channels: u16,
) -> Result<mpsc::Sender<Vec<u8>>, String> {
    if !command_available("pw-play") {
        return Err("Voice playback requires PipeWire's pw-play command".to_owned());
    }
    let (audio_tx, audio_rx) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name("mitsuro-realtime-playback".to_owned())
        .spawn(move || {
            let mut child = match Command::new("pw-play")
                .args([
                    "--raw",
                    "--format",
                    "s16",
                    "--rate",
                    &sample_rate.to_string(),
                    "--channels",
                    &channels.to_string(),
                    "-",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => child,
                Err(error) => {
                    eprintln!("[mitsuro] could not launch pw-play: {error}");
                    return;
                }
            };
            if let Some(mut stdin) = child.stdin.take() {
                while let Ok(audio) = audio_rx.recv() {
                    if stdin.write_all(&audio).is_err() {
                        break;
                    }
                }
            }
            let _ = child.kill();
            let _ = child.wait();
        })
        .map_err(|error| format!("could not start voice playback worker: {error}"))?;
    Ok(audio_tx)
}

/// Replay fixture events with delay; pause when an approval is requested.
/// Honors `cancel` so Stop / `turn/interrupt` ends the stream early.
async fn replay_fixture_events(
    this: gpui::WeakEntity<MitsuroApp>,
    cx: &mut gpui::AsyncApp,
    thread_id: String,
    turn_generation: u64,
    events: Vec<TurnStreamEvent>,
    delay: Duration,
    cancel: Arc<AtomicBool>,
) {
    let mut iter = events.into_iter();
    while let Some(ev) = iter.next() {
        if cancel.load(Ordering::SeqCst) {
            let _ = this.update(cx, |app, cx| {
                if !app.is_current_turn(turn_generation, &thread_id) {
                    return;
                }
                if let Some(thread) = app.threads.iter_mut().find(|t| t.summary.id == thread_id) {
                    for m in &mut thread.messages {
                        m.streaming = false;
                    }
                }
                app.turn_in_progress = false;
                app.fixture_resume = None;
                app.active_turn_thread_id = None;
                app.active_turn_id = None;
                app.turn_cancel = None;
                if app.status_line.as_ref() != "Turn interrupted." {
                    app.status_line = "Fixture turn stopped.".into();
                }
                cx.notify();
            });
            return;
        }
        let is_approval = matches!(ev, TurnStreamEvent::ApprovalRequested(_));
        let done = matches!(ev, TurnStreamEvent::TurnCompleted { .. });
        let _ = this.update(cx, |app, cx| {
            if !app.is_current_turn(turn_generation, &thread_id) {
                return;
            }
            app.apply_stream_event(&thread_id, ev);
            cx.notify();
        });
        if is_approval {
            // Stash remaining events; user must Approve/Reject to continue.
            let rest: Vec<TurnStreamEvent> = iter.collect();
            let _ = this.update(cx, |app, cx| {
                if !app.is_current_turn(turn_generation, &thread_id) {
                    return;
                }
                app.fixture_resume = Some((thread_id.clone(), rest));
                // Keep turn_in_progress true while waiting so Send is blocked.
                app.turn_in_progress = true;
                app.status_line = "Waiting for approval…".into();
                cx.notify();
            });
            return;
        }
        if done {
            break;
        }
        let d = delay;
        let _ = cx
            .background_spawn(async move {
                std::thread::sleep(d);
            })
            .await;
    }
    let _ = this.update(cx, |app, cx| {
        if !app.is_current_turn(turn_generation, &thread_id) {
            return;
        }
        app.turn_in_progress = false;
        app.fixture_resume = None;
        app.active_turn_thread_id = None;
        app.active_turn_id = None;
        app.turn_cancel = None;
        if app.pending_approval.is_none() && !cancel.load(Ordering::SeqCst) {
            app.status_line = "Fixture turn complete.".into();
        }
        cx.notify();
    });
}

fn demo_image_attachments(images: Vec<ConversationImage>) -> Vec<DemoImageAttachment> {
    const MAX_EMBEDDED_BASE64_CHARS: usize = 28 * 1024 * 1024;
    const MAX_DECODED_IMAGE_BYTES: usize = 20 * 1024 * 1024;

    images
        .into_iter()
        .map(|image| match image {
            ConversationImage::LocalPath(path) => {
                let label = Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Attached image")
                    .to_owned();
                DemoImageAttachment {
                    label,
                    source: DemoImageSource::LocalPath(path),
                }
            }
            ConversationImage::Url(url) => {
                let label = url
                    .split(['/', '?'])
                    .rfind(|part| !part.is_empty())
                    .unwrap_or("Attached image")
                    .to_owned();
                DemoImageAttachment {
                    label,
                    source: DemoImageSource::Url(url),
                }
            }
            ConversationImage::Embedded { media_type, data } => {
                let format = ImageFormat::from_mime_type(&media_type);
                let decoded = if data.len() <= MAX_EMBEDDED_BASE64_CHARS {
                    base64::engine::general_purpose::STANDARD.decode(data).ok()
                } else {
                    None
                };
                let source = match (format, decoded) {
                    (Some(format), Some(bytes)) if bytes.len() <= MAX_DECODED_IMAGE_BYTES => {
                        DemoImageSource::Decoded(Arc::new(gpui::Image::from_bytes(format, bytes)))
                    }
                    _ => DemoImageSource::Unavailable(
                        "Embedded image could not be decoded safely".to_owned(),
                    ),
                };
                DemoImageAttachment {
                    label: format!("Attached {media_type}"),
                    source,
                }
            }
        })
        .collect()
}

fn demo_audio_attachments(audio: Vec<ConversationAudio>) -> Vec<DemoAudioAttachment> {
    const MAX_EMBEDDED_BASE64_CHARS: usize = 28 * 1024 * 1024;
    const MAX_DECODED_AUDIO_BYTES: usize = 20 * 1024 * 1024;

    audio
        .into_iter()
        .map(|audio| match audio {
            ConversationAudio::LocalPath(path) => {
                let label = Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Attached audio")
                    .to_owned();
                DemoAudioAttachment {
                    label,
                    source: DemoAudioSource::LocalPath(path),
                }
            }
            ConversationAudio::Url(url) => {
                let label = url
                    .split(['/', '?'])
                    .rfind(|part| !part.is_empty())
                    .unwrap_or("Attached audio")
                    .to_owned();
                DemoAudioAttachment {
                    label,
                    source: DemoAudioSource::Url(url),
                }
            }
            ConversationAudio::Embedded { media_type, data } => {
                let decoded = if data.len() <= MAX_EMBEDDED_BASE64_CHARS {
                    base64::engine::general_purpose::STANDARD.decode(data).ok()
                } else {
                    None
                };
                let source = match decoded {
                    Some(bytes) if bytes.len() <= MAX_DECODED_AUDIO_BYTES => {
                        DemoAudioSource::Embedded {
                            media_type: media_type.clone(),
                            byte_len: bytes.len(),
                        }
                    }
                    _ => DemoAudioSource::Unavailable(
                        "Embedded audio could not be decoded safely".to_owned(),
                    ),
                };
                DemoAudioAttachment {
                    label: format!("Attached {media_type}"),
                    source,
                }
            }
        })
        .collect()
}

fn demo_reference_attachments(
    references: Vec<ConversationReference>,
) -> Vec<DemoReferenceAttachment> {
    references
        .into_iter()
        .map(|reference| DemoReferenceAttachment {
            kind: match reference.kind {
                ConversationReferenceKind::Skill => DemoReferenceKind::Skill,
                ConversationReferenceKind::Mention => DemoReferenceKind::Mention,
            },
            name: reference.name,
            path: reference.path,
        })
        .collect()
}

fn disconnect_backend_best_effort(
    backend: Option<Arc<DesktopBackend>>,
    cx: &mut Context<MitsuroApp>,
) {
    let Some(backend) = backend else {
        return;
    };
    cx.spawn(async move |_this, cx| {
        let _ = cx
            .background_spawn(async move {
                let runner = Arc::clone(&backend);
                backend.block_on(async move { runner.disconnect().await })
            })
            .await;
    })
    .detach();
}

fn delete_session_best_effort(
    backend: Arc<DesktopBackend>,
    session_id: BackendSessionId,
    cx: &mut Context<MitsuroApp>,
) {
    cx.spawn(async move |_this, cx| {
        let _ = cx
            .background_spawn(async move {
                let runner = Arc::clone(&backend);
                backend.block_on(async move { runner.delete_session(&session_id).await })
            })
            .await;
    })
    .detach();
}

struct BackendBootstrap {
    init: mitsuro_desktop_backend::InitializeResponse,
    sessions: Vec<SessionSummary>,
    has_auth: bool,
    models: Vec<ProductModel>,
    collaboration_modes: Vec<CollaborationModeMask>,
    realtime_voices: Result<Option<RealtimeVoicesList>, String>,
    config_snip: Option<String>,
    skills: Vec<SkillMetadata>,
    mcp: Vec<McpServerStatus>,
    plugins: Vec<PluginSummary>,
    processes: Option<Vec<ProductProcess>>,
    hive: Option<ProductHiveSnapshot>,
    schedules: Option<Vec<ProductSchedule>>,
}

fn connect_list_auth_and_models(backend: Arc<DesktopBackend>) -> Result<BackendBootstrap, String> {
    // MUST use the backend pump runtime so child I/O stays alive after return.
    let b = Arc::clone(&backend);
    backend.block_on(async move {
        let init = b.connect().await.map_err(|e| format!("initialize: {e}"))?;
        let sessions = b
            .list_sessions(40)
            .await
            .map_err(|e| format!("sessions: {e}"))?;
        let has_auth = b.has_usable_auth().await;
        // model/list is best-effort — missing method or error falls back to empty
        // (UI seeds fixture demo models).
        let models = b.list_product_models(100).await.unwrap_or_default();
        let collaboration_modes = b
            .collaboration_mode_list(CollaborationModeListParams::default())
            .await
            .map(|response| response.data)
            .unwrap_or_default();
        let realtime_voices = if b.capabilities().realtime_voice {
            b.realtime_list_voices()
                .await
                .map(|response| Some(response.voices))
                .map_err(|error| error.to_string())
        } else {
            Ok(None)
        };
        // config/read best-effort for Settings snippet.
        let config_snip = match b
            .config_read(ConfigReadParams {
                include_layers: Some(false),
                ..Default::default()
            })
            .await
        {
            Ok(resp) => Some(resp.settings_snippet()),
            Err(_) => None,
        };
        // skills/list best-effort.
        let skills = match b.list_product_skills().await {
            Ok(skills) => skills
                .into_iter()
                .map(skill_metadata_from_product)
                .collect(),
            Err(_) => Vec::new(),
        };
        // Product catalogs are best-effort for the Extensions panel.
        let mcp = match b.list_product_mcp_servers().await {
            Ok(servers) => servers.into_iter().map(mcp_status_from_product).collect(),
            Err(_) => Vec::new(),
        };
        let plugins = match b.list_product_extensions().await {
            Ok(extensions) => extensions
                .into_iter()
                .map(plugin_summary_from_product)
                .collect(),
            Err(_) => Vec::new(),
        };
        let processes = b.list_background_processes().await.ok();
        let hive = b.hive_snapshot().await.ok();
        let schedules = b.list_schedules().await.ok();
        Ok(BackendBootstrap {
            init,
            sessions,
            has_auth,
            models,
            collaboration_modes,
            realtime_voices,
            config_snip,
            skills,
            mcp,
            plugins,
            processes,
            hive,
            schedules,
        })
    })
}

impl Focusable for MitsuroApp {
    fn focus_handle(&self, _cx: &gpui::App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MitsuroApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_search_from_input(cx);
        if matches!(self.active_mode, ProductMode::Settings) {
            self.sync_settings_search(cx);
        }
        // Keep OS titlebar in sync with product mode (Chat / Work / Codex / …).
        window.set_window_title(&self.active_mode.window_title());
        let colors = theme::colors();
        // Bar home: always-on left sidebar for Chat/Codex (+ stubs/plugins).
        // Activity rail only for advanced modes outside bar home nav.
        let show_sidebar = self.active_mode.shows_thread_sidebar();
        let show_rail = self.active_mode.shows_activity_rail();

        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.bg_under)
            .text_color(colors.text)
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_1()
                    .min_h_0()
                    .when(show_rail, |this| {
                        this.child(components::activity_rail(self, cx))
                    })
                    .when(show_sidebar, |this| {
                        this.child(components::sidebar(self, &self.search_input, cx))
                    })
                    .child(components::main_column(self, &self.composer_input, cx)),
            )
            .when_some(
                gpui_component::Root::render_notification_layer(window, cx),
                |this, layer| this.child(layer),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> UiConnection {
        UiConnection::Ready {
            detail: "test".into(),
            has_auth: true,
        }
    }

    #[test]
    fn ready_product_backends_send_live_by_default() {
        assert_eq!(
            decide_send_mode(&ready(), Some(BackendKind::MitsuroHttp), true, false),
            SendMode::Live
        );
        assert_eq!(
            decide_send_mode(&ready(), Some(BackendKind::CodexStdio), true, false),
            SendMode::Live
        );
    }

    #[test]
    fn fixture_replay_must_be_explicit() {
        assert_eq!(
            decide_send_mode(
                &UiConnection::Fixture,
                Some(BackendKind::Fixture),
                false,
                false,
            ),
            SendMode::Fixture
        );
        assert_eq!(
            decide_send_mode(&ready(), Some(BackendKind::MitsuroHttp), true, true),
            SendMode::Fixture
        );
        assert_eq!(
            decide_send_mode(&ready(), Some(BackendKind::Fixture), true, false),
            SendMode::Unavailable
        );
    }

    #[test]
    fn fixture_records_are_forbidden_for_every_product_backend_state() {
        for kind in [
            BackendKind::MitsuroHttp,
            BackendKind::CodexStdio,
            BackendKind::CodexWebSocket,
        ] {
            assert!(!fixture_records_allowed(&ready(), Some(kind)));
            assert!(!fixture_records_allowed(
                &UiConnection::Connecting,
                Some(kind)
            ));
            assert!(!fixture_records_allowed(
                &UiConnection::Error {
                    message: "test".into(),
                },
                Some(kind)
            ));
        }
        assert!(!fixture_records_allowed(
            &UiConnection::Fixture,
            Some(BackendKind::MitsuroHttp)
        ));
        assert!(fixture_records_allowed(
            &UiConnection::Fixture,
            Some(BackendKind::Fixture)
        ));
    }

    #[test]
    fn disconnected_or_unauthenticated_backend_cannot_send() {
        assert_eq!(
            decide_send_mode(
                &UiConnection::Connecting,
                Some(BackendKind::MitsuroHttp),
                true,
                false,
            ),
            SendMode::Unavailable
        );
        assert_eq!(
            decide_send_mode(
                &UiConnection::Ready {
                    detail: "test".into(),
                    has_auth: false,
                },
                Some(BackendKind::CodexStdio),
                true,
                false,
            ),
            SendMode::Unavailable
        );
    }

    #[test]
    fn turn_updates_are_scoped_to_the_originating_thread_and_generation() {
        assert!(turn_update_is_current(7, Some("thread-a"), 7, "thread-a"));
        assert!(!turn_update_is_current(7, Some("thread-a"), 7, "thread-b"));
        assert!(!turn_update_is_current(8, Some("thread-a"), 7, "thread-a"));
        assert!(!turn_update_is_current(7, None, 7, "thread-a"));
    }

    #[test]
    fn account_login_completion_preserves_server_identity_and_failure() {
        let success = LifecycleNotification::from_known(
            "account/login/completed",
            Some(&serde_json::json!({
                "loginId": "login-7",
                "success": true,
                "error": null
            })),
        )
        .expect("known account lifecycle event");
        assert_eq!(
            account_login_completion(&success),
            Some(AccountLoginCompletion {
                success: true,
                login_id: Some("login-7".to_owned()),
                error: None,
            })
        );

        let failure = LifecycleNotification::from_known(
            "account/login/completed",
            Some(&serde_json::json!({
                "loginId": "login-8",
                "success": false,
                "error": "authorization declined"
            })),
        )
        .expect("known account lifecycle event");
        assert_eq!(
            account_login_completion(&failure),
            Some(AccountLoginCompletion {
                success: false,
                login_id: Some("login-8".to_owned()),
                error: Some("authorization declined".to_owned()),
            })
        );
    }

    #[test]
    fn conversation_images_become_renderable_transcript_attachments() {
        let images = demo_image_attachments(vec![
            ConversationImage::LocalPath("/tmp/reference.png".to_owned()),
            ConversationImage::Url("https://example.com/remote.webp".to_owned()),
            ConversationImage::Embedded {
                media_type: "image/png".to_owned(),
                data: base64::engine::general_purpose::STANDARD.encode(b"png bytes"),
            },
        ]);

        assert_eq!(images.len(), 3);
        assert_eq!(images[0].label, "reference.png");
        assert!(matches!(
            &images[0].source,
            DemoImageSource::LocalPath(path) if path == "/tmp/reference.png"
        ));
        assert_eq!(images[1].label, "remote.webp");
        assert!(matches!(
            &images[1].source,
            DemoImageSource::Url(url) if url == "https://example.com/remote.webp"
        ));
        assert!(matches!(&images[2].source, DemoImageSource::Decoded(_)));
    }

    #[test]
    fn unsafe_embedded_images_remain_visible_as_unavailable_attachments() {
        let images = demo_image_attachments(vec![ConversationImage::Embedded {
            media_type: "text/plain".to_owned(),
            data: base64::engine::general_purpose::STANDARD.encode(b"not an image"),
        }]);

        assert_eq!(images.len(), 1);
        assert!(matches!(
            &images[0].source,
            DemoImageSource::Unavailable(reason)
                if reason == "Embedded image could not be decoded safely"
        ));
    }

    #[test]
    fn conversation_audio_becomes_truthful_transcript_attachments() {
        let audio = demo_audio_attachments(vec![
            ConversationAudio::LocalPath("/tmp/reference.wav".to_owned()),
            ConversationAudio::Url("https://example.com/remote.mp3".to_owned()),
            ConversationAudio::Embedded {
                media_type: "audio/ogg".to_owned(),
                data: base64::engine::general_purpose::STANDARD.encode(b"ogg bytes"),
            },
        ]);

        assert_eq!(audio.len(), 3);
        assert_eq!(audio[0].label, "reference.wav");
        assert!(matches!(
            &audio[0].source,
            DemoAudioSource::LocalPath(path) if path == "/tmp/reference.wav"
        ));
        assert_eq!(audio[1].label, "remote.mp3");
        assert!(matches!(
            &audio[1].source,
            DemoAudioSource::Url(url) if url == "https://example.com/remote.mp3"
        ));
        assert!(matches!(
            &audio[2].source,
            DemoAudioSource::Embedded { media_type, byte_len }
                if media_type == "audio/ogg" && *byte_len == 9
        ));
    }

    #[test]
    fn unsafe_embedded_audio_remains_visible_as_an_unavailable_attachment() {
        let audio = demo_audio_attachments(vec![ConversationAudio::Embedded {
            media_type: "audio/wav".to_owned(),
            data: "not base64".to_owned(),
        }]);
        assert!(matches!(
            &audio[0].source,
            DemoAudioSource::Unavailable(reason)
                if reason == "Embedded audio could not be decoded safely"
        ));
    }

    #[test]
    fn reasoning_effort_labels_match_the_advertised_wire_values() {
        assert_eq!(reasoning_effort_display_name("none"), "Off");
        assert_eq!(reasoning_effort_display_name("xhigh"), "XHigh");
        assert_eq!(reasoning_effort_display_name("ultra"), "Ultra");
        assert_eq!(reasoning_effort_display_name("medium"), "Medium");
    }

    #[test]
    fn model_picker_searches_live_catalog_fields_case_insensitively() {
        let models = fixture_demo_models();
        assert!(model_matches_query(&models[0], "SOL ULTRA"));
        assert!(model_matches_query(&models[1], "o3-demo"));
        assert!(model_matches_query(&models[2], "codex-shaped"));
        assert!(!model_matches_query(&models[0], "definitely absent"));
        assert!(model_matches_query(&models[0], ""));
    }

    #[test]
    fn plugin_projection_preserves_exact_marketplace_and_policy() {
        let plugin = plugin_summary_from_product(ProductExtension {
            id: "documents@openai".to_owned(),
            name: "documents".to_owned(),
            display_name: "Documents".to_owned(),
            description: Some("Create documents".to_owned()),
            category: Some("productivity".to_owned()),
            installed: false,
            enabled: false,
            install_policy: mitsuro_desktop_backend::PluginInstallPolicy::Available,
            auth_policy: mitsuro_desktop_backend::PluginAuthPolicy::OnInstall,
            availability: mitsuro_desktop_backend::PluginAvailability::Available,
            version: Some("1.0.0".to_owned()),
            capabilities: vec!["documents".to_owned()],
            source: "remote".to_owned(),
            marketplace_path: None,
            remote_marketplace_name: Some("openai-curated-remote".to_owned()),
        });

        assert_eq!(
            plugin.extra.get("remoteMarketplaceName"),
            Some(&serde_json::json!("openai-curated-remote"))
        );
        assert_eq!(
            plugin.install_policy,
            mitsuro_desktop_backend::PluginInstallPolicy::Available
        );
        assert_eq!(
            plugin.auth_policy,
            mitsuro_desktop_backend::PluginAuthPolicy::OnInstall
        );
    }

    #[test]
    fn remote_environment_urls_fail_closed_before_io() {
        assert!(valid_exec_server_url("ws://127.0.0.1:4100"));
        assert!(valid_exec_server_url("wss://exec.example.com/socket"));
        assert!(!valid_exec_server_url("https://exec.example.com"));
        assert!(!valid_exec_server_url("wss://"));
        assert!(!valid_exec_server_url("wss://bad host"));
    }
}
