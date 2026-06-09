mod api;
mod app;
mod components;
mod design;
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
    let root = workspace_root();
    let font_dir = root.join("assets/fonts");
    if !font_dir.exists() {
        return Ok(());
    }

    let fonts = [
        "PlantinNowVariable-Upright.woff2",
        "PlantinNowVariable-Italic.woff2",
    ]
    .into_iter()
    .filter_map(|file| {
        let path = font_dir.join(file);
        path.exists().then(|| std::fs::read(path).map(Cow::Owned))
    })
    .collect::<std::io::Result<Vec<_>>>()?;

    if !fonts.is_empty() {
        cx.text_system().add_fonts(fonts)?;
    }

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
