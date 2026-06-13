//! Native dynamic-library plugin host.
//!
//! Native plugins are intentionally unsafe: loading a plugin is equivalent to
//! executing arbitrary local code. Krusty keeps the ABI small and C-compatible so
//! Rust, C, and C++ packages can implement it without relying on Rust trait ABI.

use std::{
    any::Any,
    ffi::{c_char, c_void, CStr},
    fs,
    path::{Path, PathBuf},
};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use libloading::Library;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Paragraph, Widget},
};
use uuid::Uuid;

use super::{
    kitty_graphics::PluginFrame, InstalledPluginDescriptor, Plugin, PluginContext,
    PluginEventResult, PluginRenderMode,
};

pub const KRUSTY_NATIVE_PLUGIN_ABI_VERSION: u32 = 1;
pub const KRUSTY_NATIVE_EVENT_KEY: u32 = 1;
pub const KRUSTY_NATIVE_EVENT_ENTER: u32 = 0x1_0000;
pub const KRUSTY_NATIVE_EVENT_ESC: u32 = 0x1_0001;
pub const KRUSTY_NATIVE_EVENT_BACKSPACE: u32 = 0x1_0002;
pub const KRUSTY_NATIVE_EVENT_UP: u32 = 0x1_0003;
pub const KRUSTY_NATIVE_EVENT_DOWN: u32 = 0x1_0004;
pub const KRUSTY_NATIVE_EVENT_LEFT: u32 = 0x1_0005;
pub const KRUSTY_NATIVE_EVENT_RIGHT: u32 = 0x1_0006;

pub const KRUSTY_NATIVE_EVENT_RESULT_CONSUMED: u32 = 1;

type LineSink = unsafe extern "C" fn(userdata: *mut c_void, text: *const c_char);

type CreateFn = unsafe extern "C" fn() -> *mut c_void;
type DestroyFn = unsafe extern "C" fn(instance: *mut c_void);
type LifecycleFn = unsafe extern "C" fn(instance: *mut c_void);
type TickFn = unsafe extern "C" fn(instance: *mut c_void) -> bool;
type RenderTextFn = unsafe extern "C" fn(
    instance: *mut c_void,
    width: u16,
    height: u16,
    sink: LineSink,
    userdata: *mut c_void,
);
type HandleEventFn = unsafe extern "C" fn(instance: *mut c_void, event: KrustyNativeEvent) -> u32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct KrustyNativePluginV1 {
    pub abi_version: u32,
    pub create: Option<CreateFn>,
    pub destroy: Option<DestroyFn>,
    pub on_activate: Option<LifecycleFn>,
    pub on_deactivate: Option<LifecycleFn>,
    pub tick: Option<TickFn>,
    pub render_text: Option<RenderTextFn>,
    pub handle_event: Option<HandleEventFn>,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct KrustyNativeEvent {
    pub kind: u32,
    pub key_code: u32,
    pub modifiers: u32,
}

pub struct NativePluginHost {
    descriptor: InstalledPluginDescriptor,
    inner: Result<NativePluginInstance, String>,
}

struct NativePluginInstance {
    _library: Library,
    _shadow_path: PathBuf,
    api: KrustyNativePluginV1,
    instance: *mut c_void,
}

// Native plugins are arbitrary code. The host only calls them from the TUI thread,
// but Plugin requires Send + Sync so plugin trait objects can live in shared state.
unsafe impl Send for NativePluginInstance {}
unsafe impl Sync for NativePluginInstance {}
unsafe impl Send for NativePluginHost {}
unsafe impl Sync for NativePluginHost {}

impl NativePluginHost {
    pub fn new(descriptor: InstalledPluginDescriptor) -> Self {
        let inner = NativePluginInstance::load(&descriptor).map_err(|err| err.to_string());
        Self { descriptor, inner }
    }

    fn render_error_lines(&self) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        lines.push(Line::from(format!(
            "{} v{}",
            self.descriptor.name, self.descriptor.version
        )));
        lines.push(Line::from("native plugin failed to load"));
        lines.push(Line::from(""));
        if let Err(error) = &self.inner {
            lines.push(Line::from(error.clone()));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(format!(
            "entry: {}",
            self.descriptor.entry_component_path.display()
        )));
        lines
    }
}

