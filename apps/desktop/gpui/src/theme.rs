//! Mitsuro desktop semantic tokens derived from the current reference visual language.
//!
//! Key surfaces (dark / electron-dark):
//! - app / surface-under: `#0d0d0d` / near-black rails
//! - elevated primary: `#212121`
//! - text foreground: white / gray-0
//! - text secondary: `#ffffffb3`
//! - text tertiary: `#ffffff80`
//! - border heavy: `#ffffff29`

use std::time::Duration;

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

/// Mitsuro dark palette used by every desktop surface.
#[derive(Clone, Copy, Debug)]
pub struct MitsuroColors {
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
    /// High-contrast keyboard focus outline; never reused as a selection fill.
    pub focus_ring: Hsla,
    /// Modal/popover scrim over the app surface.
    pub overlay_scrim: Hsla,
    /// Shadow color for the few genuinely elevated surfaces.
    pub shadow: Hsla,
    /// Destructive hover/selection tint.
    pub destructive_soft: Hsla,
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

impl Default for MitsuroColors {
    fn default() -> Self {
        Self::dark()
    }
}

impl MitsuroColors {
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
            focus_ring: hex_alpha(0xffffff, 0.88),
            overlay_scrim: hex_alpha(0x000000, 0.58),
            shadow: hex_alpha(0x000000, 0.48),
            destructive_soft: hex_alpha(0xfa423e, 0.14),
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

/// Compact 2/4/6/8/12/16/24/32 spacing rhythm used across shell primitives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MitsuroSpacing {
    pub xxs: f32,
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
    pub xxl: f32,
    pub xxxl: f32,
}

impl Default for MitsuroSpacing {
    fn default() -> Self {
        Self {
            xxs: 2.0,
            xs: 4.0,
            sm: 6.0,
            md: 8.0,
            lg: 12.0,
            xl: 16.0,
            xxl: 24.0,
            xxxl: 32.0,
        }
    }
}

/// Text sizes and line-height ratios for each stable information role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MitsuroTypography {
    pub window_chrome: f32,
    pub navigation: f32,
    pub body: f32,
    pub message: f32,
    pub code: f32,
    pub label: f32,
    pub metadata: f32,
    pub button: f32,
    pub heading: f32,
    pub title: f32,
    pub compact_line_height: f32,
    pub body_line_height: f32,
    pub message_line_height: f32,
}

impl Default for MitsuroTypography {
    fn default() -> Self {
        Self {
            window_chrome: 13.0,
            navigation: 14.0,
            body: 15.0,
            message: 16.0,
            code: 14.0,
            label: 12.0,
            metadata: 11.0,
            button: 14.0,
            heading: 20.0,
            title: 28.0,
            compact_line_height: 1.25,
            body_line_height: 1.45,
            message_line_height: 1.55,
        }
    }
}

/// Shared shape, hit-target, icon, border, and opacity values.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MitsuroShape {
    pub radius_xs: f32,
    pub radius_sm: f32,
    pub radius_md: f32,
    pub radius_lg: f32,
    pub radius_xl: f32,
    pub radius_pill: f32,
    pub border_hairline: f32,
    pub border_focus: f32,
    pub icon_sm: f32,
    pub icon_md: f32,
    pub icon_lg: f32,
    pub control_sm: f32,
    pub control_md: f32,
    pub control_lg: f32,
    pub hit_target_min: f32,
    pub disabled_opacity: f32,
    pub muted_opacity: f32,
    pub shadow_blur: f32,
}

