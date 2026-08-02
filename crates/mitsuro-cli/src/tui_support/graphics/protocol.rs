//! Terminal graphics protocol detection
//!
//! Provides picker for image rendering in file previews.

use ratatui_image::picker::Picker;

/// Graphics context with cached picker for image rendering
pub struct GraphicsContext {
    pub picker: Option<Picker>,
}

impl GraphicsContext {
    /// Detect and create graphics context
    pub fn detect() -> Self {
        match Picker::from_query_stdio() {
            Ok(picker) => Self {
                picker: Some(picker),
            },
            Err(_) => Self { picker: None },
        }
    }
}
