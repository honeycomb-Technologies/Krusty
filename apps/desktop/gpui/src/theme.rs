//! Mitsuro desktop dark color tokens derived from the reference visual language.
//!
//! Key surfaces (dark / electron-dark):
//! - app / surface-under: `#0d0d0d` / near-black rails
//! - elevated primary: `#212121`
//! - text foreground: white / gray-0
//! - text secondary: `#ffffffb3`
//! - text tertiary: `#ffffff80`
//! - border heavy: `#ffffff29`

use gpui::{rgb, rgba, Hsla};

/// Convert 0xRRGGBB to GPUI Hsla.
pub fn hex(rgb_value: u32) -> Hsla {
    rgb(rgb_value).into()
}

/// Convert 0xRRGGBB + alpha (0.0–1.0) to Hsla via 0xRRGGBBAA.
pub fn hex_alpha(rgb_value: u32, alpha: f32) -> Hsla {
    let a = ((alpha.clamp(0.0, 1.0) * 255.0).round() as u32).min(255);
    rgba((rgb_value << 8) | a).into()
}

/// Codex-like dark palette used by the shell.
#[derive(Clone, Copy, Debug)]
pub struct CodexColors {
    /// Deepest underlay (window fill).
    pub bg_under: Hsla,
    /// Main app / transcript background (near black). Alias of under for theme maps.
    #[allow(dead_code)]
    pub bg_app: Hsla,
    /// Activity rail.
    pub bg_rail: Hsla,
    /// Sidebar surface.
    pub bg_sidebar: Hsla,
    /// Transcript / main surface.
    pub bg_main: Hsla,
    /// Composer / input surface (elevated family).
    #[allow(dead_code)]
    pub bg_composer: Hsla,
    /// Elevated chip / status surfaces.
    pub bg_elevated: Hsla,
    /// Selected row / active rail item.
    pub bg_selected: Hsla,
    /// Hover fill.
    pub bg_hover: Hsla,
    /// Secondary button fill.
    pub bg_button_secondary: Hsla,
    /// Codex dark primary: `color-text-foreground` (white/fg fill).
    pub bg_button_primary: Hsla,
    /// Label on primary button (near-black underlay).
    pub fg_button_primary: Hsla,
    /// Primary hover (slightly dimmed fg).
    pub bg_button_primary_hover: Hsla,
    /// Primary active press.
    pub bg_button_primary_active: Hsla,
    pub border: Hsla,
    #[allow(dead_code)]
    pub border_subtle: Hsla,
    pub border_heavy: Hsla,
    pub text: Hsla,
    pub text_secondary: Hsla,
    pub text_tertiary: Hsla,
    pub accent: Hsla,
    pub accent_soft: Hsla,
    /// Codex "Full access" / warning orange (bar composer chip).
    pub accent_orange: Hsla,
    pub status_ready: Hsla,
    pub status_connecting: Hsla,
    pub status_error: Hsla,
    pub status_offline: Hsla,
    /// Unified-diff addition line (green-ish).
    pub diff_add: Hsla,
    /// Unified-diff deletion line (red-ish).
    pub diff_del: Hsla,
    /// Diff meta / hunk header.
    pub diff_meta: Hsla,
}

impl Default for CodexColors {
    fn default() -> Self {
        Self::dark()
    }
}

