//! Long-session and release-mode response-budget proofs.

use std::time::{Duration, Instant};

use mitsuro_core::agent::LoopEvent;
use serde_json::json;

use crate::tui_v2::{
    app::{
        route::{AppRoute, SessionId},
        state::UiState,
    },
    model::{
        artifact::{ArtifactContent, RetentionLevel},
        capability::{CapabilityProfile, ColorDepth, GlyphMode},
        conversation::TimelinePart,
    },
    motion::preference::MotionPreference,
    projection::ConversationProjection,
};

use super::RenderHarness;

const TURN_COUNT: usize = 500;
const TOOLS_PER_TURN: usize = 4;

fn capability() -> CapabilityProfile {
    CapabilityProfile {
        glyph_mode: GlyphMode::Unicode,
        color_depth: ColorDepth::TrueColor,
    }
}

fn long_session() -> ConversationProjection {
    let mut projection = ConversationProjection::new("stress-session");
    projection.set_title(Some("Long-session validation".to_owned()));
    let large_output = format!("{}\n", "0123456789abcdef".repeat(4_096));

    for turn in 0..TURN_COUNT {
        projection.push_user_prompt(
            &format!("user-{turn}"),
            format!("Validate deterministic behavior for turn {turn}."),
            Vec::new(),
            false,
        );
        projection.apply_event(LoopEvent::TextDelta {
            delta: format!("Planning turn {turn}. "),
        });

        for tool_index in 0..TOOLS_PER_TURN {
            let id = format!("tool-{turn}-{tool_index}");
            let name = if tool_index == 0 && turn < 50 {
                "apply_patch"
            } else if tool_index == 1 {
                "bash"
            } else {
                "read"
            };
            projection.apply_event(LoopEvent::ToolCallComplete {
                id: id.clone(),
                name: name.to_owned(),
                arguments: json!({
                    "command": format!("validate --turn {turn} --tool {tool_index}"),
                    "path": format!("src/fixture_{turn}.rs"),
                }),
            });
            projection.apply_event(LoopEvent::ToolExecuting {
                id: id.clone(),
                name: name.to_owned(),
            });
            projection.apply_event(LoopEvent::ToolOutputDelta {
                id: id.clone(),
                delta: format!("progress {turn}:{tool_index}\rcomplete {turn}:{tool_index}\n"),
            });
            projection.apply_event(LoopEvent::ToolResult {
                id,
                output: if name == "bash" && turn < 100 {
                    large_output.clone()
                } else {
                    format!("completed turn {turn} tool {tool_index}")
                },
                is_error: false,
            });
            projection.apply_event(LoopEvent::TextDelta {
                delta: format!("Tool {tool_index} complete. "),
            });
        }
        projection.apply_event(LoopEvent::TextDelta {
            delta: format!("Turn {turn} complete."),
        });
        projection.apply_event(LoopEvent::TurnComplete {
            turn,
            has_more: false,
        });
    }

    projection
}

fn conversation_state() -> UiState {
    let mut state = UiState::preview(capability());
    state.route = AppRoute::Conversation {
        session_id: SessionId::from_canonical("stress-session"),
    };
    state.appearance.motion.preference = MotionPreference::Off;
    state
}

