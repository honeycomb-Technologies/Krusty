use std::{
    any::Any,
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

use crossterm::event::Event;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::{
    InstalledPluginDescriptor, Plugin, PluginContext, PluginEventResult, PluginRenderMode,
};

const JS_RENDER_HELP: &str =
    "JS/TS plugins must call krusty.registerPlugin({ renderText() { return ['line']; } }).";

pub struct JsPluginHost {
    descriptor: InstalledPluginDescriptor,
    node: Option<edon::Nodejs>,
    status: String,
    render_lines: Vec<String>,
    render_file: PathBuf,
    last_tick: Instant,
}

impl JsPluginHost {
    pub fn new(descriptor: InstalledPluginDescriptor) -> Self {
        let render_file = std::env::temp_dir().join(format!(
            "krusty-js-plugin-{}-{}.json",
            sanitize_for_file(&descriptor.id),
            uuid::Uuid::new_v4()
        ));
        let mut host = Self {
            descriptor,
            node: None,
            status: String::new(),
            render_lines: Vec::new(),
            render_file,
            last_tick: Instant::now(),
        };
        host.load();
        host
    }

    fn load(&mut self) {
        match self.load_inner() {
            Ok(node) => {
                self.node = Some(node);
                self.status = "edon/libnode JS runtime loaded".to_string();
                self.refresh_render_lines();
            }
            Err(err) => {
                self.status = format!("JS runtime unavailable: {err}");
                self.render_lines = vec![JS_RENDER_HELP.to_string()];
            }
        }
    }

    fn load_inner(&self) -> anyhow::Result<edon::Nodejs> {
        let libnode_path = resolve_libnode_path().ok_or_else(|| {
            anyhow::anyhow!(
                "set KRUSTY_LIBNODE or EDON_LIBNODE_PATH to libnode.so/libnode.dylib/libnode.dll"
            )
        })?;
        let node = edon::Nodejs::load_with_args(&libnode_path, &["--no-warnings"])
            .map_err(|err| anyhow::anyhow!("failed to load {}: {err}", libnode_path.display()))?;

        let source = fs::read_to_string(&self.descriptor.entry_component_path).map_err(|err| {
            anyhow::anyhow!(
                "failed to read {}: {err}",
                self.descriptor.entry_component_path.display()
            )
        })?;
        let plugin_id = serde_json::to_string(&self.descriptor.id)?;
        let prelude = format!(
            r#"
globalThis.__krustyPluginRegistry = globalThis.__krustyPluginRegistry || Object.create(null);
globalThis.__krustyPluginLogs = globalThis.__krustyPluginLogs || Object.create(null);
globalThis.__krustyPluginLogs[{plugin_id}] = [];
globalThis.console = {{
  log: (...args) => globalThis.__krustyPluginLogs[{plugin_id}].push(args.map(String).join(' ')),
  warn: (...args) => globalThis.__krustyPluginLogs[{plugin_id}].push(args.map(String).join(' ')),
  error: (...args) => globalThis.__krustyPluginLogs[{plugin_id}].push(args.map(String).join(' ')),
}};
globalThis.krusty = globalThis.krusty || {{}};
globalThis.krusty.registerPlugin = function(plugin) {{
  globalThis.__krustyPluginRegistry[{plugin_id}] = plugin;
}};
"#
        );
        let postlude = format!(
            r#"
if (globalThis.krustyPlugin) {{
  globalThis.__krustyPluginRegistry[{plugin_id}] = globalThis.krustyPlugin;
}}
"#
        );
        let wrapped = format!("{prelude}\n{source}\n{postlude}");

        if is_typescript_entry(&self.descriptor.entry_component_path) {
            node.eval_typescript_blocking(&wrapped)
                .map_err(|err| anyhow::anyhow!("failed to evaluate TypeScript plugin: {err}"))?;
        } else {
            node.eval_blocking(&wrapped)
                .map_err(|err| anyhow::anyhow!("failed to evaluate JavaScript plugin: {err}"))?;
        }

        Ok(node)
    }

    fn refresh_render_lines(&mut self) {
        let Some(node) = self.node.as_ref() else {
            return;
        };

        let plugin_id = match serde_json::to_string(&self.descriptor.id) {
            Ok(value) => value,
            Err(err) => {
                self.status = format!("failed to encode plugin id: {err}");
                return;
            }
        };
        let output_path = match serde_json::to_string(&self.render_file.to_string_lossy()) {
            Ok(value) => value,
            Err(err) => {
                self.status = format!("failed to encode render path: {err}");
                return;
            }
        };
        let script = format!(
            r#"
const fs = require('fs');
const plugin = globalThis.__krustyPluginRegistry && globalThis.__krustyPluginRegistry[{plugin_id}];
let lines = [];
if (plugin && typeof plugin.renderText === 'function') {{
  const rendered = plugin.renderText();
  lines = Array.isArray(rendered) ? rendered : [rendered];
}} else if (plugin && typeof plugin.render === 'function') {{
  const rendered = plugin.render();
  lines = Array.isArray(rendered) ? rendered : [rendered];
}}
fs.writeFileSync({output_path}, JSON.stringify(lines.map((line) => String(line))));
"#
        );

        if let Err(err) = node.eval_blocking(script) {
            self.status = format!("render failed: {err}");
            return;
        }

        match fs::read_to_string(&self.render_file)
            .ok()
            .and_then(|content| serde_json::from_str::<Vec<String>>(&content).ok())
        {
            Some(lines) => self.render_lines = lines,
            None => self.status = "render did not produce JSON lines".to_string(),
        }
    }

    fn call_hook(&mut self, hook: &str) {
        let Some(node) = self.node.as_ref() else {
            return;
        };
        let plugin_id = match serde_json::to_string(&self.descriptor.id) {
            Ok(value) => value,
            Err(err) => {
                self.status = format!("failed to encode plugin id: {err}");
                return;
            }
        };
        let hook = match serde_json::to_string(hook) {
            Ok(value) => value,
            Err(err) => {
                self.status = format!("failed to encode hook: {err}");
                return;
            }
        };
        let script = format!(
            r#"
const plugin = globalThis.__krustyPluginRegistry && globalThis.__krustyPluginRegistry[{plugin_id}];
const hook = {hook};
if (plugin && typeof plugin[hook] === 'function') {{ plugin[hook](); }}
"#
        );
        if let Err(err) = node.eval_blocking(script) {
            self.status = format!("hook failed: {err}");
        }
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let mut lines = vec![
            Line::from(format!(
                "{} v{}",
                self.descriptor.name, self.descriptor.version
            )),
            Line::from(format!("publisher: {}", self.descriptor.publisher)),
            Line::from("runtime: js/ts via edon/libnode"),
            Line::from(format!("status: {}", self.status)),
            Line::from(""),
        ];

        if self.render_lines.is_empty() {
            lines.push(Line::from(JS_RENDER_HELP));
        } else {
            lines.extend(self.render_lines.iter().cloned().map(Line::from));
        }
        lines
    }
}

impl Plugin for JsPluginHost {
    fn id(&self) -> &str {
        &self.descriptor.id
    }

    fn name(&self) -> &str {
        &self.descriptor.name
    }

    fn display_name(&self) -> String {
        format!("{} ({})", self.descriptor.name, self.descriptor.version)
    }

    fn render_mode(&self) -> PluginRenderMode {
        PluginRenderMode::Text
    }

    fn render(&self, area: Rect, buf: &mut Buffer, ctx: &PluginContext) {
        Paragraph::new(self.lines())
            .style(
                Style::default()
                    .fg(ctx.theme.text_color)
                    .bg(ctx.theme.bg_color),
            )
            .render(area, buf);
    }

    fn handle_event(&mut self, _event: &Event, _area: Rect) -> PluginEventResult {
        PluginEventResult::Ignored
    }

    fn tick(&mut self) -> bool {
        if self.last_tick.elapsed() < Duration::from_millis(250) {
            return false;
        }
        self.last_tick = Instant::now();
        self.call_hook("tick");
        self.refresh_render_lines();
        true
    }

    fn on_activate(&mut self) {
        self.call_hook("onActivate");
    }

    fn on_deactivate(&mut self) {
        self.call_hook("onDeactivate");
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Drop for JsPluginHost {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.render_file);
    }
}

fn resolve_libnode_path() -> Option<PathBuf> {
    std::env::var_os("KRUSTY_LIBNODE")
        .or_else(|| std::env::var_os("EDON_LIBNODE_PATH"))
        .map(PathBuf::from)
}

fn is_typescript_entry(path: &std::path::Path) -> bool {
    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("ts" | "tsx" | "mts" | "cts")
    )
}

