//! Home — box-drawing mitsuro + drift fireflies.

use ratatui::{layout::Rect, Frame};

use crate::tui_v2::{
    app::state::UiState, components::splash, presentation::theme::SemanticTheme,
    services::HomeSnapshot,
};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &UiState,
    _home: Option<&HomeSnapshot>,
    theme: SemanticTheme,
) {
    if area.is_empty() {
        return;
    }

    splash::render(
        frame,
        area,
        state.splash,
        state.appearance.motion.clock.elapsed_ms(),
        state.appearance.motion.preference,
        state.capability.glyph_mode,
        theme,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use crate::tui_v2::{
        app::state::UiState,
        model::capability::{CapabilityProfile, ColorDepth, GlyphMode},
        motion::preference::MotionPreference,
        presentation::theme::{SemanticTheme, ThemeKind},
        services::HomeSnapshot,
    };

    use super::*;

    #[test]
    fn home_renders_box_wordmark_without_pan_rail() {
        let capability = CapabilityProfile {
            glyph_mode: GlyphMode::Unicode,
            color_depth: ColorDepth::TrueColor,
        };
        let mut state = UiState::preview(capability);
        state.appearance.motion.preference = MotionPreference::Off;
        state.splash.settled = true;
        let theme = SemanticTheme::resolve(ThemeKind::MitsuroDark, capability.color_depth);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, frame.area(), &state, None, theme))
            .expect("home");
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains('┌') || text.contains('┬') || text.contains('┴'));
        assert!(!text.contains('◦') && !text.contains('•'));
        assert!(!text.contains("[ ] pan"));
        assert!(!text.contains('█'));
    }

    #[test]
    fn home_omits_recent_sessions() {
        let capability = CapabilityProfile {
            glyph_mode: GlyphMode::Unicode,
            color_depth: ColorDepth::TrueColor,
        };
        let mut state = UiState::preview(capability);
        state.appearance.motion.preference = MotionPreference::Off;
        state.splash.settled = true;
        let theme = SemanticTheme::resolve(ThemeKind::MitsuroDark, capability.color_depth);
        let home = HomeSnapshot {
            project: "workspace".to_owned(),
            branch: Some("main".to_owned()),
            model: Some("grok-4.5".to_owned()),
            provider: "xAI".to_owned(),
            recent_sessions: vec![crate::tui_v2::services::RecentSession {
                session_id: "s1".to_owned(),
                title: "should not appear".to_owned(),
                model: Some("grok-4.5".to_owned()),
            }],
        };
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal
            .draw(|frame| render(frame, Rect::new(0, 0, 80, 18), &state, Some(&home), theme))
            .expect("home");
        let text: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains("recent conversations"));
        assert!(!text.contains("should not appear"));
    }
}
