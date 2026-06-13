use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};

use gpui::{hsla, px, App, Hsla, SharedString, Window};
use serde::{Deserialize, Serialize};

const DEFAULT_THEME_NAME: &str = "Default Dark";
const CONFIG_DIR_NAME: &str = "krusty-desktop";
const APPEARANCE_FILE_NAME: &str = "appearance.json";
const THEMES_DIR_NAME: &str = "themes";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppearanceSettings {
    pub theme_name: String,
    pub font: AppFont,
}

impl AppearanceSettings {
    pub fn with_theme_name(&self, theme_name: impl Into<String>) -> Self {
        Self {
            theme_name: theme_name.into(),
            font: self.font,
        }
    }

    pub fn with_font(&self, font: AppFont) -> Self {
        Self {
            font,
            ..self.clone()
        }
    }
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme_name: DEFAULT_THEME_NAME.to_owned(),
            font: AppFont::Inter,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppFont {
    #[default]
    #[serde(alias = "system", alias = "plantin")]
    Inter,
    NotoSans,
    JetBrainsMono,
    FiraCode,
    #[serde(alias = "mono")]
    FiraMono,
    Hack,
    SourceCodePro,
    IbmPlexMono,
    NotoSansMono,
    Inconsolata,
    SpaceMono,
    VictorMono,
}

impl AppFont {
    pub const ALL: [Self; 12] = [
        Self::Inter,
        Self::NotoSans,
        Self::JetBrainsMono,
        Self::FiraCode,
        Self::FiraMono,
        Self::Hack,
        Self::SourceCodePro,
        Self::IbmPlexMono,
        Self::NotoSansMono,
        Self::Inconsolata,
        Self::SpaceMono,
        Self::VictorMono,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Inter => "Inter",
            Self::NotoSans => "Noto Sans",
            Self::JetBrainsMono => "JetBrains Mono",
            Self::FiraCode => "Fira Code",
            Self::FiraMono => "Fira Mono",
            Self::Hack => "Hack",
            Self::SourceCodePro => "Source Code Pro",
            Self::IbmPlexMono => "IBM Plex Mono",
            Self::NotoSansMono => "Noto Sans Mono",
            Self::Inconsolata => "Inconsolata",
            Self::SpaceMono => "Space Mono",
            Self::VictorMono => "Victor Mono",
        }
    }