impl NativePluginInstance {
    fn load(descriptor: &InstalledPluginDescriptor) -> anyhow::Result<Self> {
        let shadow_path = shadow_copy_native_library(
            &descriptor.id,
            &descriptor.install_path,
            &descriptor.entry_component_path,
        )?;

        // SAFETY: Loading a native plugin is explicitly unsafe by policy. The path is
        // a shadow copy of the installed plugin entry component.
        let library = unsafe { Library::new(&shadow_path)? };
        let api = unsafe {
            let entry: libloading::Symbol<unsafe extern "C" fn() -> *const KrustyNativePluginV1> =
                library.get(b"krusty_plugin_entry")?;
            let api_ptr = entry();
            if api_ptr.is_null() {
                anyhow::bail!("krusty_plugin_entry returned null");
            }
            *api_ptr
        };

        if api.abi_version != KRUSTY_NATIVE_PLUGIN_ABI_VERSION {
            anyhow::bail!(
                "unsupported native plugin ABI {}; expected {}",
                api.abi_version,
                KRUSTY_NATIVE_PLUGIN_ABI_VERSION
            );
        }

        let instance = match api.create {
            Some(create) => unsafe { create() },
            None => std::ptr::null_mut(),
        };

        Ok(Self {
            _library: library,
            _shadow_path: shadow_path,
            api,
            instance,
        })
    }

    fn render_text_lines(&self, width: u16, height: u16) -> Vec<String> {
        let Some(render_text) = self.api.render_text else {
            return vec!["Native plugin does not provide render_text.".to_string()];
        };

        let mut lines = Vec::<String>::new();
        // SAFETY: The callback only runs synchronously during render_text and userdata
        // points to the stack-owned Vec<String> for the duration of the call.
        unsafe {
            render_text(
                self.instance,
                width,
                height,
                collect_line_sink,
                (&mut lines as *mut Vec<String>).cast::<c_void>(),
            );
        }
        lines
    }

    fn handle_event(&mut self, event: &Event) -> PluginEventResult {
        let Some(handle_event) = self.api.handle_event else {
            return PluginEventResult::Ignored;
        };
        let Some(native_event) = map_event(event) else {
            return PluginEventResult::Ignored;
        };

        let result = unsafe { handle_event(self.instance, native_event) };
        if result == KRUSTY_NATIVE_EVENT_RESULT_CONSUMED {
            PluginEventResult::Consumed
        } else {
            PluginEventResult::Ignored
        }
    }

    fn tick(&mut self) -> bool {
        self.api
            .tick
            .map(|tick| unsafe { tick(self.instance) })
            .unwrap_or(false)
    }

    fn on_activate(&mut self) {
        if let Some(on_activate) = self.api.on_activate {
            unsafe { on_activate(self.instance) };
        }
    }

    fn on_deactivate(&mut self) {
        if let Some(on_deactivate) = self.api.on_deactivate {
            unsafe { on_deactivate(self.instance) };
        }
    }
}

impl Drop for NativePluginInstance {
    fn drop(&mut self) {
        self.on_deactivate();
        if let Some(destroy) = self.api.destroy {
            unsafe { destroy(self.instance) };
        }
    }
}

unsafe extern "C" fn collect_line_sink(userdata: *mut c_void, text: *const c_char) {
    if userdata.is_null() || text.is_null() {
        return;
    }

    // SAFETY: userdata is supplied by render_text_lines as a valid Vec<String>
    // pointer, and text is expected to be a null-terminated string for this call.
    let lines = unsafe { &mut *(userdata.cast::<Vec<String>>()) };
    let line = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .to_string();
    lines.push(line);
}

fn shadow_copy_native_library(
    id: &str,
    install_path: &Path,
    source: &Path,
) -> anyhow::Result<PathBuf> {
    if !source.exists() {
        anyhow::bail!("native plugin entry does not exist: {}", source.display());
    }

    let shadow_dir = install_path.join(".krusty-shadow");
    fs::create_dir_all(&shadow_dir)?;

    let ext = source
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("so");
    let sanitized_id = id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>();
    let shadow_path = shadow_dir.join(format!("{}-{}.{}", sanitized_id, Uuid::new_v4(), ext));
    fs::copy(source, &shadow_path)?;
    Ok(shadow_path)
}