fn sanitize_for_file(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_typescript_extensions() {
        assert!(is_typescript_entry(std::path::Path::new("plugin.ts")));
        assert!(is_typescript_entry(std::path::Path::new("plugin.tsx")));
        assert!(!is_typescript_entry(std::path::Path::new("plugin.js")));
    }

    #[test]
    fn sanitizes_plugin_ids_for_temp_files() {
        assert_eq!(sanitize_for_file("scope/plugin.one"), "scope-plugin-one");
    }

    #[test]
    fn runs_ts_plugin_when_libnode_available() {
        if std::env::var_os("KRUSTY_LIBNODE").is_none()
            && std::env::var_os("EDON_LIBNODE_PATH").is_none()
        {
            eprintln!("skipping JS plugin smoke test because libnode is not configured");
            return;
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let entry = temp.path().join("index.ts");
        std::fs::write(
            &entry,
            r#"
let ticks = 0;
(globalThis as any).krusty.registerPlugin({
  tick() { ticks += 1; },
  renderText() { return ['edon smoke', `ticks ${ticks}`]; }
});
"#,
        )
        .expect("write entry");

        let descriptor = InstalledPluginDescriptor {
            id: "js-smoke".to_string(),
            name: "JS Smoke".to_string(),
            version: "0.1.0".to_string(),
            publisher: "test".to_string(),
            description: None,
            runtime: crate::plugins::PluginRuntime::Js,
            install_path: temp.path().to_path_buf(),
            entry_component_path: entry,
            enabled: true,
            render_mode: PluginRenderMode::Text,
        };

        let mut host = JsPluginHost::new(descriptor);
        assert!(host.status.contains("loaded"), "{}", host.status);
        assert!(host.render_lines.iter().any(|line| line == "edon smoke"));
        host.tick();
        assert!(host.render_lines.iter().any(|line| line.contains("ticks")));
    }
}
