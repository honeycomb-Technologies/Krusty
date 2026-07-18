use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use tokio::sync::{mpsc, RwLock};
use tokio_stream::wrappers::ReceiverStream;

use krusty_core::agent::{
    AgenticOrchestrator, LoopEvent, OrchestratorConfig, OrchestratorServices,
};
use krusty_core::ai::model_profile::ModelProfile;
use krusty_core::storage::{SessionType, WorkMode};
use krusty_core::tools::registry::PermissionMode;

use super::stream_notify::ChatStreamRunOutcome;
use super::{ChatSessionContext, SSE_CHANNEL_BUFFER};
use crate::error::AppError;
use crate::types::AgenticEvent;
use crate::AppState;

const SSE_KEEP_ALIVE_INTERVAL: Duration = Duration::from_secs(15);

// ── Orchestrator → SSE bridge ────────────────────────────────────────

fn loop_event_requires_delivery(event: &LoopEvent) -> bool {
    matches!(
        event,
        LoopEvent::AwaitingInput { .. }
            | LoopEvent::SteeringInjected { .. }
            | LoopEvent::ToolApprovalRequired { .. }
            | LoopEvent::PlanComplete { .. }
            | LoopEvent::AgentSleeping { .. }
            | LoopEvent::UserMessage { .. }
            | LoopEvent::ClassifierDecision { .. }
            | LoopEvent::Usage { .. }
            | LoopEvent::Finished { .. }
            | LoopEvent::Error { .. }
    )
}

fn event_to_sse(event: &AgenticEvent) -> Option<Event> {
    Event::default().json_data(event).ok()
}

pub(super) async fn forward_loop_event(
    sse_tx: &mpsc::Sender<Result<Event, Infallible>>,
    session_id: &str,
    loop_event: LoopEvent,
    skipped_events: &mut usize,
) -> bool {
    let requires_delivery = loop_event_requires_delivery(&loop_event);

    if *skipped_events > 0 {
        let lagged_event = AgenticEvent::Lagged {
            skipped: *skipped_events,
        };

        if let Some(sse_event) = event_to_sse(&lagged_event) {
            if requires_delivery {
                if sse_tx.send(Ok(sse_event)).await.is_err() {
                    return false;
                }
                *skipped_events = 0;
            } else {
                match sse_tx.try_send(Ok(sse_event)) {
                    Ok(()) => *skipped_events = 0,
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        *skipped_events = skipped_events.saturating_add(1);
                        tracing::warn!(
                            session_id,
                            skipped = *skipped_events,
                            "Dropping SSE event because client queue is full"
                        );
                        return true;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => return false,
                }
            }
        } else {
            *skipped_events = 0;
        }
    }

    let agentic_event: AgenticEvent = loop_event.into();
    let Some(sse_event) = event_to_sse(&agentic_event) else {
        return true;
    };

    if requires_delivery {
        sse_tx.send(Ok(sse_event)).await.is_ok()
    } else {
        match sse_tx.try_send(Ok(sse_event)) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(_)) => {
                *skipped_events = skipped_events.saturating_add(1);
                tracing::warn!(
                    session_id,
                    skipped = *skipped_events,
                    "Dropping SSE event because client queue is full"
                );
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }
}

