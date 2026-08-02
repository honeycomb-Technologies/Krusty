//! Ratatui test harness.

mod buffer;
mod harness;
mod layout;
#[cfg(test)]
mod stress;

pub use buffer::BufferSnapshot;
pub use harness::RenderHarness;
pub use layout::serialize_layout;

#[cfg(test)]
mod tests {
    use mitsuro_core::agent::LoopEvent;
    use serde_json::json;

    use crate::tui_v2::{
        app::route::{AppRoute, SessionId},
        model::{
            capability::{CapabilityProfile, ColorDepth, GlyphMode},
            conversation::TimelinePart,
        },
        motion::preference::MotionPreference,
        presentation::theme::ThemeKind,
        projection::ConversationProjection,
    };

    use super::*;

    #[test]
    fn minimum_preview_has_exact_dimensions_and_stable_regions() {
        let capability = CapabilityProfile {
            glyph_mode: GlyphMode::Unicode,
            color_depth: ColorDepth::TrueColor,
        };
        let mut state = crate::tui_v2::app::state::UiState::preview(capability);
        // Settle identity so the wordmark is fully present in the golden.
        state.appearance.motion.preference = MotionPreference::Off;
        let rendered = RenderHarness::new(50, 16).draw(&state);
        let snapshot = rendered.buffer;

        assert_eq!(snapshot.width, 50);
        assert_eq!(snapshot.height, 16);
        snapshot.assert_contains("┌┬┐");
        snapshot.assert_contains("Ask Agent");
        snapshot.assert_contains("build");
        snapshot.assert_contains("Ctrl+Q quit");
        assert!(
            !snapshot.text().contains("recent conversations"),
            "home must not list recent sessions"
        );
        assert!(
            !snapshot.text().contains('◦') && !snapshot.text().contains('•'),
            "drift fireflies must be removed"
        );
        assert!(
            !snapshot.text().contains("_ __ ___"),
            "home must not use the old figlet wordmark"
        );
        assert_eq!(snapshot.cells.len(), 50 * 16);
        assert_eq!(
            snapshot.cell(0, 0).background,
            ratatui::style::Color::Rgb(0x0e, 0x0e, 0x11)
        );
        assert_eq!(snapshot.cursor, None);
        snapshot.assert_text_eq(include_str!("goldens/50x16-preview.txt"));
        assert_eq!(
            serialize_layout(&rendered.layout),
            include_str!("goldens/50x16-layout.txt").trim_end()
        );
    }

    #[test]
    fn too_small_preview_is_a_dedicated_ascii_safe_state() {
        let rendered = RenderHarness::new(49, 15).render_with_layout(CapabilityProfile {
            glyph_mode: GlyphMode::Ascii,
            color_depth: ColorDepth::Monochrome,
        });
        let snapshot = rendered.buffer;

        snapshot.assert_contains("Mitsuro needs at least 50x16");
        snapshot.assert_contains("current: 49x15");
        assert!(snapshot.text().is_ascii());
        snapshot.assert_text_eq(include_str!("goldens/49x15-too-small.txt"));
        assert_eq!(
            serialize_layout(&rendered.layout),
            include_str!("goldens/49x15-layout.txt").trim_end()
        );
    }

