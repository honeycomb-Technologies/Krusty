//! UI components for Krusty TUI
//!
//! Reusable rendering components: toolbar, status bar, scrollbars, plan sidebar, plugin window, toasts, etc.

pub mod decision_prompt;
pub mod plan_sidebar;
pub mod plugin_window;
pub mod scrollbars;
pub mod status_bar;
pub mod toast;
pub mod toolbar;

pub use decision_prompt::{DecisionPrompt, PromptAnswer, PromptOption, PromptQuestion, PromptType};
pub use plan_sidebar::{render_plan_sidebar, PlanSidebarState, MIN_TERMINAL_WIDTH};
pub use plugin_window::{render_plugin_window, PluginWindowState};
pub use scrollbars::{render_input_scrollbar, render_messages_scrollbar};
pub use status_bar::render_status_bar;
pub use toast::{render_toasts, Toast, ToastQueue};
pub use toolbar::{render_toolbar, PlanInfo};