pub(super) async fn start_orchestrator_sse(
    state: &AppState,
    ctx: ChatSessionContext,
    work_mode: WorkMode,
    permission_mode: PermissionMode,
    generate_title: bool,
) -> Result<Sse<ReceiverStream<Result<Event, Infallible>>>, AppError> {
    if ctx.session_type == SessionType::Mako {
        return Err(AppError::BadGateway(
            "Mako execution is owned by the daemon control plane".to_string(),
        ));
    }

    let (sse_tx, sse_rx) = mpsc::channel::<Result<Event, Infallible>>(SSE_CHANNEL_BUFFER);
    let stream_idle_timeout = model_stream_idle_timeout(&ctx.ai_client);

    let services = OrchestratorServices {
        ai_client: ctx.ai_client,
        tool_registry: Arc::clone(&state.tool_registry),
        process_registry: Arc::clone(&state.process_registry),
        db_path: (*state.db_path).clone(),
        skills_manager: Arc::clone(&state.skills_manager),
    };
    let config = OrchestratorConfig {
        session_id: ctx.session_id.clone(),
        working_dir: ctx.working_dir,
        project_dir: ctx.project_dir,
        mako_crew_slug: ctx.mako_crew_slug.clone(),
        session_type: ctx.session_type,
        permission_mode,
        user_id: ctx.user_id.clone(),
        initial_work_mode: work_mode,
        stream_idle_timeout,
        generate_title,
        max_iterations: krusty_core::agent::AgentConfig::default().primary_max_turns(),
        ..Default::default()
    };

    let orchestrator = AgenticOrchestrator::new(services, config);
    let (event_rx, input_tx) = orchestrator.run(ctx.conversation, ctx.options);

    let session_id = ctx.session_id;
    {
        let mut inputs = state.session_inputs.write().await;
        inputs.insert(session_id.clone(), input_tx);
    }

    let session_inputs = Arc::clone(&state.session_inputs);
    let active_agent_streams = Arc::clone(&state.active_agent_streams);
    let push_service = state.push_service.clone();
    let apns_service = state.apns_service.clone();
    let user_id = ctx.user_id;
    let db_path = Arc::clone(&state.db_path);
    let guard = ctx.guard;

    tokio::spawn(async move {
        active_agent_streams.fetch_add(1, Ordering::Relaxed);
        let _guard = guard;
        run_orchestrator_event_bridge(
            event_rx,
            sse_tx,
            session_id,
            session_inputs,
            push_service,
            apns_service,
            user_id,
            db_path,
        )
        .await;
        active_agent_streams.fetch_sub(1, Ordering::Relaxed);
    });

    let stream = ReceiverStream::new(sse_rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(SSE_KEEP_ALIVE_INTERVAL)))
}

fn model_stream_idle_timeout(ai_client: &Arc<krusty_core::ai::client::AiClient>) -> Duration {
    let profile = ModelProfile::resolve(
        ai_client.provider_id(),
        ai_client.config().api_format,
        &ai_client.config().model,
    );
    Duration::from_secs(profile.stream_idle_timeout_secs)
}

pub(super) async fn run_orchestrator_event_bridge(
    mut event_rx: mpsc::UnboundedReceiver<LoopEvent>,
    sse_tx: mpsc::Sender<Result<Event, Infallible>>,
    session_id: String,
    session_inputs: Arc<RwLock<crate::SessionInputMap>>,
    push_service: Option<Arc<crate::push::PushService>>,
    apns_service: Option<Arc<crate::apns::ApnsService>>,
    user_id: Option<String>,
    db_path: Arc<std::path::PathBuf>,
) {
    let mut outcome = ChatStreamRunOutcome::default();
    let mut skipped_events = 0usize;
    let mut sse_connected = true;

    while let Some(loop_event) = event_rx.recv().await {
        outcome.record_event(
            &push_service,
            &apns_service,
            user_id.as_deref(),
            &session_id,
            &loop_event,
        );
        let is_finished = matches!(loop_event, LoopEvent::Finished { .. });

        if sse_connected
            && !forward_loop_event(&sse_tx, &session_id, loop_event, &mut skipped_events).await
        {
            sse_connected = false;
            skipped_events = 0;
            tracing::info!(
                session_id = %session_id,
                "SSE client disconnected; continuing to drain orchestrator events"
            );
        }

        if is_finished {
            break;
        }
    }

    outcome.finalize(
        &push_service,
        &apns_service,
        user_id.as_deref(),
        &session_id,
        db_path.as_ref(),
    );
    session_inputs.write().await.remove(&session_id);
}
