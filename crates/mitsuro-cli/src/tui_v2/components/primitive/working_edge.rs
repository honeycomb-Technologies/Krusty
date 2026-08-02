//! Bottom purple working edge (Toad-style thin line, Mitsuro purple ramp).
//!
//! Geometry: one full-width row of `━` (or `─` in ascii), border thickness —
//! not a dense block bar. Color is dark→light purple only (no white).
//! Scrolls left→right while the agent is running.

use ratatui::{layout::Rect, style::Color, Frame};

use crate::tui_v2::{
    model::capability::{ColorDepth, GlyphMode},
    motion::clock::MotionClock,
    presentation::theme::SemanticTheme,
};

// Dark → light purple only (theme family; light end stays pigmented).
const PURPLE_DIM: (u8, u8, u8) = (0x2a, 0x22, 0x2f);
const PURPLE_DEEP: (u8, u8, u8) = (0x4a, 0x38, 0x55);
const PURPLE_MID: (u8, u8, u8) = (0x75, 0x61, 0x7e);
const PURPLE_LIT: (u8, u8, u8) = (0x9a, 0x82, 0xa5);

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    let t = t.clamp(0.0, 1.0);
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> Color {
    Color::Rgb(
        lerp_u8(a.0, b.0, t),
        lerp_u8(a.1, b.1, t),
        lerp_u8(a.2, b.2, t),
    )
}

/// Continuous dark→light→dark purple loop for the scrolling edge.
fn purple_at(t: f32) -> Color {
    let stops: &[(f32, (u8, u8, u8))] = &[
        (0.00, PURPLE_DIM),
        (0.18, PURPLE_DEEP),
        (0.36, PURPLE_MID),
        (0.50, PURPLE_LIT),
        (0.64, PURPLE_MID),
        (0.82, PURPLE_DEEP),
        (1.00, PURPLE_DIM),
    ];
    let u = t.rem_euclid(1.0);
    for w in stops.windows(2) {
        let (t0, c0) = w[0];
        let (t1, c1) = w[1];
        if u >= t0 && u <= t1 {
            let local = if (t1 - t0).abs() < f32::EPSILON {
                0.0
            } else {
                (u - t0) / (t1 - t0)
            };
            return lerp_rgb(c0, c1, local);
        }
    }
    Color::Rgb(PURPLE_MID.0, PURPLE_MID.1, PURPLE_MID.2)
}

fn approx_purple(depth: ColorDepth, t: f32) -> Color {
    match depth {
        ColorDepth::TrueColor => purple_at(t),
        ColorDepth::Ansi256 => {
            // Magenta/purple band indices; sample by intensity.
            let u = t.rem_euclid(1.0);
            let idx = if u < 0.25 {
                53u8 // deep purple
            } else if u < 0.5 {
                96
            } else if u < 0.75 {
                139
            } else {
                96
            };
            Color::Indexed(idx)
        }
        ColorDepth::Ansi16 | ColorDepth::Monochrome => Color::Magenta,
    }
}

/// Paint the bottom working edge when the agent is running.
pub fn render_working_edge(
    frame: &mut Frame,
    area: Rect,
    clock: MotionClock,
    theme: SemanticTheme,
    glyph_mode: GlyphMode,
    color_depth: ColorDepth,
) {
    if area.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    let ch = match glyph_mode {
        GlyphMode::Unicode => '━',
        GlyphMode::Ascii => '=',
    };
    let w = area.width as usize;
    // Phase advances so the bright head travels left → right.
    let phase = (clock.elapsed_ms() as f32 * 0.0005).rem_euclid(1.0);
    let buf = frame.buffer_mut();
    let y = area.y;
    for x in 0..w {
        let xf = x as f32 / (w.saturating_sub(1).max(1) as f32);
        let color = approx_purple(color_depth, xf - phase);
        if let Some(cell) = buf.cell_mut((area.x + x as u16, y)) {
            cell.set_char(ch);
            cell.set_fg(color);
            cell.set_bg(theme.canvas);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purple_ramp_stays_in_bounds() {
        for i in 0..20 {
            let c = purple_at(i as f32 * 0.05);
            match c {
                Color::Rgb(r, g, b) => {
                    // Never near white.
                    assert!(r < 200 && g < 200 && b < 220, "too light {r},{g},{b}");
                }
                _ => panic!("expected rgb"),
            }
        }
    }
}
