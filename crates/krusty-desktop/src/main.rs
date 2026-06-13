mod api;
mod app;
mod branding;
mod chat;
mod components;
mod design;
mod panel_actions;
mod panels;
mod server;

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::Result;
use gpui::{
    px, size, App, AppContext as _, Application, AssetSource, Bounds, SharedString, WindowBounds,
    WindowOptions,
};

struct FileAssets;

impl AssetSource for FileAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path.is_empty() {
            return Ok(None);
        }

        for candidate in asset_candidates(path) {
            if candidate.exists() {
                return std::fs::read(candidate)
                    .map(|bytes| Some(Cow::Owned(bytes)))
                    .map_err(Into::into);
            }
        }

        Ok(None)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(std::fs::read_dir(Path::new(path))?
            .filter_map(|entry| {
                entry
                    .ok()
                    .map(|entry| SharedString::from(entry.path().display().to_string()))
            })
            .collect())
    }
}

fn main() {
    Application::new()
        .with_assets(FileAssets)
        .run(|cx: &mut App| {
            if let Err(error) = load_custom_fonts(cx) {
                eprintln!("failed to load custom fonts: {error:#}");
            }

            gpui_component::init(cx);
            app::init(cx);
            design::theme::init(cx);
            design::theme::apply_component_theme(cx);

            let bounds = Bounds::centered(None, size(px(1180.0), px(780.0)), cx);
            if let Err(error) = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| app::KrustyDesktop::new(window, cx));
                    cx.new(|cx| gpui_component::Root::new(view, window, cx))
                },
            ) {
                eprintln!("failed to open Krusty desktop window: {error}");
            }
        });
}

fn asset_candidates(path: &str) -> Vec<PathBuf> {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        return vec![path];
    }

    vec![PathBuf::from(&path), workspace_root().join(&path)]
}

fn load_custom_fonts(cx: &mut App) -> Result<()> {
    cx.text_system().add_fonts(vec![
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/Inter-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/NotoSans-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/JetBrainsMono-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/FiraCode-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/FiraMono-Medium.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/Hack-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/SourceCodePro-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/IBMPlexMono-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/NotoSansMono-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/Inconsolata-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/SpaceMono-Regular.ttf"
        ))),
        Cow::Borrowed(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/fonts/VictorMono-Regular.ttf"
        ))),
    ])?;

    Ok(())
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(manifest)
}
