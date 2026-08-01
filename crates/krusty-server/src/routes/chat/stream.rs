use std::convert::Infallible;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive, Sse};
use tokio::sync::{mpsc, OwnedMutexGuard, RwLock};
use tokio_stream::wrappers::ReceiverStream;

use krusty_core::agent::{
    LoopEvent, LoopInput, OrchestratorServices, RunProvenance, RunSpecBuilder,
};
use krusty_core::ai::transport_policy::StreamTransportPolicy;
use krusty_core::storage::{Database, SessionType, WorkMode};
use krusty_core::tools::registry::PermissionMode;
use krusty_core::SessionManager;

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
            | LoopEvent::RunBudgetResolved { .. }
            | LoopEvent::ProviderRequestPrepared { .. }
            | LoopEvent::MicrocompactionApplied { .. }
            | LoopEvent::ProgressGuard { .. }
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
    let (sse_tx, sse_rx) = mpsc::channel::<Result<Event, Infallible>>(SSE_CHANNEL_BUFFER);
    let run = start_chat_run(state, ctx, work_mode, permission_mode, generate_title)?;
    launch_chat_run_bridge(state, run, sse_tx).await;

    let stream = ReceiverStream::new(sse_rx);
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(SSE_KEEP_ALIVE_INTERVAL)))
}

/// Resume an idle Chat/Code session without an attached SSE client while
/// retaining the normal run registration, notification, trace, cleanup, and
/// session-lock lifecycle.
pub(super) async fn start_orchestrator_detached(
    state: &AppState,
    ctx: ChatSessionContext,
    work_mode: WorkMode,
    permission_mode: PermissionMode,
) -> Result<(), AppError> {
    let run = start_chat_run(state, ctx, work_mode, permission_mode, false)?;
    let (sse_tx, sse_rx) = mpsc::channel::<Result<Event, Infallible>>(1);
    drop(sse_rx);
    launch_chat_run_bridge(state, run, sse_tx).await;
    Ok(())
}

struct StartedChatRun {
    event_rx: mpsc::UnboundedReceiver<LoopEvent>,
    input_tx: mpsc::UnboundedSender<LoopInput>,
    session_id: String,
    user_id: Option<String>,
    guard: OwnedMutexGuard<()>,
}

fn start_chat_run(
    state: &AppState,
    ctx: ChatSessionContext,
    work_mode: WorkMode,
    permission_mode: PermissionMode,
    generate_title: bool,
) -> Result<StartedChatRun, AppError> {
    if ctx.session_type == SessionType::Mako {
        return Err(AppError::BadGateway(
            "Hive execution is owned by its background service".to_string(),
        ));
    }

    let stream_idle_timeout = model_stream_idle_timeout(&ctx.ai_client);
    let mode_aware_code_tools =
        ctx.session_type == SessionType::Code && ctx.options.tools.is_some();

    let run_spec = RunSpecBuilder::new(
        RunProvenance::Server,
        ctx.session_id.clone(),
        ctx.working_dir,
        ctx.session_type,
    )
    .project_dir(ctx.project_dir)
    .mako_crew_slug(ctx.mako_crew_slug.clone())
    .permission_mode(permission_mode)
    .execution_tool_allowlist(ctx.execution_tool_allowlist)
    .user_id(ctx.user_id.clone())
    .initial_work_mode(work_mode)
    .mode_aware_code_tools(mode_aware_code_tools)
    .stream_idle_timeout(stream_idle_timeout)
    .generate_title(generate_title)
    .call_options(ctx.options)
    .build(ctx.ai_client.as_ref())
    .map_err(|error| AppError::BadRequest(error.to_string()))?;
    let services = OrchestratorServices {
        ai_client: ctx.ai_client,
        tool_registry: Arc::clone(&state.tool_registry),
        process_registry: Arc::clone(&state.process_registry),
        db_path: (*state.db_path).clone(),
        skills_manager: Arc::clone(&state.skills_manager),
    };

    let (event_rx, input_tx) = run_spec.start(services, ctx.conversation);

    Ok(StartedChatRun {
        event_rx,
        input_tx,
        session_id: ctx.session_id,
        user_id: ctx.user_id,
        guard: ctx.guard,
    })
}

async fn launch_chat_run_bridge(
    state: &AppState,
    run: StartedChatRun,
    sse_tx: mpsc::Sender<Result<Event, Infallible>>,
) {
    let StartedChatRun {
        event_rx,
        input_tx,
        session_id,
        user_id,
        guard,
    } = run;
    {
        let mut inputs = state.session_inputs.write().await;
        inputs.insert(session_id.clone(), input_tx);
    }

    let session_inputs = Arc::clone(&state.session_inputs);
    let active_agent_streams = Arc::clone(&state.active_agent_streams);
    let push_service = state.push_service.clone();
    let apns_service = state.apns_service.clone();
    let db_path = Arc::clone(&state.db_path);

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
}

fn model_stream_idle_timeout(ai_client: &Arc<krusty_core::ai::client::AiClient>) -> Duration {
    StreamTransportPolicy::resolve(ai_client.provider_id(), ai_client.config().api_format)
        .idle_timeout
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
    yield_orphaned_continuation_claim(db_path.as_ref(), &session_id);
    session_inputs.write().await.remove(&session_id);
}

/// `RunSpec::start` spawns the orchestrator before returning its event stream.
/// If that task exits before its initial recovery handoff, no event can clear
/// the durable `resuming_input` lease. Yield the transient lease here while
/// retaining the accepted response, allowing an exact retry instead of a
/// permanently busy session or a lost prompt.
fn yield_orphaned_continuation_claim(db_path: &std::path::Path, session_id: &str) {
    let result = (|| -> anyhow::Result<()> {
        let session_manager = SessionManager::new(Database::new(db_path)?);
        let Some(recovery) = session_manager.load_recovery_state(session_id)? else {
            return Ok(());
        };
        let Some(claim) = recovery.continuation_claim else {
            return Ok(());
        };
        session_manager.yield_awaiting_interaction_claim(
            session_id,
            &claim.interaction_id,
            &claim.accepted_response,
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        tracing::error!(
            session_id,
            %error,
            "Failed to yield orphaned continuation execution lease"
        );
    }
}