impl CodexColors {
    pub fn dark() -> Self {
        Self {
            // #0d0d0d family — soft near-black surfaces (Codex density)
            bg_under: hex(0x0d0d0d),
            bg_app: hex(0x0d0d0d),
            // Rail matches underlay so first paint doesn't read as chrome scaffold
            bg_rail: hex(0x0d0d0d),
            bg_sidebar: hex(0x111111),
            bg_main: hex(0x0d0d0d),
            bg_composer: hex(0x1a1a1a),
            bg_elevated: hex(0x1a1a1a),
            bg_selected: hex_alpha(0xffffff, 0.06),
            bg_hover: hex_alpha(0xffffff, 0.04),
            bg_button_secondary: hex_alpha(0xffffff, 0.06),
            // electron-dark: --color-background-button-primary: var(--color-text-foreground)
            bg_button_primary: hex(0xffffff),
            fg_button_primary: hex(0x0d0d0d),
            bg_button_primary_hover: hex_alpha(0xffffff, 0.90),
            bg_button_primary_active: hex_alpha(0xffffff, 0.78),
            // Softest chrome borders (fail-closed product density)
            border: hex_alpha(0xffffff, 0.05),
            border_subtle: hex_alpha(0xffffff, 0.03),
            border_heavy: hex_alpha(0xffffff, 0.08),
            text: hex(0xffffff),
            text_secondary: hex_alpha(0xffffff, 0.68),
            text_tertiary: hex_alpha(0xffffff, 0.42),
            // blue-300
            accent: hex(0x339cff),
            accent_soft: hex_alpha(0x339cff, 0.12),
            // orange-400-ish — Full access chip on bar composer
            accent_orange: hex(0xf5a524),
            status_ready: hex(0x04b84c),
            status_connecting: hex(0xf5a524),
            status_error: hex(0xfa423e),
            status_offline: hex(0x8a8a8a),
            // Soft git-style diff hues on dark surfaces
            diff_add: hex(0x3dd68c),
            diff_del: hex(0xff6b6b),
            diff_meta: hex(0x8b9cff),
        }
    }
}

/// Soft ambient wash for main surfaces — dark blue radial-ish feel via
/// linear gradient (not OpenAI bloom trademark). Base underlay only; multi-blob
/// atmosphere is layered in `ambient_atmosphere_layers`.
pub fn ambient_main_bg() -> gpui::Background {
    use gpui::{linear_color_stop, linear_gradient};
    linear_gradient(
        165.0,
        linear_color_stop(hex(0x0c1016), 0.0),
        linear_color_stop(hex(0x0d0d0d), 0.62),
    )
}

/// Diagonal cool wash (upper-left → lower-right) for atmosphere stack.
pub fn ambient_wash_cool() -> gpui::Background {
    use gpui::{linear_color_stop, linear_gradient};
    linear_gradient(
        125.0,
        linear_color_stop(hex_alpha(0x1a3a5c, 0.22), 0.0),
        linear_color_stop(hex_alpha(0x0d0d0d, 0.0), 0.55),
    )
}

/// Warm accent wash (lower-left) — soft amber, very low alpha.
pub fn ambient_wash_warm() -> gpui::Background {
    use gpui::{linear_color_stop, linear_gradient};
    linear_gradient(
        45.0,
        linear_color_stop(hex_alpha(0x3a2818, 0.28), 0.0),
        linear_color_stop(hex_alpha(0x0d0d0d, 0.0), 0.48),
    )
}

/// Teal/cyan wash (upper-right) — cool product depth, not trademark bloom.
pub fn ambient_wash_teal() -> gpui::Background {
    use gpui::{linear_color_stop, linear_gradient};
    linear_gradient(
        220.0,
        linear_color_stop(hex_alpha(0x0e2a32, 0.26), 0.0),
        linear_color_stop(hex_alpha(0x0d0d0d, 0.0), 0.52),
    )
}

/// Deep center vignette to keep hero readable on multi-wash stage.
pub fn ambient_wash_vignette() -> gpui::Background {
    use gpui::{linear_color_stop, linear_gradient};
    linear_gradient(
        180.0,
        linear_color_stop(hex_alpha(0x0d0d0d, 0.0), 0.0),
        linear_color_stop(hex_alpha(0x050505, 0.45), 1.0),
    )
}

/// Quieter elevated fill for empty-state hero cards.
#[allow(dead_code)]
pub fn ambient_glow_stop() -> Hsla {
    hex_alpha(0x1a3a5c, 0.35)
}

pub fn colors() -> CodexColors {
    CodexColors::dark()
}

/// Fully transparent fill (inactive rail / ghost base).
pub fn transparent() -> Hsla {
    hex_alpha(0x000000, 0.0)
}
