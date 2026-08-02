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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EventDelivery {
    Delivered,
    Cancelled,
    OutputClosed,
}

pub(super) async fn drive(
    services: OrchestratorServices,
    config: OrchestratorConfig,
    tick_config: TickEngineConfig,
    initial_conversation: Vec<ModelMessage>,
    options: CallOptions,
    outer_tx: mpsc::Sender<LoopEvent>,
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
                    if send_outer_event(&outer_tx, LoopEvent::Error { error }).await {
                        let _ = send_outer_event(
                            &outer_tx,
                            LoopEvent::Finished {
                                session_id: session_id.clone(),
                                stop_reason: LoopStopReason::ProviderError,
                            },
                        )
                        .await;
                    }
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
                let _ = send_outer_event(
                    &outer_tx,
                    LoopEvent::Finished {
                        session_id: session_id.clone(),
                        stop_reason,
                    },
                )
                .await;
                return;
            }
            PostTurnAction::Sleep(duration) => {
                if send_outer_event(
                    &outer_tx,
                    LoopEvent::AgentSleeping {
                        duration_secs: duration.as_secs(),
                        reason: "sleep_idle signal from tool".into(),
                    },
                )
                .await
                {
                    let _ = send_outer_event(
                        &outer_tx,
                        LoopEvent::Finished {
                            session_id: session_id.clone(),
                            stop_reason: LoopStopReason::Sleeping,
                        },
                    )
                    .await;
                }
                return;
            }
            PostTurnAction::Continue { tick_number, delay } => {
                match wait_for_next_tick(delay, &mut outer_input_rx).await {
                    WaitOutcome::Continue => {
                        tick_count = tick_number;
                        if !send_outer_event(&outer_tx, LoopEvent::TickInjected { tick_number })
                            .await
                        {
                            return;
                        }
                    }
                    WaitOutcome::Cancelled | WaitOutcome::Closed => {
                        let _ = send_outer_event(
                            &outer_tx,
                            LoopEvent::Finished {
                                session_id: session_id.clone(),
                                stop_reason: LoopStopReason::UserAbort,
                            },
                        )
                        .await;
                        return;
                    }
                }
            }
        }
    }
}

async fn forward_events(
    inner_rx: &mut mpsc::UnboundedReceiver<LoopEvent>,
    inner_input_tx: &mut mpsc::UnboundedSender<LoopInput>,
    outer_input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
    outer_tx: &mpsc::Sender<LoopEvent>,
    last_tool_output: &mut Option<String>,
) -> Option<LoopStopReason> {
    loop {
        tokio::select! {
            event = inner_rx.recv() => {
                let Some(event) = event else {
                    if !send_outer_event(
                        outer_tx,
                        LoopEvent::Error {
                            error: "inner agent event stream closed before LoopEvent::Finished"
                                .to_string(),
                        },
                    )
                    .await
                    {
                        return None;
                    }
                    return Some(LoopStopReason::ProviderError);
                };

                if let LoopEvent::ToolResult { ref output, .. } = event {
                    *last_tool_output = Some(output.clone());
                }

                if let LoopEvent::Finished { stop_reason, .. } = event {
                    return Some(stop_reason);
                }

                match deliver_event_with_backpressure(
                    event,
                    outer_tx,
                    outer_input_rx,
                    inner_input_tx,
                )
                .await
                {
                    EventDelivery::Delivered => {}
                    EventDelivery::Cancelled => {
                        if !drain_cancelled_inner_events(inner_rx, outer_tx, last_tool_output).await {
                            return None;
                        }
                        return Some(LoopStopReason::UserAbort);
                    }
                    EventDelivery::OutputClosed => return None,
                }
            }
            input = outer_input_rx.recv() => {
                let Some(input) = input else {
                    let _ = inner_input_tx.send(LoopInput::Cancel);
                    if !drain_cancelled_inner_events(inner_rx, outer_tx, last_tool_output).await {
                        return None;
                    }
                    return Some(LoopStopReason::UserAbort);
                };

                if matches!(input, LoopInput::Cancel) {
                    let _ = inner_input_tx.send(LoopInput::Cancel);
                    if !drain_cancelled_inner_events(inner_rx, outer_tx, last_tool_output).await {
                        return None;
                    }
                    return Some(LoopStopReason::UserAbort);
                }

                let _ = inner_input_tx.send(input);
            }
        }
    }
}

/// Deliver one event losslessly while still servicing control input. If the
/// consumer is slow, the pending event remains owned here and upstream pauses.
/// Cancellation is forwarded immediately, but the pending event is not
/// silently discarded; it is committed to the bounded queue before the caller
/// begins draining the cancelled inner run.
async fn deliver_event_with_backpressure(
    event: LoopEvent,
    outer_tx: &mpsc::Sender<LoopEvent>,
    outer_input_rx: &mut mpsc::UnboundedReceiver<LoopInput>,
    inner_input_tx: &mpsc::UnboundedSender<LoopInput>,
) -> EventDelivery {
    let mut cancelled = false;

    loop {
        tokio::select! {
            permit = outer_tx.reserve() => {
                let Ok(permit) = permit else {
                    tracing::warn!(
                        "TickEngine event consumer closed; cancelling active inner execution"
                    );
                    let _ = inner_input_tx.send(LoopInput::Cancel);
                    return EventDelivery::OutputClosed;
                };
                permit.send(event);
                return if cancelled {
                    EventDelivery::Cancelled
                } else {
                    EventDelivery::Delivered
                };
            }
            input = outer_input_rx.recv(), if !cancelled => {
                match input {
                    Some(LoopInput::Cancel) | None => {
                        let _ = inner_input_tx.send(LoopInput::Cancel);
                        cancelled = true;
                    }
                    Some(input) => {
                        let _ = inner_input_tx.send(input);
                    }
                }
            }
        }
    }
}

