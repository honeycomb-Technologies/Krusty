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
use ratatui::{backend::CrosstermBackend, style::Color, Terminal};
use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;

use crate::agent::{AgentCancellation, AgentConfig, AgentEventBus, AgentState, UserHookManager};
use crate::ai::client::AiClient;
use crate::ai::format_detection::detect_api_format;
use crate::ai::model_profile::ModelProfile;
use crate::ai::models::resolve_context_window;
use crate::ai::models::SharedModelRegistry;
use crate::ai::providers::ProviderId;
use crate::ai::types::{AiTool, AiToolCall, Usage};
use crate::extensions::{WasmExtension, WasmHost};
use crate::plan::{PlanFile, PlanManager};
use crate::plugins::PluginManager;
use crate::process::ProcessRegistry;
use crate::storage::{CredentialStore, Preferences, SessionManager};
use crate::tools::registry::PermissionMode;
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
use crate::tui::utils::{AsyncChannels, TitleEditor};
use krusty_core::skills::SkillsManager;

mod behavior;
mod lifecycle;
mod state;
mod types;

pub use state::{App, AppRuntime, AppServices, AppUi};
pub use types::{Popup, ThinkingLevel, View, WorkMode};
