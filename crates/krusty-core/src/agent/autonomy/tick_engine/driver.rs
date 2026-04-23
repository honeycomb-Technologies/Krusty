use std::time::Duration;

use tokio::sync::mpsc;

use crate::agent::loop_events::{LoopEvent, LoopInput, LoopStopReason};
use crate::agent::orchestrator::{AgenticOrchestrator, OrchestratorConfig, OrchestratorServices};
use crate::ai::client::CallOptions;
use crate::ai::types::ModelMessage;

use super::conversation::load_tick_conversation;
use super::policy::{determine_post_turn_action, PostTurnAction};
use super::TickEngineConfig;

enum WaitOutcome {
    Continue,
    Cancelled,
    Closed,
}

pub(super) async fn drive(
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
            AgenticOrchestrator::new(services.clone(), clone_for_tick(&config))
                .run(conversation, options.clone());

        let mut last_tool_output: Option<String> = None;
        let stop_reason = forward_events(
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
                match wait_for_next_tick(delay, &mut outer_input_rx).await {
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

fn clone_for_tick(config: &OrchestratorConfig) -> OrchestratorConfig {
    OrchestratorConfig {
        session_id: config.session_id.clone(),
        working_dir: config.working_dir.clone(),
        project_dir: config.project_dir.clone(),
        mako_crew_slug: config.mako_crew_slug.clone(),
        session_type: config.session_type,
        permission_mode: config.permission_mode,
        max_iterations: config.max_iterations,
        stream_idle_timeout: config.stream_idle_timeout,
        user_id: config.user_id.clone(),
        initial_work_mode: config.initial_work_mode,
        generate_title: false,
        delegated_progress_tx: None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::agent::orchestrator::OrchestratorConfig;
    use crate::storage::SessionType;

    use super::clone_for_tick;

    #[test]
    fn clone_for_tick_preserves_session_identity() {
        let config = OrchestratorConfig {
            session_id: "session-1".to_string(),
            working_dir: PathBuf::from("/tmp/workspace"),
            project_dir: Some(PathBuf::from("/tmp/workspace/project")),
            session_type: SessionType::Mako,
            generate_title: true,
            ..Default::default()
        };

        let cloned = clone_for_tick(&config);

        assert_eq!(cloned.project_dir, config.project_dir);
        assert_eq!(cloned.session_type, SessionType::Mako);
        assert!(!cloned.generate_title);
    }
}
