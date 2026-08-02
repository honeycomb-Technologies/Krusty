//! Input routing through the central action registry.

pub mod action;
pub mod composer_buffer;
pub mod file_search;
pub mod mouse;
pub mod slash;

use crate::tui_v2::{app::state::UiState, model::focus::FocusTarget};

use action::ActionContext;

pub fn active_context(state: &UiState) -> ActionContext {
    if state.overlay.is_some() {
        return ActionContext::Overlay;
    }

    match state.focus {
        FocusTarget::Composer => ActionContext::Composer,
        FocusTarget::Transcript { .. } => ActionContext::Transcript,
        FocusTarget::Artifact { .. } => ActionContext::Artifact,
        FocusTarget::DecisionDock => ActionContext::DecisionDock,
        FocusTarget::PlanDock => ActionContext::Transcript,
        FocusTarget::PluginDock => ActionContext::Transcript,
        FocusTarget::Overlay { .. } => ActionContext::Overlay,
    }
}
