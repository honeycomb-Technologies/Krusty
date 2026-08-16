//! Mitsuro desktop — GPUI shell chrome (Codex-like layout).

mod app;
mod browser;
mod components;
pub mod connection_registry;
mod demo;
mod mcp_app_runtime;
mod preferences;
mod theme;

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use gpui::{
    px, size, App, AppContext as _, Application, AssetSource, Bounds, WindowBounds, WindowOptions,
};

/// Load SVG icons (and other assets) for `gpui_component::Icon`.
///
/// `gpui-component` does **not** ship Lucide SVGs — paths like `icons/plus.svg`
/// must resolve via this `AssetSource` (see docs: Icons & Assets).
struct FileAssets;

impl AssetSource for FileAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        for candidate in asset_candidates(path) {
            if candidate.is_file() {
                return std::fs::read(&candidate)
                    .map(|bytes| Some(Cow::Owned(bytes)))
                    .map_err(Into::into);
            }
        }

        Ok(None)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<gpui::SharedString>> {
        for candidate in asset_candidates(path) {
            if candidate.is_dir() {
                let mut names = Vec::new();
                if let Ok(rd) = std::fs::read_dir(&candidate) {
                    for entry in rd.flatten() {
                        names.push(gpui::SharedString::from(entry.path().display().to_string()));
                    }
                }
                return Ok(names);
            }
        }
        Ok(Vec::new())
    }
}

fn asset_candidates(path: &str) -> Vec<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return vec![path];
    }

    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("MITSURO_GPUI_ASSET_DIR") {
        candidates.push(PathBuf::from(root).join(&path));
    }
    if let Ok(executable) = std::env::current_exe() {
        for root in executable_asset_roots(&executable) {
            candidates.push(root.join(&path));
        }
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    candidates.extend([
        // As requested by IconName: "icons/foo.svg"
        path.clone(),
        // Bundled under crate assets/
        manifest.join("assets").join(&path),
        // If caller already prefixes assets/
        manifest.join(&path),
        // Cwd variants (cargo run from workspace root)
        PathBuf::from("crates/mitsuro-desktop/assets").join(&path),
        PathBuf::from("assets").join(&path),
    ]);
    candidates
}

fn executable_asset_roots(executable: &Path) -> Vec<PathBuf> {
    let Some(bin_dir) = executable.parent() else {
        return Vec::new();
    };
    let mut roots = vec![bin_dir.join("assets")];
    if let Some(prefix) = bin_dir.parent() {
        roots.push(prefix.join("share/mitsuro-gpui-desktop/assets"));
    }
    roots
}

fn main() {
    Application::new()
        .with_assets(FileAssets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            app::init_keybindings(cx);

            // Prefer dark component theme to match Codex tokens.
            // `Theme::change` applies the matching component palette; assigning
            // `mode` alone leaves input foregrounds on the light-theme colors.
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            {
                let component_theme = gpui_component::Theme::global_mut(cx);
                let tokens = theme::tokens();
                component_theme.font_size = px(tokens.typography.body);
                component_theme.mono_font_size = px(tokens.typography.code);
                component_theme.radius = px(tokens.shape.radius_md);
                component_theme.radius_lg = px(tokens.shape.radius_xl);
                component_theme.shadow = !theme::motion().reduced;
                component_theme.background = tokens.colors.bg_main;
                component_theme.foreground = tokens.colors.text;
                component_theme.border = tokens.colors.border;
                component_theme.input = tokens.colors.border_heavy;
                component_theme.ring = tokens.colors.focus_ring;
                component_theme.selection = tokens.colors.accent_soft;
                component_theme.muted = tokens.colors.bg_elevated;
                component_theme.muted_foreground = tokens.colors.text_tertiary;
                component_theme.popover = tokens.colors.bg_elevated;
                component_theme.popover_foreground = tokens.colors.text;
                component_theme.primary = tokens.colors.bg_button_primary;
                component_theme.primary_hover = tokens.colors.bg_button_primary_hover;
                component_theme.primary_active = tokens.colors.bg_button_primary_active;
                component_theme.primary_foreground = tokens.colors.fg_button_primary;
                component_theme.secondary = tokens.colors.bg_button_secondary;
                component_theme.secondary_hover = tokens.colors.bg_hover;
                component_theme.secondary_active = tokens.colors.bg_selected;
                component_theme.secondary_foreground = tokens.colors.text;
                component_theme.danger = tokens.colors.status_error;
                component_theme.danger_hover = tokens.colors.destructive_soft;
                component_theme.danger_active = tokens.colors.status_error;
                component_theme.sidebar = tokens.colors.bg_sidebar;
                component_theme.sidebar_border = tokens.colors.border_subtle;
                component_theme.sidebar_accent = tokens.colors.bg_selected;
                component_theme.sidebar_accent_foreground = tokens.colors.text;
            }

            let bounds = Bounds::centered(None, size(px(1280.0), px(840.0)), cx);
            let open = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui_component::TitleBar::title_bar_options()),
                    app_id: Some("io.mitsuro.desktop".to_owned()),
                    ..Default::default()
                },
                |window, cx| {
                    window.set_rem_size(px(theme::metrics().root_rem_size));
                    let view = cx.new(|cx| app::MitsuroApp::new(window, cx));
                    cx.new(|cx| gpui_component::Root::new(view, window, cx))
                },
            );

            if let Err(error) = open {
                eprintln!("failed to open Mitsuro window: {error}");
            }
        });
}

#[cfg(test)]
mod tests {
    use super::executable_asset_roots;
    use std::path::{Path, PathBuf};

    #[test]
    fn installed_binary_resolves_packaged_assets() {
        assert_eq!(
            executable_asset_roots(Path::new("/usr/bin/mitsuro-desktop")),
            vec![
                PathBuf::from("/usr/bin/assets"),
                PathBuf::from("/usr/share/mitsuro-gpui-desktop/assets"),
            ]
        );
    }
}
