//! App State Components
//!
//! Centralized state management for the TUI.
//! Groups related state into logical modules.

mod blocks;
mod chat;
mod hover;
mod indices;
mod layout;
mod popups;
mod scroll;
mod scroll_system;
mod selection;
mod ui_state;

pub use blocks::BlockManager;
pub use chat::ChatState;
pub use hover::{HoverState, HoveredLink};
pub use indices::BlockIndices;
pub use layout::LayoutState;
pub use popups::PopupState;
pub use scroll::{LayoutCache, ScrollState};
pub use scroll_system::ScrollSystem;
pub use selection::{
    BlockScrollbarDrag, DragTarget, EdgeScrollDirection, EdgeScrollState, ScrollbarDrag,
    SelectionArea, SelectionState,
};
pub use ui_state::{hash_content, BlockUiStates, ToolResultCache};