    #[test]
    fn viewport_and_appearance_matrix_has_exact_whole_buffer_fingerprints() {
        let viewport_profile = CapabilityProfile {
            glyph_mode: GlyphMode::Unicode,
            color_depth: ColorDepth::TrueColor,
        };
        let viewport_actual = [(50, 16), (80, 24), (120, 36), (160, 48)]
            .into_iter()
            .map(|(width, height)| {
                let mut state = crate::tui_v2::app::state::UiState::preview(viewport_profile);
                state.appearance.motion.preference = MotionPreference::Off;
                let frame = RenderHarness::new(width, height).draw(&state);
                (
                    format!("{width}x{height}"),
                    frame.buffer.stable_fingerprint(),
                )
            })
            .collect::<Vec<_>>();

        let appearance_actual = [
            (
                "dark-full",
                CapabilityProfile {
                    glyph_mode: GlyphMode::Unicode,
                    color_depth: ColorDepth::TrueColor,
                },
                ThemeKind::MitsuroDark,
                MotionPreference::Full,
            ),
            (
                "light-reduced",
                CapabilityProfile {
                    glyph_mode: GlyphMode::Unicode,
                    color_depth: ColorDepth::TrueColor,
                },
                ThemeKind::MitsuroLight,
                MotionPreference::Reduced,
            ),
            (
                "adaptive-ansi16",
                CapabilityProfile {
                    glyph_mode: GlyphMode::Unicode,
                    color_depth: ColorDepth::Ansi16,
                },
                ThemeKind::TerminalAdaptive,
                MotionPreference::Off,
            ),
            (
                "high-contrast",
                CapabilityProfile {
                    glyph_mode: GlyphMode::Unicode,
                    color_depth: ColorDepth::TrueColor,
                },
                ThemeKind::HighContrast,
                MotionPreference::Off,
            ),
            (
                "monochrome-ascii",
                CapabilityProfile {
                    glyph_mode: GlyphMode::Ascii,
                    color_depth: ColorDepth::Monochrome,
                },
                ThemeKind::TerminalAdaptive,
                MotionPreference::Off,
            ),
        ]
        .into_iter()
        .map(|(label, capability, theme, motion)| {
            let mut state = crate::tui_v2::app::state::UiState::preview(capability);
            state.appearance.theme = theme;
            state.appearance.motion.preference = motion;
            state.appearance.motion.clock.advance_to(720);
            let frame = RenderHarness::new(80, 24).draw(&state);
            if capability.glyph_mode == GlyphMode::Ascii {
                assert!(
                    frame.buffer.text().is_ascii(),
                    "ASCII fallback leaked a Unicode glyph:\n{}",
                    frame.buffer.text()
                );
            }
            (label.to_owned(), frame.buffer.stable_fingerprint())
        })
        .collect::<Vec<_>>();

        assert_eq!(
            viewport_actual,
            [
                ("50x16", "d9f085e32e43f428"),
                ("80x24", "9cd77d4bdacafb27"),
                ("120x36", "ef82ad8710044f80"),
                ("160x48", "2c202895f167a1fd"),

            ]
            .into_iter()
            .map(|(label, fingerprint)| (label.to_owned(), fingerprint.to_owned()))
            .collect::<Vec<_>>()
        );
        assert_eq!(
            appearance_actual,
            [
                ("dark-full", "fb120c2c707def39"),
                ("light-reduced", "a1373e8ce26cc3d5"),
                ("adaptive-ansi16", "7c199a51e0cad562"),
                ("high-contrast", "6cf6e9fd47b2aeb0"),
                ("monochrome-ascii", "e0d079e144085df4"),
            ]
            .into_iter()
            .map(|(label, fingerprint)| (label.to_owned(), fingerprint.to_owned()))
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn overlay_open_and_close_leave_no_stale_cells() {
        use crate::tui_v2::{
            app::reducer::{reduce, UiAction},
            model::overlay::OverlayKind,
        };

        let capability = CapabilityProfile {
            glyph_mode: GlyphMode::Unicode,
            color_depth: ColorDepth::TrueColor,
        };
        let mut state = crate::tui_v2::app::state::UiState::preview(capability);
        let mut harness = RenderHarness::new(80, 24);
        let base = harness.draw(&state);
        reduce(
            &mut state,
            UiAction::OverlayOpened(OverlayKind::CommandPalette),
        );
        let open = harness.draw(&state);
        let overlay_rect = open
            .layout
            .region(crate::tui_v2::layout::snapshot::LayoutRegionId::Overlay)
            .expect("overlay rectangle");
        let overlay_id = state.overlay.as_ref().expect("overlay state").id;
        reduce(&mut state, UiAction::OverlayClosed(overlay_id));
        let closed = harness.draw(&state);

        assert!(!base.buffer.text().contains("type a command"));
        open.buffer.assert_contains("type a command");
        assert!(!closed.buffer.text().contains("type a command"));
        assert_eq!(closed.invalidation.clear_regions, vec![overlay_rect]);
    }

    #[test]
    fn every_overlay_family_restores_the_exact_underlying_buffer() {
        use crate::tui_v2::{
            app::reducer::{reduce, UiAction},
            model::{artifact::PartId, overlay::OverlayKind},
        };

        let overlays = vec![
            OverlayKind::CommandPalette,
            OverlayKind::Help,
            OverlayKind::SessionPicker,
            OverlayKind::ModelPicker,
            OverlayKind::Connections,
            OverlayKind::ThemeAppearance,
            OverlayKind::PlanGoal,
            OverlayKind::Processes,
            OverlayKind::ExtensionsCenter,
            OverlayKind::FileArtifactInspector {
                part_id: PartId::from_semantic("missing-artifact"),
            },
        ];
        for capability in [
            CapabilityProfile {
                glyph_mode: GlyphMode::Unicode,
                color_depth: ColorDepth::TrueColor,
            },
            CapabilityProfile {
                glyph_mode: GlyphMode::Ascii,
                color_depth: ColorDepth::Monochrome,
            },
        ] {
            for kind in overlays.clone() {
                let mut state = crate::tui_v2::app::state::UiState::preview(capability);
                state.appearance.motion.preference = MotionPreference::Off;
                let mut harness = RenderHarness::new(80, 24);
                let base = harness.draw(&state);
                reduce(&mut state, UiAction::OverlayOpened(kind.clone()));
                let open = harness.draw(&state);
                if capability.glyph_mode == GlyphMode::Ascii {
                    assert!(
                        open.buffer.text().is_ascii(),
                        "{:?} leaked a Unicode UI glyph in ASCII mode:\n{}",
                        kind,
                        open.buffer.text()
                    );
                }
                assert_ne!(
                    open.buffer, base.buffer,
                    "{:?} did not render a distinct surface",
                    kind
                );
                let overlay_id = state.overlay.as_ref().expect("overlay state").id;
                reduce(&mut state, UiAction::OverlayClosed(overlay_id));
                let closed = harness.draw(&state);
                assert_eq!(
                    closed.buffer, base.buffer,
                    "{:?} left stale characters or styles after close",
                    kind
                );
            }
        }
    }

    #[test]
    fn tool_heavy_conversation_viewport_matrix_is_stable_and_ascii_safe() {
        let mut projection = ConversationProjection::new("matrix-session");
        projection.set_title(Some("TUI validation".to_owned()));
        projection.push_user_prompt(
            "user-1",
            "Run the focused validation and summarize failures.".to_owned(),
            Vec::new(),
            false,
        );
        for event in [
            LoopEvent::TextDelta {
                delta: "I will inspect the current state first.\n".to_owned(),
            },
            LoopEvent::ToolCallComplete {
                id: "bash-matrix".to_owned(),
                name: "bash".to_owned(),
                arguments: json!({"command": "cargo test -p mitsuro tui_v2"}),
            },
            LoopEvent::ToolExecuting {
                id: "bash-matrix".to_owned(),
                name: "bash".to_owned(),
            },
            LoopEvent::ToolOutputDelta {
                id: "bash-matrix".to_owned(),
                delta: "Compiling mitsuro\nrunning focused tests\n".to_owned(),
            },
        ] {
            projection.apply_event(event);
        }

        let cases = [
            (
                "50x16",
                50,
                16,
                CapabilityProfile {
                    glyph_mode: GlyphMode::Unicode,
                    color_depth: ColorDepth::TrueColor,
                },
            ),
            (
                "80x24",
                80,
                24,
                CapabilityProfile {
                    glyph_mode: GlyphMode::Unicode,
                    color_depth: ColorDepth::TrueColor,
                },
            ),
            (
                "120x36",
                120,
                36,
                CapabilityProfile {
                    glyph_mode: GlyphMode::Unicode,
                    color_depth: ColorDepth::TrueColor,
                },
            ),
            (
                "160x48",
                160,
                48,
                CapabilityProfile {
                    glyph_mode: GlyphMode::Unicode,
                    color_depth: ColorDepth::TrueColor,
                },
            ),
            (
                "80x24-ascii",
                80,
                24,
                CapabilityProfile {
                    glyph_mode: GlyphMode::Ascii,
                    color_depth: ColorDepth::Monochrome,
                },
            ),
        ];
        let actual = cases
            .into_iter()
            .map(|(label, width, height, capability)| {
                let mut state = crate::tui_v2::app::state::UiState::preview(capability);
                state.route = AppRoute::Conversation {
                    session_id: SessionId::from_canonical("matrix-session"),
                };
                state.appearance.motion.preference = MotionPreference::Off;
                let frame = RenderHarness::new(width, height)
                    .draw_conversation(&state, projection.presentation());
                if capability.glyph_mode == GlyphMode::Ascii {
                    assert!(
                        frame.buffer.text().is_ascii(),
                        "ASCII conversation leaked a Unicode UI glyph:\n{}",
                        frame.buffer.text()
                    );
                }
                (label.to_owned(), frame.buffer.stable_fingerprint())
            })
            .collect::<Vec<_>>();

        assert_eq!(
            actual,
            [
                ("50x16", "e2220e0c8c331829"),
                ("80x24", "91cd62e2c9b40ae6"),
                ("120x36", "34abe546ba819097"),
                ("160x48", "acc933c6d0522100"),
                ("80x24-ascii", "a1edc0fcd1db2c6e"),
            ]
            .into_iter()
            .map(|(label, fingerprint)| (label.to_owned(), fingerprint.to_owned()))
            .collect::<Vec<_>>()
        );
    }

    #[test]
    fn conversation_tool_output_is_compact_expandable_and_artifact_clean() {
        let capability = CapabilityProfile {
            glyph_mode: GlyphMode::Unicode,
            color_depth: ColorDepth::TrueColor,
        };
        let mut state = crate::tui_v2::app::state::UiState::preview(capability);
        state.route = AppRoute::Conversation {
            session_id: SessionId::from_canonical("session"),
        };
        let mut projection = ConversationProjection::new("session");
        projection.push_user_prompt("u1", "Run the focused tests.".to_owned(), Vec::new(), false);
        for event in [
            LoopEvent::ToolCallComplete {
                id: "bash-1".to_owned(),
                name: "bash".to_owned(),
                arguments: json!({"command": "cargo test -p mitsuro tui_v2"}),
            },
            LoopEvent::ToolExecuting {
                id: "bash-1".to_owned(),
                name: "bash".to_owned(),
            },
            LoopEvent::ToolOutputDelta {
                id: "bash-1".to_owned(),
                delta: "Compiling mitsuro\n56 tests passed\n".to_owned(),
            },
        ] {
            projection.apply_event(event);
        }
        let tool_id = projection.presentation().turns[0]
            .parts
            .iter()
            .find_map(|part| match part {
                TimelinePart::Tool(tool) => Some(tool.id.clone()),
                _ => None,
            })
            .expect("tool part");
        let mut harness = RenderHarness::new(80, 24);

        let collapsed = harness.draw_conversation(&state, projection.presentation());
        let collapsed_text = collapsed.buffer.text();
        assert!(!collapsed_text.lines().any(|line| line.trim() == "you"));
        let prompt = "Run the focused tests.";
        let prompt_line = collapsed_text
            .lines()
            .find(|line| line.contains(prompt))
            .expect("right-aligned user bubble");
        let prompt_start = prompt_line.find(prompt).expect("prompt column");
        assert!(
            prompt_line.contains('│') || prompt_line.contains('|'),
            "user bubble should use a framed border:\n{prompt_line:?}"
        );
        assert!(
            prompt_start > usize::from(collapsed.buffer.width) / 2,
            "user bubble should sit past center, got column {prompt_start}"
        );
        let collapsed_tool = collapsed
            .layout
            .transcript
            .parts
            .iter()
            .find(|part| part.part_id == tool_id)
            .expect("collapsed tool layout");
        assert_eq!(collapsed_tool.full_height, 1);
        collapsed.buffer.assert_contains("Bash");
        collapsed
            .buffer
            .assert_contains("cargo test -p mitsuro tui_v2");
        assert!(!collapsed.buffer.text().contains("56 tests passed"));

        state
            .artifacts
            .entry(tool_id.clone())
            .or_default()
            .toggle_expanded();
        let expanded = harness.draw_conversation(&state, projection.presentation());
        expanded.buffer.assert_contains("56 tests passed");
        assert!(expanded
            .layout
            .transcript
            .parts
            .iter()
            .find(|part| part.part_id == tool_id)
            .is_some_and(|part| part.full_height > 1));

        state
            .artifacts
            .entry(tool_id)
            .or_default()
            .toggle_expanded();
        let recollapsed = harness.draw_conversation(&state, projection.presentation());
        assert!(!recollapsed.buffer.text().contains("56 tests passed"));

        projection.apply_event(LoopEvent::ToolApprovalRequired {
            id: "write-1".to_owned(),
            name: "write".to_owned(),
            arguments: json!({"path": "src/main.rs"}),
        });
        state.focus = crate::tui_v2::model::focus::FocusTarget::DecisionDock;
        let approval = harness.draw_conversation(&state, projection.presentation());
        assert!(approval
            .layout
            .region(crate::tui_v2::layout::snapshot::LayoutRegionId::DecisionDock)
            .is_some());
        approval.buffer.assert_contains("Approval required");
        approval.buffer.assert_contains("A approve");

        state.capability = CapabilityProfile {
            glyph_mode: GlyphMode::Ascii,
            color_depth: ColorDepth::Monochrome,
        };
        let ascii_approval = harness.draw_conversation(&state, projection.presentation());
        assert!(
            ascii_approval.buffer.text().is_ascii(),
            "decision dock leaked a Unicode UI glyph in ASCII mode:\n{}",
            ascii_approval.buffer.text()
        );
    }

    #[test]
    fn scrolling_every_transcript_offset_does_not_panic() {
        let mut projection = ConversationProjection::new("scroll-session");
        projection.set_title(Some("Scroll stress".to_owned()));
        for i in 0..12 {
            projection.push_user_prompt(
                &format!("user-{i}"),
                format!("Prompt number {i} with enough text to wrap across multiple bubble rows when the column is narrow."),
                Vec::new(),
                false,
            );
            // Wide unwrapped-looking prose + markdown so clip rows exercise
            // agent markdown indexing at every partial-clip offset.
            let long = "word ".repeat(80);
            projection.apply_event(LoopEvent::TextDelta {
                delta: format!(
                    "Agent reply {i}.\n\n# Heading {i}\n\n{long}\n\n- item one\n- item two\n\n```rust\nfn main() {{ println!(\"{i}\"); }}\n```\n\nMore prose after the fence so markdown has many measured lines.\n\nhttps://example.com/doc/{i}\n"
                ),
            });
            projection.apply_event(LoopEvent::ToolCallComplete {
                id: format!("bash-{i}"),
                name: "bash".to_owned(),
                arguments: json!({"command": format!("echo {i}")}),
            });
            projection.apply_event(LoopEvent::ToolExecuting {
                id: format!("bash-{i}"),
                name: "bash".to_owned(),
            });
            projection.apply_event(LoopEvent::ToolOutputDelta {
                id: format!("bash-{i}"),
                delta: format!("line {i}\noutput more\n"),
            });
        }

        let presentation = projection.presentation();
        for (width, height) in [(50, 16), (80, 24), (120, 36)] {
            let capability = CapabilityProfile {
                glyph_mode: GlyphMode::Unicode,
                color_depth: ColorDepth::TrueColor,
            };
            let mut state = crate::tui_v2::app::state::UiState::preview(capability);
            state.route = AppRoute::Conversation {
                session_id: SessionId::from_canonical("scroll-session"),
            };
            state.appearance.motion.preference = MotionPreference::Off;

            // Discover max scroll via follow-live frame then walk every offset.
            state.transcript.follow_live = true;
            let live = RenderHarness::new(width, height).draw_conversation(&state, &presentation);
            let max = live.layout.transcript.total_height.saturating_sub(u32::from(
                live.layout.transcript.viewport.height,
            ));
            state.transcript.follow_live = false;
            for offset in 0..=max {
                state.transcript.scroll_rows = offset;
                let _ = RenderHarness::new(width, height).draw_conversation(&state, &presentation);
            }
        }
    }
}
