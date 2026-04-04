use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::client::CallOptions;
use crate::ai::types::ModelMessage;

use super::loop_events::{LoopEvent, LoopInput, LoopStopReason};
use super::orchestrator::{AgenticOrchestrator, OrchestratorConfig, OrchestratorServices};

pub struct TickEngineConfig {
    pub tick_interval: Duration,
    pub max_ticks: usize,
    pub enabled: bool,
}

impl Default for TickEngineConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_secs(30),
            max_ticks: 1000,
            enabled: false,
        }
    }
}

pub struct TickEngine;

impl TickEngine {
    pub fn run(
        services: OrchestratorServices,
        config: OrchestratorConfig,
        tick_config: TickEngineConfig,
        initial_conversation: Vec<ModelMessage>,
        options: CallOptions,
    ) -> (
        mpsc::UnboundedReceiver<LoopEvent>,
        mpsc::UnboundedSender<LoopInput>,
    ) {
        let (outer_tx, outer_rx) = mpsc::unbounded_channel();
        let (outer_input_tx, outer_input_rx) = mpsc::unbounded_channel();

        tokio::spawn(Self::drive(
            services,
            config,
            tick_config,
            initial_conversation,
            options,
            outer_tx,
            outer_input_rx,
        ));

        (outer_rx, outer_input_tx)
    }

    async fn drive(
        services: OrchestratorServices,
        config: OrchestratorConfig,
        tick_config: TickEngineConfig,
        initial_conversation: Vec<ModelMessage>,
        options: CallOptions,
        outer_tx: mpsc::UnboundedSender<LoopEvent>,
        mut outer_input_rx: mpsc::UnboundedReceiver<LoopInput>,
    ) {
        let session_id = config.session_id.clone();
        let (mut inner_rx, mut inner_input_tx) =
            AgenticOrchestrator::new(services, config.clone_for_tick())
                .run(initial_conversation, options);

        let mut last_tool_output: Option<String> = None;
        let stop_reason = Self::forward_events(
            &mut inner_rx,
            &mut inner_input_tx,
            &mut outer_input_rx,
            &outer_tx,
            &mut last_tool_output,
        )
        .await;

        let Some(stop_reason) = stop_reason else {
            return;
        };

        let final_stop_reason = if tick_config.enabled && stop_reason == LoopStopReason::Completed {
            if let Some(duration) = parse_sleep_signal(last_tool_output.as_deref()) {
                let _ = outer_tx.send(LoopEvent::AgentSleeping {
                    duration_secs: duration.as_secs(),
                    reason: "sleep_idle signal from tool".into(),
                });
                LoopStopReason::Sleeping
            } else {
                stop_reason
            }
        } else {
            stop_reason
        };

        let _ = outer_tx.send(LoopEvent::Finished {
            session_id,
            stop_reason: final_stop_reason,
        });
    }

    async fn forward_events(
        inner_rx: &mut mpsc::UnboundedReceiver<LoopEvent>,
        inner_input_tx: &mut mpsc::UnboundedSender<LoopInput>,
        outer_input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
        outer_tx: &mpsc::UnboundedSender<LoopEvent>,
        last_tool_output: &mut Option<String>,
    ) -> Option<LoopStopReason> {
        loop {
            tokio::select! {
                event = inner_rx.recv() => {
                    let Some(event) = event else { break };

                    if let LoopEvent::ToolResult { ref output, .. } = event {
                        *last_tool_output = Some(output.clone());
                    }

                    if let LoopEvent::Finished { stop_reason, .. } = event {
                        return Some(stop_reason);
                    }

                    if outer_tx.send(event).is_err() {
                        break;
                    }
                }
                input = outer_input_rx.recv() => {
                    let Some(input) = input else { break };

                    if matches!(input, LoopInput::Cancel) {
                        let _ = inner_input_tx.send(LoopInput::Cancel);
                        while let Some(event) = inner_rx.recv().await {
                            if let LoopEvent::ToolResult { ref output, .. } = event {
                                *last_tool_output = Some(output.clone());
                            }
                            if matches!(event, LoopEvent::Finished { .. }) {
                                break;
                            }
                            let _ = outer_tx.send(event);
                        }
                        return None;
                    }

                    let _ = inner_input_tx.send(input);
                }
            }
        }

        None
    }
}

fn parse_sleep_signal(output: Option<&str>) -> Option<Duration> {
    let output = output?;
    if !output.contains("\"signal\"") || !output.contains("\"sleep_idle\"") {
        return None;
    }

    let parsed: serde_json::Value = serde_json::from_str(output).ok()?;
    if parsed.get("signal")?.as_str()? != "sleep_idle" {
        return None;
    }

    let secs = parsed
        .get("duration_secs")
        .and_then(|value| value.as_u64())
        .unwrap_or(60);
    Some(Duration::from_secs(secs))
}

trait CloneForTick {
    fn clone_for_tick(&self) -> Self;
}

impl CloneForTick for OrchestratorConfig {
    fn clone_for_tick(&self) -> Self {
        Self {
            session_id: self.session_id.clone(),
            working_dir: self.working_dir.clone(),
            project_dir: self.project_dir.clone(),
            permission_mode: self.permission_mode,
            max_iterations: self.max_iterations,
            stream_idle_timeout: self.stream_idle_timeout,
            user_id: self.user_id.clone(),
            initial_work_mode: self.initial_work_mode,
            generate_title: false,
            delegated_progress_tx: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_engine_config_defaults() {
        let config = TickEngineConfig::default();
        assert_eq!(config.tick_interval, Duration::from_secs(30));
        assert_eq!(config.max_ticks, 1000);
        assert!(!config.enabled);
    }

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
}
