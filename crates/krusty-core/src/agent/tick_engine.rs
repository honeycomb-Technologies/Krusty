use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};

use crate::ai::client::CallOptions;
use crate::ai::types::{Content, ModelMessage, Role};

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
        mut conversation: Vec<ModelMessage>,
        options: CallOptions,
        outer_tx: mpsc::UnboundedSender<LoopEvent>,
        mut outer_input_rx: mpsc::UnboundedReceiver<LoopInput>,
    ) {
        let shared = SharedServices::from_orchestrator_services(&services);
        let mut tick_count: usize = 0;
        let mut last_tool_output: Option<String> = None;

        // First run uses the provided services directly
        let (mut inner_rx, mut inner_input_tx) =
            AgenticOrchestrator::new(services, config.clone_for_tick())
                .run(conversation.clone(), options.clone());

        loop {
            let stop_reason = Self::forward_events(
                &mut inner_rx,
                &mut inner_input_tx,
                &mut outer_input_rx,
                &outer_tx,
                &mut last_tool_output,
            )
            .await;

            let Some(stop_reason) = stop_reason else {
                break;
            };

            match stop_reason {
                LoopStopReason::Completed
                | LoopStopReason::AwaitingInput
                | LoopStopReason::Sleeping => {}
                _ => break,
            }

            tick_count += 1;
            if tick_count >= tick_config.max_ticks {
                let _ = outer_tx.send(LoopEvent::Finished {
                    session_id: String::new(),
                    stop_reason: LoopStopReason::BudgetExhausted,
                });
                break;
            }

            let sleep_signal = parse_sleep_signal(last_tool_output.as_deref());
            let sleep_duration = sleep_signal.unwrap_or(tick_config.tick_interval);

            if let Some(duration) = sleep_signal {
                let _ = outer_tx.send(LoopEvent::AgentSleeping {
                    duration_secs: duration.as_secs(),
                    reason: "sleep_idle signal from tool".into(),
                });
            }

            tokio::select! {
                _ = tokio::time::sleep(sleep_duration) => {}
                Some(LoopInput::Cancel) = outer_input_rx.recv() => {
                    break;
                }
            }

            conversation.push(tick_message());
            let _ = outer_tx.send(LoopEvent::TickInjected {
                tick_number: tick_count,
            });
            tracing::debug!(tick = tick_count, "injecting tick message");

            let services = shared.to_orchestrator_services();
            let run_config = config.clone_for_tick();

            let (rx, tx) = AgenticOrchestrator::new(services, run_config)
                .run(conversation.clone(), options.clone());
            inner_rx = rx;
            inner_input_tx = tx;
            last_tool_output = None;
        }
    }

    async fn forward_events(
        inner_rx: &mut mpsc::UnboundedReceiver<LoopEvent>,
        inner_input_tx: &mut mpsc::UnboundedSender<LoopInput>,
        outer_input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
        outer_tx: &mpsc::UnboundedSender<LoopEvent>,
        last_tool_output: &mut Option<String>,
    ) -> Option<LoopStopReason> {
        let mut stop_reason = None;

        loop {
            tokio::select! {
                event = inner_rx.recv() => {
                    let Some(event) = event else { break };

                    if let LoopEvent::ToolResult { ref output, .. } = event {
                        *last_tool_output = Some(output.clone());
                    }

                    if let LoopEvent::Finished { stop_reason: ref reason, .. } = event {
                        stop_reason = Some(reason.clone());
                        let _ = outer_tx.send(event);
                        break;
                    }

                    if outer_tx.send(event).is_err() {
                        break;
                    }
                }
                input = outer_input_rx.recv() => {
                    let Some(input) = input else { break };

                    if matches!(input, LoopInput::Cancel) {
                        let _ = inner_input_tx.send(LoopInput::Cancel);
                        // Drain remaining events until the orchestrator finishes
                        while let Some(event) = inner_rx.recv().await {
                            let is_finished = matches!(event, LoopEvent::Finished { .. });
                            let _ = outer_tx.send(event);
                            if is_finished { break; }
                        }
                        return None;
                    }

                    let _ = inner_input_tx.send(input);
                }
            }
        }

        stop_reason
    }
}

fn tick_message() -> ModelMessage {
    ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: "<tick>".to_string(),
        }],
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

struct SharedServices {
    ai_client: Arc<crate::ai::client::AiClient>,
    tool_registry: Arc<crate::tools::registry::ToolRegistry>,
    process_registry: Arc<crate::process::ProcessRegistry>,
    db_path: std::path::PathBuf,
    skills_manager: Arc<RwLock<crate::skills::SkillsManager>>,
}

impl SharedServices {
    fn from_orchestrator_services(s: &OrchestratorServices) -> Self {
        Self {
            ai_client: Arc::clone(&s.ai_client),
            tool_registry: Arc::clone(&s.tool_registry),
            process_registry: Arc::clone(&s.process_registry),
            db_path: s.db_path.clone(),
            skills_manager: Arc::clone(&s.skills_manager),
        }
    }

    fn to_orchestrator_services(&self) -> OrchestratorServices {
        OrchestratorServices {
            ai_client: Arc::clone(&self.ai_client),
            tool_registry: Arc::clone(&self.tool_registry),
            process_registry: Arc::clone(&self.process_registry),
            db_path: self.db_path.clone(),
            skills_manager: Arc::clone(&self.skills_manager),
        }
    }
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
    fn tick_message_is_user_role_with_tick_text() {
        let msg = tick_message();
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content.len(), 1);
        match &msg.content[0] {
            Content::Text { text } => assert_eq!(text, "<tick>"),
            _ => panic!("expected Content::Text"),
        }
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
