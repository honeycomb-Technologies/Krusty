//! Input handling for Mitsuro TUI
//!
//! - Multi-line editor with cursor management
//! - Slash command autocomplete
//! - File search with @ trigger
//! - Image reference parsing

pub mod autocomplete;
pub mod file_search;
pub mod image_parser;
pub mod multi_line;

pub use autocomplete::AutocompletePopup;
pub use file_search::FileSearchPopup;
pub use image_parser::{has_image_references, parse_input, InputSegment};
pub use multi_line::{InputAction, MultiLineInput};
