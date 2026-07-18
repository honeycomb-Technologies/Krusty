//! Plugin System
//!
//! Provides a trait-based plugin architecture for hosting custom content
//! in the TUI's plugin window (widgets, games, video, etc.).
//!
//! Supports multiple render modes:
//! - Text: Standard ratatui widgets
//! - KittyGraphics: Pixel rendering via Kitty graphics protocol (60fps capable)

use std::{any::Any, path::PathBuf, sync::RwLock};

use crossterm::event::Event;
use once_cell::sync::Lazy;
use ratatui::{buffer::Buffer, layout::Rect};

use crate::tui::themes::Theme;

#[cfg(unix)]
pub mod gamepad;
pub mod js;
pub mod kitty_graphics;
#[cfg(unix)]
pub mod libretro;
pub mod managed;
pub mod native;
#[cfg(unix)]
pub mod retroarch;

#[cfg(unix)]
pub use gamepad::GamepadHandler;
pub use js::JsPluginHost;
pub use kitty_graphics::{KittyGraphics, PluginFrame};
pub use managed::ManagedPlugin;
pub use native::NativePluginHost;
#[cfg(unix)]
pub use retroarch::{RetroArchPlugin, GAME_BOY_COLOR_PLUGIN_ID, LEGACY_RETROARCH_PLUGIN_ID};

/// Runtime descriptor for installable managed plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPluginDescriptor {
    pub id: String,
    pub name: String,
    pub version: String,
    pub publisher: String,
    pub description: Option<String>,
    pub runtime: crate::plugins::PluginRuntime,
    pub install_path: PathBuf,
    pub entry_component_path: PathBuf,
    pub enabled: bool,
    pub render_mode: PluginRenderMode,
    process_granted: bool,
}

impl InstalledPluginDescriptor {
    /// Build a TUI runtime descriptor when the package contributes a renderable
    /// entry component. Bundle-only packages still remain visible in the plugin
    /// catalog; they simply have no TUI runtime to instantiate. Callers must
    /// supply a freshly resolved process grant for native and JS runtimes.
    pub(crate) fn from_installed(
        plugin: &crate::plugins::InstalledPlugin,
        process_granted: bool,
    ) -> Option<Self> {
        let entry_component_path = plugin.entry_component_path.clone()?;
        let render_mode = if plugin
            .render_capabilities
            .iter()
            .any(|cap| matches!(cap, crate::plugins::PluginRenderCapability::Frame))
        {
            PluginRenderMode::KittyGraphics
        } else {
            PluginRenderMode::Text
        };

        Some(Self {
            id: plugin.id.clone(),
            name: plugin.name.clone(),
            version: plugin.version.clone(),
            publisher: plugin.publisher.clone(),
            description: plugin.description.clone(),
            runtime: plugin.runtime,
            install_path: plugin.install_path.clone(),
            entry_component_path,
            enabled: plugin.enabled,
            render_mode,
            process_granted,
        })
    }

    fn can_instantiate(&self) -> bool {
        !self.runtime.requires_process_permission() || self.process_granted
    }
}

static INSTALLED_PLUGINS: Lazy<RwLock<Vec<InstalledPluginDescriptor>>> =
    Lazy::new(|| RwLock::new(Vec::new()));

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

/// List available plugin IDs without loading or evaluating plugin code.
/// Runtime instantiation happens only when the user selects one ID.
pub fn available_plugin_ids() -> Vec<String> {
    let mut ids = Vec::new();
    #[cfg(unix)]
    ids.push(GAME_BOY_COLOR_PLUGIN_ID.to_string());
    ids.extend(
        installed_plugins()
            .into_iter()
            .filter(|descriptor| descriptor.enabled && descriptor.can_instantiate())
            .map(|descriptor| descriptor.id),
    );
    ids
}

pub fn is_plugin_available(id: &str) -> bool {
    available_plugin_ids()
        .iter()
        .any(|candidate| candidate == id)
}

/// Get a plugin by ID
pub fn get_plugin_by_id(id: &str) -> Option<Box<dyn Plugin>> {
    #[cfg(unix)]
    if id == GAME_BOY_COLOR_PLUGIN_ID || id == LEGACY_RETROARCH_PLUGIN_ID {
        return Some(Box::new(RetroArchPlugin::new()));
    }

    installed_plugin_by_id(id).and_then(|descriptor| {
        if descriptor.enabled {
            plugin_from_descriptor(descriptor)
        } else {
            None
        }
    })
}

fn plugin_from_descriptor(descriptor: InstalledPluginDescriptor) -> Option<Box<dyn Plugin>> {
    if !descriptor.can_instantiate() {
        return None;
    }

    let plugin: Box<dyn Plugin> = match descriptor.runtime {
        crate::plugins::PluginRuntime::Native => Box::new(NativePluginHost::new(descriptor)),
        crate::plugins::PluginRuntime::Js => Box::new(JsPluginHost::new(descriptor)),
        crate::plugins::PluginRuntime::Wasm => Box::new(ManagedPlugin::new(descriptor)),
    };
    Some(plugin)
}

pub fn set_installed_plugins(mut descriptors: Vec<InstalledPluginDescriptor>) {
    descriptors.sort_by_key(|descriptor| descriptor.name.to_lowercase());
    if let Ok(mut guard) = INSTALLED_PLUGINS.write() {
        *guard = descriptors;
    }
}

pub fn installed_plugins() -> Vec<InstalledPluginDescriptor> {
    INSTALLED_PLUGINS
        .read()
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn installed_plugin_by_id(id: &str) -> Option<InstalledPluginDescriptor> {
    INSTALLED_PLUGINS
        .read()
        .ok()
        .and_then(|guard| guard.iter().find(|descriptor| descriptor.id == id).cloned())
}

pub fn installed_plugin_version_map() -> std::collections::HashMap<String, String> {
    installed_plugins()
        .into_iter()
        .map(|plugin| (plugin.id, plugin.version))
        .collect()
}

pub fn plugin_descriptor_by_id(id: &str) -> Option<InstalledPluginDescriptor> {
    installed_plugin_by_id(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(
        runtime: crate::plugins::PluginRuntime,
        process_granted: bool,
    ) -> InstalledPluginDescriptor {
        InstalledPluginDescriptor {
            id: format!("test-{runtime:?}"),
            name: "Test Plugin".to_string(),
            version: "1.0.0".to_string(),
            publisher: "test.publisher".to_string(),
            description: None,
            runtime,
            install_path: PathBuf::from("/nonexistent/plugin"),
            entry_component_path: PathBuf::from("/nonexistent/plugin/entry"),
            enabled: true,
            render_mode: PluginRenderMode::Text,
            process_granted,
        }
    }

    #[test]
    fn native_and_js_hosts_are_not_instantiated_without_process_grant() {
        for runtime in [
            crate::plugins::PluginRuntime::Native,
            crate::plugins::PluginRuntime::Js,
        ] {
            let descriptor = descriptor(runtime, false);
            assert!(!descriptor.can_instantiate());
            assert!(plugin_from_descriptor(descriptor).is_none());
        }
    }

    #[test]
    fn wasm_host_remains_available_without_process_grant() {
        let descriptor = descriptor(crate::plugins::PluginRuntime::Wasm, false);
        assert!(descriptor.can_instantiate());
        assert!(plugin_from_descriptor(descriptor).is_some());
    }
}
