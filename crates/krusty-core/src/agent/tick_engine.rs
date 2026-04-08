use std::path::Path;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::ai::client::CallOptions;
use crate::ai::types::{Content, ModelMessage, Role};
use crate::storage::{Database, SessionManager};

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

enum PostTurnAction {
    Finish(LoopStopReason),
    Sleep(Duration),
    Continue { tick_number: usize, delay: Duration },
}

enum WaitOutcome {
    Continue,
    Cancelled,
    Closed,
}

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
        let mut next_conversation = Some(initial_conversation);
        let mut tick_count = 0usize;

        loop {
            let conversation = match next_conversation.take() {
                Some(conversation) => conversation,
                None => match load_tick_conversation(&services.db_path, &session_id, tick_count) {
                    Ok(conversation) => conversation,
                    Err(error) => {
                        let _ = outer_tx.send(LoopEvent::Error { error });
                        let _ = outer_tx.send(LoopEvent::Finished {
                            session_id: session_id.clone(),
                            stop_reason: LoopStopReason::ProviderError,
                        });
                        return;
                    }
                },
            };

            let (mut inner_rx, mut inner_input_tx) =
                AgenticOrchestrator::new(services.clone(), config.clone_for_tick())
                    .run(conversation, options.clone());

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

            match determine_post_turn_action(
                &tick_config,
                stop_reason,
                last_tool_output.as_deref(),
                tick_count,
            ) {
                PostTurnAction::Finish(stop_reason) => {
                    let _ = outer_tx.send(LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason,
                    });
                    return;
                }
                PostTurnAction::Sleep(duration) => {
                    let _ = outer_tx.send(LoopEvent::AgentSleeping {
                        duration_secs: duration.as_secs(),
                        reason: "sleep_idle signal from tool".into(),
                    });
                    let _ = outer_tx.send(LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason: LoopStopReason::Sleeping,
                    });
                    return;
                }
                PostTurnAction::Continue { tick_number, delay } => {
                    match Self::wait_for_next_tick(delay, &mut outer_input_rx).await {
                        WaitOutcome::Continue => {
                            tick_count = tick_number;
                            let _ = outer_tx.send(LoopEvent::TickInjected { tick_number });
                        }
                        WaitOutcome::Cancelled => {
                            let _ = outer_tx.send(LoopEvent::Finished {
                                session_id: session_id.clone(),
                                stop_reason: LoopStopReason::UserAbort,
                            });
                            return;
                        }
                        WaitOutcome::Closed => return,
                    }
                }
            }
        }
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
                        return Some(LoopStopReason::UserAbort);
                    }

                    let _ = inner_input_tx.send(input);
                }
            }
        }

        None
    }

    async fn wait_for_next_tick(
        delay: Duration,
        outer_input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
    ) -> WaitOutcome {
        let sleep = tokio::time::sleep(delay);
        tokio::pin!(sleep);

        loop {
            tokio::select! {
                _ = &mut sleep => return WaitOutcome::Continue,
                input = outer_input_rx.recv() => {
                    match input {
                        Some(LoopInput::Cancel) => return WaitOutcome::Cancelled,
                        Some(_) => continue,
                        None => return WaitOutcome::Closed,
                    }
                }
            }
        }
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

fn determine_post_turn_action(
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
    if next_tick_number > tick_config.max_ticks {
        return PostTurnAction::Finish(LoopStopReason::BudgetExhausted);
    }

    PostTurnAction::Continue {
        tick_number: next_tick_number,
        delay: tick_config.tick_interval,
    }
}

fn load_tick_conversation(
    db_path: &Path,
    session_id: &str,
    tick_number: usize,
) -> Result<Vec<ModelMessage>, String> {
    let db = Database::new(db_path)
        .map_err(|error| format!("Failed to open database for tick reload: {error}"))?;
    let session_manager = SessionManager::new(db);
    let raw_messages = session_manager
        .load_session_messages(session_id)
        .map_err(|error| format!("Failed to load session messages for tick reload: {error}"))?;

    let mut conversation = raw_messages
        .into_iter()
        .filter_map(|(role_str, content_json)| {
            let role = match role_str.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => return None,
            };

            serde_json::from_str(&content_json)
                .ok()
                .map(|content| ModelMessage { role, content })
        })
        .collect::<Vec<_>>();
    conversation.push(build_tick_message(tick_number));
    Ok(conversation)
}

fn build_tick_message(tick_number: usize) -> ModelMessage {
    ModelMessage {
        role: Role::User,
        content: vec![Content::Text {
            text: format!(
                "<tick>\nAutonomous wake #{tick_number}. Reassess the current task graph, recent progress, and whether to act, communicate, or sleep again.\n</tick>"
            ),
        }],
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
            session_type: self.session_type,
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
    use crate::storage::SessionType;
    use crate::storage::WorkspaceMode;

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
    fn parse_sleep_signal_reads_tool_result_envelope() {
        let output = r#"{"ok":true,"data":{"slept_seconds":120,"signal":"sleep_idle","reason":"waiting for CI"}}"#;
        let duration = parse_sleep_signal(Some(output));
        assert_eq!(duration, Some(Duration::from_secs(120)));
    }

    #[test]
    fn clone_for_tick_preserves_session_identity() {
        let config = OrchestratorConfig {
            session_id: "session-1".to_string(),
            working_dir: std::path::PathBuf::from("/tmp/workspace"),
            project_dir: Some(std::path::PathBuf::from("/tmp/workspace/project")),
            session_type: SessionType::Mako,
            generate_title: true,
            ..Default::default()
        };

        let cloned = config.clone_for_tick();

        assert_eq!(cloned.project_dir, config.project_dir);
        assert_eq!(cloned.session_type, SessionType::Mako);
        assert!(!cloned.generate_title);
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
            1,
        );

        match action {
            PostTurnAction::Finish(reason) => {
                assert_eq!(reason, LoopStopReason::BudgetExhausted);
            }
            _ => panic!("expected finish action"),
        }
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

    #[test]
    fn load_tick_conversation_appends_ephemeral_tick_message() {
        let temp = tempfile::TempDir::new().unwrap();
        let db_path = temp.path().join("krusty.db");
        let db = Database::new(&db_path).unwrap();
        let session_manager = SessionManager::new(db);
        let session_id = session_manager
            .create_session_for_user_with_config(
                "Tick Test",
                None,
                None,
                None,
                WorkspaceMode::Neutral,
                None,
                None,
                SessionType::Mako,
            )
            .unwrap();

        session_manager
            .save_message(
                &session_id,
                "user",
                r#"[{"type":"text","text":"Set course for auth cleanup"}]"#,
            )
            .unwrap();
        session_manager
            .save_message(
                &session_id,
                "assistant",
                r#"[{"type":"text","text":"I will coordinate that work."}]"#,
            )
            .unwrap();

        let conversation = load_tick_conversation(&db_path, &session_id, 2).unwrap();

        assert_eq!(conversation.len(), 3);
        assert!(matches!(
            &conversation[2].content[0],
            Content::Text { text }
                if text.contains("<tick>")
                    && text.contains("Autonomous wake #2")
                    && text.contains("sleep again")
        ));
    }
}