    pub fn family(self) -> &'static str {
        match self {
            Self::Inter => "Inter",
            Self::NotoSans => "Noto Sans",
            Self::JetBrainsMono => "JetBrains Mono",
            Self::FiraCode => "Fira Code",
            Self::FiraMono => "Fira Mono",
            Self::Hack => "Hack",
            Self::SourceCodePro => "Source Code Pro",
            Self::IbmPlexMono => "IBM Plex Mono",
            Self::NotoSansMono => "Noto Sans Mono",
            Self::Inconsolata => "Inconsolata",
            Self::SpaceMono => "Space Mono",
            Self::VictorMono => "Victor Mono",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub app_bg: Hsla,
    pub surface: Hsla,
    pub surface_hover: Hsla,
    pub surface_selected: Hsla,
    pub text: Hsla,
    pub text_muted: Hsla,
    pub hairline: Hsla,
    pub accent: Hsla,
    pub complement: Hsla,
    pub danger: Hsla,
    pub danger_soft: Hsla,
    pub success: Hsla,
    pub grid_minor: Hsla,
    pub grid_major: Hsla,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeOption {
    pub name: String,
    pub mode: gpui_component::ThemeMode,
    pub is_default: bool,
    pub palette: Palette,
}

static APPEARANCE: OnceLock<RwLock<AppearanceSettings>> = OnceLock::new();
static PALETTE: OnceLock<RwLock<Palette>> = OnceLock::new();

pub fn init(cx: &mut App) {
    load_saved_appearance();
    if let Err(error) = install_bundled_themes() {
        eprintln!("failed to install bundled Krusty themes: {error:#}");
    }
    if let Some(themes_dir) = themes_dir() {
        if let Err(error) = gpui_component::ThemeRegistry::watch_dir(themes_dir, cx, |cx| {
            apply_component_theme(cx);
        }) {
            eprintln!("failed to watch Krusty theme directory: {error:#}");
        }
    }
}

pub fn current_appearance() -> AppearanceSettings {
    let lock = APPEARANCE.get_or_init(|| RwLock::new(AppearanceSettings::default()));
    match lock.read() {
        Ok(guard) => guard.clone(),
        Err(error) => error.into_inner().clone(),
    }
}

pub fn set_appearance(settings: AppearanceSettings) {
    let lock = APPEARANCE.get_or_init(|| RwLock::new(AppearanceSettings::default()));
    match lock.write() {
        Ok(mut guard) => *guard = settings,
        Err(error) => *error.into_inner() = settings,
    }
}

pub fn save_appearance(settings: &AppearanceSettings) -> std::io::Result<()> {
    let Some(path) = appearance_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(path, bytes)
}

pub fn available_themes(cx: &App) -> Vec<ThemeOption> {
    gpui_component::ThemeRegistry::global(cx)
        .sorted_themes()
        .into_iter()
        .map(|config| {
            let mut preview = gpui_component::Theme::default();
            preview.apply_config(config);
            ThemeOption {
                name: config.name.to_string(),
                mode: config.mode,
                is_default: config.is_default,
                palette: palette_from_component_theme(&preview),
            }
        })
        .collect()
}

pub fn apply_component_theme(cx: &mut App) {
    let settings = current_appearance();
    apply_theme_config(&settings, cx);
    apply_krusty_component_overrides(&settings, cx);
    cache_current_palette(cx);
}

pub fn apply_component_theme_for_window(window: &mut Window, cx: &mut App) {
    apply_component_theme(cx);
    window.refresh();
}

fn apply_theme_config(settings: &AppearanceSettings, cx: &mut App) {
    let config = gpui_component::ThemeRegistry::global(cx)
        .themes()
        .get(settings.theme_name.as_str())
        .cloned()
        .or_else(|| {
            let registry = gpui_component::ThemeRegistry::global(cx);
            registry.themes().get(DEFAULT_THEME_NAME).cloned()
        })
        .or_else(|| {
            let registry = gpui_component::ThemeRegistry::global(cx);
            Some(registry.default_dark_theme().clone())
        });

    if let Some(config) = config {
        gpui_component::Theme::global_mut(cx).apply_config(&config);
    }
}

fn apply_krusty_component_overrides(settings: &AppearanceSettings, cx: &mut App) {
    let theme = gpui_component::Theme::global_mut(cx);
    theme.radius = px(0.0);
    theme.radius_lg = px(0.0);
    theme.shadow = false;
    theme.tile_radius = px(0.0);
    theme.tile_shadow = false;
    theme.scrollbar_show = gpui_component::scroll::ScrollbarShow::Always;

    theme.font_family = SharedString::from(settings.font.family());
    if settings.font == AppFont::FiraMono {
        theme.mono_font_family = SharedString::from(settings.font.family());
    }
}

fn cache_current_palette(cx: &App) {
    let palette = palette_from_component_theme(gpui_component::Theme::global(cx));
    let lock = PALETTE.get_or_init(|| RwLock::new(default_palette()));
    match lock.write() {
        Ok(mut guard) => *guard = palette,
        Err(error) => *error.into_inner() = palette,
    }
}

fn current_palette() -> Palette {
    let lock = PALETTE.get_or_init(|| RwLock::new(default_palette()));
    match lock.read() {
        Ok(guard) => *guard,
        Err(error) => *error.into_inner(),
    }
}

fn palette_from_component_theme(theme: &gpui_component::Theme) -> Palette {
    Palette {
        app_bg: theme.background,
        surface: theme.popover,
        surface_hover: theme.secondary_hover,
        surface_selected: theme.secondary_active,
        text: theme.foreground,
        text_muted: theme.muted_foreground,
        hairline: theme.border,
        accent: theme.primary,
        complement: theme.warning,
        danger: theme.danger,
        danger_soft: theme.background.blend(theme.danger.opacity(0.16)),
        success: theme.success,
        grid_minor: theme.foreground.opacity(0.028),
        grid_major: theme.foreground.opacity(0.075),
    }
}

fn default_palette() -> Palette {
    let colors = gpui_component::ThemeColor::dark();
    Palette {
        app_bg: colors.background,
        surface: colors.popover,
        surface_hover: colors.secondary_hover,
        surface_selected: colors.secondary_active,
        text: colors.foreground,
        text_muted: colors.muted_foreground,
        hairline: colors.border,
        accent: colors.primary,
        complement: colors.warning,
        danger: colors.danger,
        danger_soft: colors.background.blend(colors.danger.opacity(0.16)),
        success: colors.success,
        grid_minor: hsla(0.0, 0.0, 1.0, 0.028),
        grid_major: hsla(0.0, 0.0, 1.0, 0.075),
    }
}

fn load_saved_appearance() {
    let Some(path) = appearance_path() else {
        return;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    let Ok(settings) = serde_json::from_slice::<AppearanceSettings>(&bytes) else {
        return;
    };
    set_appearance(settings);
}

fn appearance_path() -> Option<PathBuf> {
    Some(config_dir()?.join(APPEARANCE_FILE_NAME))
}

fn themes_dir() -> Option<PathBuf> {
    Some(config_dir()?.join(THEMES_DIR_NAME))
}

fn install_bundled_themes() -> std::io::Result<()> {
    let Some(target_dir) = themes_dir() else {
        return Ok(());
    };
    let source_dir = bundled_themes_dir();
    if !source_dir.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(&target_dir)?;
    for entry in std::fs::read_dir(source_dir)? {
        let entry = entry?;
        let source = entry.path();
        if source.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let target = target_dir.join(entry.file_name());
        if !target.exists() {
            std::fs::copy(source, target)?;
        }
    }
    Ok(())
}

fn bundled_themes_dir() -> PathBuf {
    workspace_root().join("assets/gpui-component-themes")
}

fn config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|path| path.join(CONFIG_DIR_NAME))
}

fn workspace_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or(manifest)
}

