//! Plugin System
//!
//! Provides a trait-based plugin architecture for hosting custom content
//! in the TUI's plugin window (widgets, games, video, etc.).
//!
//! Supports multiple render modes:
//! - Text: Standard ratatui widgets
//! - KittyGraphics: Pixel rendering via Kitty graphics protocol (60fps capable)

use std::any::Any;

use crossterm::event::Event;
use ratatui::{buffer::Buffer, layout::Rect};

use crate::tui::themes::Theme;

#[cfg(unix)]
pub mod gamepad;
pub mod kitty_graphics;
#[cfg(unix)]
pub mod libretro;
#[cfg(unix)]
pub mod retroarch;

#[cfg(unix)]
pub use gamepad::GamepadHandler;
pub use kitty_graphics::{KittyGraphics, PluginFrame};
#[cfg(unix)]
pub use retroarch::RetroArchPlugin;

/// No-op gamepad handler for non-Unix platforms
#[cfg(not(unix))]
pub struct GamepadHandler;

#[cfg(not(unix))]
impl GamepadHandler {
    pub fn new() -> Self {
        Self
    }

    pub fn poll(&mut self) -> bool {
        false
    }

    pub fn pressed_buttons(&self) -> std::iter::Empty<u8> {
        std::iter::empty()
    }
}

#[cfg(not(unix))]
impl Default for GamepadHandler {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of plugin event handling
#[derive(Debug, Clone, PartialEq)]
pub enum PluginEventResult {
    /// Event was consumed by the plugin (used by interactive plugins like games)
    Consumed,
    /// Event was not handled, pass to parent
    Ignored,
}

/// Rendering mode for plugins
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginRenderMode {
    /// Standard ratatui widgets (text, borders, etc.)
    #[default]
    Text,
    /// Kitty graphics protocol for pixel rendering (60fps @ 720p capable)
    KittyGraphics,
}

/// Context passed to plugins during rendering
#[derive(Debug, Clone)]
pub struct PluginContext<'a> {
    /// Current theme for styling
    pub theme: &'a Theme,
}

/// Plugin trait - implement this for custom plugin content
pub trait Plugin: Send + Sync {
    /// Unique identifier for this plugin
    fn id(&self) -> &str;

    /// Display name for the plugin
    fn name(&self) -> &str;

    /// Display name with optional status suffix (e.g., "System Monitor (paused)")
    /// Default implementation just returns name()
    fn display_name(&self) -> String {
        self.name().to_string()
    }

    /// Rendering mode - determines how content is displayed
    fn render_mode(&self) -> PluginRenderMode {
        PluginRenderMode::Text
    }

    /// Render content to buffer (for Text mode)
    fn render(&self, area: Rect, buf: &mut Buffer, ctx: &PluginContext);

    /// Render graphics frame (for KittyGraphics mode)
    /// Returns pixel data as RGBA, or None if nothing to render
    fn render_frame(&mut self, width: u32, height: u32) -> Option<PluginFrame> {
        let _ = (width, height);
        None
    }

    /// Handle input events - returns Consumed if handled, Ignored otherwise
    fn handle_event(&mut self, event: &Event, area: Rect) -> PluginEventResult;

    /// Animation tick (called at ~60fps when visible)
    /// Returns true if the plugin needs a redraw
    fn tick(&mut self) -> bool;

    /// Called when the plugin becomes active
    fn on_activate(&mut self) {}

    /// Called when the plugin becomes inactive
    fn on_deactivate(&mut self) {}

    /// Downcast to concrete type for plugin-specific operations
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// List of available built-in plugins
pub fn builtin_plugins() -> Vec<Box<dyn Plugin>> {
    let mut plugins: Vec<Box<dyn Plugin>> = vec![];
    #[cfg(unix)]
    plugins.insert(0, Box::new(RetroArchPlugin::new()));
    plugins
}

/// Get a plugin by ID
pub fn get_plugin_by_id(id: &str) -> Option<Box<dyn Plugin>> {
    match id {
        #[cfg(unix)]
        "retroarch" => Some(Box::new(RetroArchPlugin::new())),
        _ => None,
    }
}