fn map_event(event: &Event) -> Option<KrustyNativeEvent> {
    let Event::Key(KeyEvent {
        code, modifiers, ..
    }) = event
    else {
        return None;
    };

    let key_code = match code {
        KeyCode::Char(ch) => *ch as u32,
        KeyCode::Enter => KRUSTY_NATIVE_EVENT_ENTER,
        KeyCode::Esc => KRUSTY_NATIVE_EVENT_ESC,
        KeyCode::Backspace => KRUSTY_NATIVE_EVENT_BACKSPACE,
        KeyCode::Up => KRUSTY_NATIVE_EVENT_UP,
        KeyCode::Down => KRUSTY_NATIVE_EVENT_DOWN,
        KeyCode::Left => KRUSTY_NATIVE_EVENT_LEFT,
        KeyCode::Right => KRUSTY_NATIVE_EVENT_RIGHT,
        _ => return None,
    };

    Some(KrustyNativeEvent {
        kind: KRUSTY_NATIVE_EVENT_KEY,
        key_code,
        modifiers: encode_modifiers(*modifiers),
    })
}

fn encode_modifiers(modifiers: KeyModifiers) -> u32 {
    let mut encoded = 0;
    if modifiers.contains(KeyModifiers::SHIFT) {
        encoded |= 1;
    }
    if modifiers.contains(KeyModifiers::CONTROL) {
        encoded |= 1 << 1;
    }
    if modifiers.contains(KeyModifiers::ALT) {
        encoded |= 1 << 2;
    }
    encoded
}

impl Plugin for NativePluginHost {
    fn id(&self) -> &str {
        &self.descriptor.id
    }

    fn name(&self) -> &str {
        &self.descriptor.name
    }

    fn display_name(&self) -> String {
        match &self.inner {
            Ok(_) => format!("{} ({})", self.descriptor.name, self.descriptor.version),
            Err(_) => format!("{} (load error)", self.descriptor.name),
        }
    }

    fn render_mode(&self) -> PluginRenderMode {
        // Native v1 exposes text rendering only. Frame plugins can be added by
        // extending the ABI with a render_frame callback.
        PluginRenderMode::Text
    }

    fn render(&self, area: Rect, buf: &mut Buffer, ctx: &PluginContext) {
        let lines = match &self.inner {
            Ok(instance) => instance
                .render_text_lines(area.width, area.height)
                .into_iter()
                .map(Line::from)
                .collect::<Vec<_>>(),
            Err(_) => self.render_error_lines(),
        };

        let paragraph = Paragraph::new(lines).style(
            Style::default()
                .fg(ctx.theme.text_color)
                .bg(ctx.theme.bg_color),
        );
        paragraph.render(area, buf);
    }

    fn render_frame(&mut self, _width: u32, _height: u32) -> Option<PluginFrame> {
        None
    }

    fn handle_event(&mut self, event: &Event, _area: Rect) -> PluginEventResult {
        match &mut self.inner {
            Ok(instance) => instance.handle_event(event),
            Err(_) => PluginEventResult::Ignored,
        }
    }

    fn tick(&mut self) -> bool {
        match &mut self.inner {
            Ok(instance) => instance.tick(),
            Err(_) => false,
        }
    }

    fn on_activate(&mut self) {
        if let Ok(instance) = &mut self.inner {
            instance.on_activate();
        }
    }

    fn on_deactivate(&mut self) {
        if let Ok(instance) = &mut self.inner {
            instance.on_deactivate();
        }
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_basic_key_events() {
        let event = Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL));
        let mapped = map_event(&event).expect("key event");
        assert_eq!(mapped.kind, KRUSTY_NATIVE_EVENT_KEY);
        assert_eq!(mapped.key_code, 'x' as u32);
        assert_eq!(mapped.modifiers, 1 << 1);
    }

    #[test]
    fn parses_special_keys() {
        let event = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()));
        let mapped = map_event(&event).expect("key event");
        assert_eq!(mapped.key_code, KRUSTY_NATIVE_EVENT_ESC);
    }
}