async fn drain_cancelled_inner_events(
    inner_rx: &mut mpsc::UnboundedReceiver<LoopEvent>,
    outer_tx: &mpsc::Sender<LoopEvent>,
    last_tool_output: &mut Option<String>,
) -> bool {
    while let Some(event) = inner_rx.recv().await {
        if let LoopEvent::ToolResult { ref output, .. } = event {
            *last_tool_output = Some(output.clone());
        }
        if matches!(event, LoopEvent::Finished { .. }) {
            return true;
        }
        if !send_outer_event(outer_tx, event).await {
            return false;
        }
    }

    // The caller still emits an explicit UserAbort terminal event. A closed
    // inner stream after cancellation is not treated as successful completion.
    true
}

async fn send_outer_event(outer_tx: &mpsc::Sender<LoopEvent>, event: LoopEvent) -> bool {
    outer_tx.send(event).await.is_ok()
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
        hive_crew_slug: config.hive_crew_slug.clone(),
        hive_profile: config.hive_profile.clone(),
        session_type: config.session_type,
        permission_mode: config.permission_mode,
        execution_tool_allowlist: config.execution_tool_allowlist.clone(),
        refresh_code_tools_on_mode_change: config.refresh_code_tools_on_mode_change,
        run_budget: config.run_budget,
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
    use std::time::Duration;

    use crate::agent::loop_events::{LoopEvent, LoopInput};
    use crate::agent::orchestrator::OrchestratorConfig;
    use crate::storage::SessionType;
    use tokio::sync::mpsc;

    use super::{clone_for_tick, deliver_event_with_backpressure, EventDelivery};

    #[test]
    fn clone_for_tick_preserves_session_identity() {
        let config = OrchestratorConfig {
            session_id: "session-1".to_string(),
            working_dir: PathBuf::from("/tmp/workspace"),
            project_dir: Some(PathBuf::from("/tmp/workspace/project")),
            session_type: SessionType::Hive,
            generate_title: true,
            ..Default::default()
        };

        let cloned = clone_for_tick(&config);

        assert_eq!(cloned.project_dir, config.project_dir);
        assert_eq!(cloned.session_type, SessionType::Hive);
        assert!(!cloned.generate_title);
    }

    #[tokio::test]
    async fn outer_event_delivery_applies_backpressure_without_dropping() {
        let (outer_tx, mut outer_rx) = mpsc::channel(1);
        outer_tx
            .send(LoopEvent::TextDelta {
                delta: "first".into(),
            })
            .await
            .unwrap();
        let (_outer_input_tx, mut outer_input_rx) = mpsc::unbounded_channel();
        let (inner_input_tx, _inner_input_rx) = mpsc::unbounded_channel();

        let mut delivery = tokio::spawn(async move {
            deliver_event_with_backpressure(
                LoopEvent::TextDelta {
                    delta: "second".into(),
                },
                &outer_tx,
                &mut outer_input_rx,
                &inner_input_tx,
            )
            .await
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut delivery)
                .await
                .is_err(),
            "delivery must wait while the bounded channel is full"
        );
        assert!(matches!(
            outer_rx.recv().await,
            Some(LoopEvent::TextDelta { delta }) if delta == "first"
        ));
        assert_eq!(delivery.await.unwrap(), EventDelivery::Delivered);
        assert!(matches!(
            outer_rx.recv().await,
            Some(LoopEvent::TextDelta { delta }) if delta == "second"
        ));
    }

    #[tokio::test]
    async fn cancellation_crosses_full_event_channel_without_losing_pending_event() {
        let (outer_tx, mut outer_rx) = mpsc::channel(1);
        outer_tx
            .send(LoopEvent::TextDelta {
                delta: "first".into(),
            })
            .await
            .unwrap();
        let (outer_input_tx, mut outer_input_rx) = mpsc::unbounded_channel();
        let (inner_input_tx, mut inner_input_rx) = mpsc::unbounded_channel();

        let mut delivery = tokio::spawn(async move {
            deliver_event_with_backpressure(
                LoopEvent::TextDelta {
                    delta: "pending".into(),
                },
                &outer_tx,
                &mut outer_input_rx,
                &inner_input_tx,
            )
            .await
        });

        outer_input_tx.send(LoopInput::Cancel).unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_millis(100), inner_input_rx.recv())
                .await
                .unwrap(),
            Some(LoopInput::Cancel)
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut delivery)
                .await
                .is_err(),
            "the pending event remains losslessly backpressured after cancellation"
        );

        assert!(matches!(
            outer_rx.recv().await,
            Some(LoopEvent::TextDelta { delta }) if delta == "first"
        ));
        assert_eq!(delivery.await.unwrap(), EventDelivery::Cancelled);
        assert!(matches!(
            outer_rx.recv().await,
            Some(LoopEvent::TextDelta { delta }) if delta == "pending"
        ));
    }

    #[tokio::test]
    async fn closed_event_consumer_cancels_inner_execution() {
        let (outer_tx, outer_rx) = mpsc::channel(1);
        drop(outer_rx);
        let (_outer_input_tx, mut outer_input_rx) = mpsc::unbounded_channel();
        let (inner_input_tx, mut inner_input_rx) = mpsc::unbounded_channel();

        let delivery = deliver_event_with_backpressure(
            LoopEvent::TextDelta {
                delta: "orphaned".into(),
            },
            &outer_tx,
            &mut outer_input_rx,
            &inner_input_tx,
        )
        .await;

        assert_eq!(delivery, EventDelivery::OutputClosed);
        assert!(matches!(
            inner_input_rx.recv().await,
            Some(LoopInput::Cancel)
        ));
    }
}
