mod app;

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

pub fn run() {
    Application::new()
        .with_assets(FileAssets)
        .run(|cx: &mut App| {
            if let Err(error) = load_custom_fonts(cx) {
                eprintln!("failed to load custom fonts: {error:#}");
            }
            gpui_component::init(cx);

            let bounds = Bounds::centered(None, size(px(430.0), px(860.0)), cx);
            if let Err(error) = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| app::KrustyMobile::new(window, cx));
                    cx.new(|cx| gpui_component::Root::new(view, window, cx))
                },
            ) {
                eprintln!("failed to open Krusty mobile window: {error}");
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
            "/../../assets/fonts/JetBrainsMono-Regular.ttf"
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

#[cfg(target_os = "ios")]
mod ios_ffi {
    use std::ffi::c_void;
    use std::sync::Once;

    static START: Once = Once::new();

    extern "C" {
        fn gpui_ios_set_host_view(view: *mut c_void);
    }

    #[no_mangle]
    pub extern "C" fn krusty_mobile_start() {
        START.call_once(super::run);
    }

    #[no_mangle]
    pub extern "C" fn krusty_mobile_start_with_host_view(view: *mut c_void) {
        unsafe {
            gpui_ios_set_host_view(view);
        }
        START.call_once(super::run);
    }
}