impl Default for MitsuroShape {
    fn default() -> Self {
        Self {
            radius_xs: 4.0,
            radius_sm: 6.0,
            radius_md: 8.0,
            radius_lg: 12.0,
            radius_xl: 20.0,
            radius_pill: 999.0,
            border_hairline: 1.0,
            border_focus: 2.0,
            icon_sm: 14.0,
            icon_md: 16.0,
            icon_lg: 20.0,
            control_sm: 28.0,
            control_md: 34.0,
            control_lg: 40.0,
            hit_target_min: 34.0,
            disabled_opacity: 0.42,
            muted_opacity: 0.68,
            shadow_blur: 20.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MitsuroEasing {
    Standard,
    Enter,
    Exit,
}

/// Centralized restrained motion. Reduced motion resolves every duration to zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MitsuroMotion {
    pub hover: Duration,
    pub state: Duration,
    pub panel: Duration,
    pub tooltip_delay: Duration,
    pub standard_easing: MitsuroEasing,
    pub enter_easing: MitsuroEasing,
    pub exit_easing: MitsuroEasing,
    pub reduced: bool,
}

impl Default for MitsuroMotion {
    fn default() -> Self {
        Self {
            hover: Duration::from_millis(90),
            state: Duration::from_millis(140),
            panel: Duration::from_millis(180),
            tooltip_delay: Duration::from_millis(450),
            standard_easing: MitsuroEasing::Standard,
            enter_easing: MitsuroEasing::Enter,
            exit_easing: MitsuroEasing::Exit,
            reduced: false,
        }
    }
}

impl MitsuroMotion {
    pub fn reduced() -> Self {
        Self {
            hover: Duration::ZERO,
            state: Duration::ZERO,
            panel: Duration::ZERO,
            tooltip_delay: Duration::ZERO,
            reduced: true,
            ..Self::default()
        }
    }
}

/// Reference-grounded shell and control measurements.
///
/// Feature components should consume these values rather than introducing new
/// top-level widths and heights. Component-local geometry still belongs beside
/// the component when it is genuinely unique.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MitsuroMetrics {
    pub root_rem_size: f32,
    pub title_bar_height: f32,
    pub toolbar_height: f32,
    /// Coding workspace rail from the current Codex reference.
    pub sidebar_width: f32,
    /// ChatGPT connection browser is intentionally wider than the Codex rail.
    pub chat_sidebar_width: f32,
    /// Open Codex thread rail; composer inset yields the 736px live shell.
    pub thread_content_max_width: f32,
    /// ChatGPT conversation mode retains the wider reading rail.
    pub chat_thread_content_max_width: f32,
    pub composer_max_width: f32,
    pub chat_home_composer_max_width: f32,
    pub composer_radius: f32,
    pub popover_radius: f32,
    pub icon_button_size: f32,
}

impl Default for MitsuroMetrics {
    fn default() -> Self {
        Self {
            // The installed Electron reference renders its 16px CSS root at
            // roughly 1.2 framebuffer pixels on this desktop. GPUI otherwise
            // presents 14px `text_sm` where the reference is about 17px.
            root_rem_size: 19.2,
            title_bar_height: 42.0,
            toolbar_height: 56.0,
            sidebar_width: 275.0,
            chat_sidebar_width: 329.0,
            thread_content_max_width: 768.0,
            chat_thread_content_max_width: 864.0,
            composer_max_width: 912.0,
            chat_home_composer_max_width: 768.0,
            composer_radius: 20.0,
            popover_radius: 12.0,
            icon_button_size: 34.0,
        }
    }
}

/// One semantic token bundle for the GPUI shell and shared primitives.
#[derive(Clone, Copy, Debug, Default)]
pub struct MitsuroThemeTokens {
    pub colors: MitsuroColors,
    pub metrics: MitsuroMetrics,
    pub spacing: MitsuroSpacing,
    pub typography: MitsuroTypography,
    pub shape: MitsuroShape,
    pub motion: MitsuroMotion,
}

pub fn tokens() -> MitsuroThemeTokens {
    MitsuroThemeTokens::default()
}

pub fn colors() -> MitsuroColors {
    tokens().colors
}

pub fn metrics() -> MitsuroMetrics {
    tokens().metrics
}

pub fn spacing() -> MitsuroSpacing {
    tokens().spacing
}

pub fn typography() -> MitsuroTypography {
    tokens().typography
}

pub fn shape() -> MitsuroShape {
    tokens().shape
}

pub fn motion() -> MitsuroMotion {
    let reduced = std::env::var("MITSURO_REDUCED_MOTION")
        .ok()
        .is_some_and(|value| matches!(value.trim(), "1" | "true" | "yes" | "on"));
    if reduced {
        MitsuroMotion::reduced()
    } else {
        tokens().motion
    }
}

/// Fully transparent fill (inactive rail / ghost base).
pub fn transparent() -> Hsla {
    hex_alpha(0x000000, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_scales_are_monotonic_and_reference_grounded() {
        let tokens = tokens();
        assert!(tokens.spacing.xxs < tokens.spacing.xs);
        assert!(tokens.spacing.xs < tokens.spacing.md);
        assert!(tokens.spacing.md < tokens.spacing.xl);
        assert!(tokens.shape.control_sm < tokens.shape.control_md);
        assert!(tokens.shape.control_md < tokens.shape.control_lg);
        assert!(tokens.shape.hit_target_min >= tokens.shape.control_md);
        assert_eq!(tokens.metrics.title_bar_height, 42.0);
        assert_eq!(tokens.metrics.sidebar_width, 275.0);
        assert_eq!(tokens.metrics.chat_sidebar_width, 329.0);
        assert_eq!(tokens.metrics.thread_content_max_width, 768.0);
        assert_eq!(tokens.metrics.chat_thread_content_max_width, 864.0);
    }

    #[test]
    fn reduced_motion_zeroes_every_timed_transition() {
        let motion = MitsuroMotion::reduced();
        assert!(motion.reduced);
        assert_eq!(motion.hover, Duration::ZERO);
        assert_eq!(motion.state, Duration::ZERO);
        assert_eq!(motion.panel, Duration::ZERO);
        assert_eq!(motion.tooltip_delay, Duration::ZERO);
    }
}