pub fn app_bg() -> Hsla {
    current_palette().app_bg
}

pub fn surface() -> Hsla {
    current_palette().surface
}

pub fn surface_hover() -> Hsla {
    current_palette().surface_hover
}

pub fn surface_selected() -> Hsla {
    current_palette().surface_selected
}

pub fn text() -> Hsla {
    current_palette().text
}

pub fn text_muted() -> Hsla {
    current_palette().text_muted
}

pub fn hairline() -> Hsla {
    current_palette().hairline
}

pub fn accent() -> Hsla {
    current_palette().accent
}

pub fn success() -> Hsla {
    current_palette().success
}

pub fn complement() -> Hsla {
    current_palette().complement
}

pub fn danger() -> Hsla {
    current_palette().danger
}

pub fn logo_gradient_stops() -> [Hsla; 7] {
    let appearance = current_appearance();
    if matches!(
        appearance.theme_name.as_str(),
        "Default Dark" | "Default Light"
    ) {
        return original_logo_gradient_stops();
    }

    let palette = current_palette();
    let low = palette.app_bg.blend(palette.accent.opacity(0.55));
    let mid = palette.surface.blend(palette.accent.opacity(0.82));
    let peak = palette.complement;
    let tail = palette.surface.blend(palette.complement.opacity(0.72));

    [low, mid, palette.accent, peak, palette.accent, tail, low]
}

fn original_logo_gradient_stops() -> [Hsla; 7] {
    [
        logo_color(0x8b4513),
        logo_color(0xcd853f),
        logo_color(0xff6b35),
        logo_color(0xffcc00),
        logo_color(0xff6b35),
        logo_color(0xcd853f),
        logo_color(0x8b4513),
    ]
}

fn logo_color(hex: u32) -> Hsla {
    gpui::rgb(hex).into()
}
