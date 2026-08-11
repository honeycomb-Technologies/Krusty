//! Mitsuro desktop — GPUI shell chrome (Codex-like layout).

mod app;
mod browser;
mod components;
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

            // Prefer dark component theme to match Codex tokens.
            // `Theme::change` applies the matching component palette; assigning
            // `mode` alone leaves input foregrounds on the light-theme colors.
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            {
                let theme = gpui_component::Theme::global_mut(cx);
                theme.radius = px(10.0);
                theme.radius_lg = px(16.0);
                theme.shadow = false;
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
