//! Mitsuro desktop — GPUI shell chrome (Codex-like layout).

mod app;
mod browser;
mod components;
mod demo;
mod theme;

use std::borrow::Cow;
use std::path::PathBuf;

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

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();

    // As requested by IconName: "icons/foo.svg"
    out.push(path.clone());
    // Bundled under crate assets/
    out.push(manifest.join("assets").join(&path));
    // If caller already prefixes assets/
    out.push(manifest.join(&path));
    // Cwd variants (cargo run from workspace root)
    out.push(PathBuf::from("crates/mitsuro-desktop/assets").join(&path));
    out.push(PathBuf::from("assets").join(&path));

    out
}

fn main() {
    Application::new()
        .with_assets(FileAssets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);

            // Prefer dark component theme to match Codex tokens.
            {
                let theme = gpui_component::Theme::global_mut(cx);
                theme.mode = gpui_component::ThemeMode::Dark;
                theme.radius = px(10.0);
                theme.radius_lg = px(16.0);
                theme.shadow = false;
            }

            let bounds = Bounds::centered(None, size(px(1280.0), px(840.0)), cx);
            let open = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Mitsuro — Codex".into()),
                        appears_transparent: false,
                        traffic_light_position: None,
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| app::MitsuroApp::new(window, cx));
                    cx.new(|cx| gpui_component::Root::new(view, window, cx))
                },
            );

            if let Err(error) = open {
                eprintln!("failed to open Mitsuro window: {error}");
            }
        });
}