#[test]
fn five_hundred_turns_and_two_thousand_tools_remain_bounded_and_stable() {
    let projection = long_session();
    let presentation = projection.presentation();
    assert_eq!(presentation.turns.len(), TURN_COUNT);
    let tool_count = presentation
        .turns
        .iter()
        .flat_map(|turn| &turn.parts)
        .filter(|part| matches!(part, TimelinePart::Tool(_)))
        .count();
    assert_eq!(tool_count, TURN_COUNT * TOOLS_PER_TURN);

    let oldest_bash = presentation.turns[0]
        .parts
        .iter()
        .find_map(|part| match part {
            TimelinePart::Tool(tool) if tool.name == "bash" => Some(tool),
            _ => None,
        })
        .expect("expected the oldest Bash row");
    assert_eq!(oldest_bash.artifact.retention, RetentionLevel::Summary);
    assert!(matches!(
        &oldest_bash.artifact.content,
        ArtifactContent::Text(text) if text.text.len() <= 2 * 1_024 && text.omitted_bytes > 0
    ));

    let state = conversation_state();
    let mut harness = RenderHarness::new(120, 36);
    let cold = harness.draw_conversation(&state, presentation);
    cold.buffer.assert_contains("Turn 499 complete.");
    assert!(!cold.layout.transcript.parts.is_empty());
    assert!(cold.layout.transcript.parts.len() <= 36);
    let stats = harness.measurement_cache_stats();
    assert!(stats.entries <= stats.max_entries);
    assert!(stats.weight <= stats.max_weight);
    assert!(
        stats.entries >= 5_000,
        "the warm set should cover the fixture"
    );

    let warm = harness.draw_conversation(&state, presentation);
    assert_eq!(warm.buffer, cold.buffer);
    assert_eq!(harness.measurement_cache_stats(), stats);

    for (width, height) in [(50, 16), (160, 48), (80, 24), (120, 36)] {
        harness.resize(width, height);
        let resized = harness.draw_conversation(&state, presentation);
        assert_eq!(resized.buffer.width, width);
        assert_eq!(resized.buffer.height, height);
        assert!(resized.layout.validate().is_ok());
        assert!(resized.layout.transcript.at_live_edge);
    }
}

#[test]
fn release_frames_meet_the_approved_long_session_budgets() {
    if cfg!(debug_assertions) {
        return;
    }

    let mut projection = long_session();
    let state = conversation_state();
    let mut p95_harness = RenderHarness::new(120, 36);
    let mut p99_harness = RenderHarness::new(160, 48);
    p95_harness.draw_conversation(&state, projection.presentation());
    p99_harness.draw_conversation(&state, projection.presentation());

    let p95_samples = frame_samples(&mut p95_harness, &state, &projection, 40);
    let p99_samples = frame_samples(&mut p99_harness, &state, &projection, 100);
    let p95 = percentile(&p95_samples, 95);
    let p99 = percentile(&p99_samples, 99);
    eprintln!("long-session frame budgets: 120x36 p95={p95:?}, 160x48 p99={p99:?}");

    assert!(
        p95 < Duration::from_millis(8),
        "120x36 p95 {:?} exceeded the 8 ms budget",
        p95
    );
    assert!(
        p99 < Duration::from_millis(16),
        "160x48 p99 {:?} exceeded the 16 ms budget",
        p99
    );

    let response_start = Instant::now();
    projection.push_user_prompt(
        "visible-response",
        "Show this response immediately.".to_owned(),
        Vec::new(),
        false,
    );
    projection.apply_event(LoopEvent::TextDelta {
        delta: "Visible under streaming pressure.".to_owned(),
    });
    let response = p95_harness.draw_conversation(&state, projection.presentation());
    let input_to_visible = response_start.elapsed();
    eprintln!("long-session input-to-visible={input_to_visible:?}");
    response
        .buffer
        .assert_contains("Visible under streaming pressure.");
    assert!(!response.invalidation.full);
    assert!(
        input_to_visible < Duration::from_millis(50),
        "input-to-visible {:?} exceeded the 50 ms budget",
        input_to_visible
    );
}

fn frame_samples(
    harness: &mut RenderHarness,
    state: &UiState,
    projection: &ConversationProjection,
    count: usize,
) -> Vec<Duration> {
    (0..count)
        .map(|_| {
            let start = Instant::now();
            let _ = harness.draw_conversation(state, projection.presentation());
            start.elapsed()
        })
        .collect()
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = percentile
        .saturating_mul(sorted.len())
        .div_ceil(100)
        .saturating_sub(1)
        .min(sorted.len().saturating_sub(1));
    sorted[rank]
}
