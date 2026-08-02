use std::time::Duration;

use crate::agent::loop_events::LoopStopReason;

use super::TickEngineConfig;

pub(super) enum PostTurnAction {
    Finish(LoopStopReason),
    Sleep(Duration),
    Continue { tick_number: usize, delay: Duration },
}

pub(super) fn determine_post_turn_action(
    tick_config: &TickEngineConfig,
    stop_reason: LoopStopReason,
    last_tool_output: Option<&str>,
    tick_count: usize,
) -> PostTurnAction {
    if !tick_config.enabled || stop_reason != LoopStopReason::Completed {
        return PostTurnAction::Finish(stop_reason);
    }

    if let Some(duration) = parse_sleep_signal(last_tool_output) {
        return PostTurnAction::Sleep(duration);
    }

    let next_tick_number = tick_count.saturating_add(1);
    // `max_ticks` is the maximum number of inner orchestrator runs, including
    // the initial run whose zero-based index is `tick_count == 0`.
    if next_tick_number >= tick_config.max_ticks {
        return PostTurnAction::Finish(LoopStopReason::BudgetExhausted);
    }

    PostTurnAction::Continue {
        tick_number: next_tick_number,
        delay: tick_config.tick_interval,
    }
}

fn parse_sleep_signal(output: Option<&str>) -> Option<Duration> {
    let output = output?;
    if !output.contains("\"signal\"") || !output.contains("\"sleep_idle\"") {
        return None;
    }

    let parsed: serde_json::Value = serde_json::from_str(output).ok()?;
    let payload = parsed.get("data").unwrap_or(&parsed);
    if payload.get("signal")?.as_str()? != "sleep_idle" {
        return None;
    }

    let secs = payload
        .get("duration_secs")
        .and_then(|value| value.as_u64())
        .or_else(|| {
            payload
                .get("slept_seconds")
                .and_then(|value| value.as_u64())
        })
        .unwrap_or(60);
    Some(Duration::from_secs(secs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sleep_signal_with_valid_json() {
        let output = r#"{"signal": "sleep_idle", "duration_secs": 120}"#;
        let duration = parse_sleep_signal(Some(output));
        assert_eq!(duration, Some(Duration::from_secs(120)));
    }

    #[test]
    fn parse_sleep_signal_defaults_to_60s_without_duration() {
        let output = r#"{"signal": "sleep_idle"}"#;
        let duration = parse_sleep_signal(Some(output));
        assert_eq!(duration, Some(Duration::from_secs(60)));
    }

    #[test]
    fn parse_sleep_signal_reads_tool_result_envelope() {
        let output = r#"{"ok":true,"data":{"slept_seconds":120,"signal":"sleep_idle","reason":"waiting for CI"}}"#;
        let duration = parse_sleep_signal(Some(output));
        assert_eq!(duration, Some(Duration::from_secs(120)));
    }

    #[test]
    fn parse_sleep_signal_returns_none_for_other_signals() {
        let output = r#"{"signal": "other_signal"}"#;
        assert!(parse_sleep_signal(Some(output)).is_none());
    }

    #[test]
    fn parse_sleep_signal_returns_none_for_plain_text() {
        assert!(parse_sleep_signal(Some("just some output")).is_none());
    }

    #[test]
    fn parse_sleep_signal_returns_none_for_none() {
        assert!(parse_sleep_signal(None).is_none());
    }

    #[test]
    fn completed_turn_continues_with_next_tick_when_enabled() {
        let action = determine_post_turn_action(
            &TickEngineConfig {
                tick_interval: Duration::from_secs(15),
                max_ticks: 4,
                enabled: true,
            },
            LoopStopReason::Completed,
            None,
            0,
        );

        match action {
            PostTurnAction::Continue { tick_number, delay } => {
                assert_eq!(tick_number, 1);
                assert_eq!(delay, Duration::from_secs(15));
            }
            _ => panic!("expected tick continuation"),
        }
    }

    #[test]
    fn completed_turn_sleeps_when_tool_requests_idle() {
        let action = determine_post_turn_action(
            &TickEngineConfig {
                tick_interval: Duration::from_secs(30),
                max_ticks: 4,
                enabled: true,
            },
            LoopStopReason::Completed,
            Some(r#"{"signal":"sleep_idle","duration_secs":90}"#),
            0,
        );

        match action {
            PostTurnAction::Sleep(duration) => {
                assert_eq!(duration, Duration::from_secs(90));
            }
            _ => panic!("expected sleep action"),
        }
    }

    #[test]
    fn completed_turn_stops_when_tick_budget_is_exhausted() {
        let action = determine_post_turn_action(
            &TickEngineConfig {
                tick_interval: Duration::from_secs(30),
                max_ticks: 1,
                enabled: true,
            },
            LoopStopReason::Completed,
            None,
            0,
        );

        match action {
            PostTurnAction::Finish(reason) => {
                assert_eq!(reason, LoopStopReason::BudgetExhausted);
            }
            _ => panic!("expected finish action"),
        }
    }

    #[test]
    fn max_ticks_counts_the_initial_inner_run() {
        let action = determine_post_turn_action(
            &TickEngineConfig {
                tick_interval: Duration::from_secs(30),
                max_ticks: 4,
                enabled: true,
            },
            LoopStopReason::Completed,
            None,
            3,
        );

        assert!(matches!(
            action,
            PostTurnAction::Finish(LoopStopReason::BudgetExhausted)
        ));
    }

    #[test]
    fn non_completed_turn_finishes_without_tick_continuation() {
        let action = determine_post_turn_action(
            &TickEngineConfig {
                tick_interval: Duration::from_secs(30),
                max_ticks: 4,
                enabled: true,
            },
            LoopStopReason::AwaitingInput,
            None,
            0,
        );

        match action {
            PostTurnAction::Finish(reason) => {
                assert_eq!(reason, LoopStopReason::AwaitingInput);
            }
            _ => panic!("expected finish action"),
        }
    }
}
