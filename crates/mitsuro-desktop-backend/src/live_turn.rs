//! Progressive live `turn/start` streaming with mid-stream approval handling.
//!
//! Unlike collect-all-then-apply, this path delivers [`TurnStreamEvent`]s as they
//! arrive from app-server stdout. When an [`TurnStreamEvent::ApprovalRequested`]
//! is observed, the configured policy answers via
//! [`CodexAppServerBackend::respond_approval`] **before** waiting for further
//! server events — avoiding hangs when the server blocks on the approval RPC.

use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tracing::debug;

use crate::approvals::{ApprovalChoice, PendingApproval};
use crate::backend::AgentBackend;
use crate::codex::CodexAppServerBackend;
use crate::protocol::{ReviewStartParams, TurnStartParams};
use crate::types::{AgentError, Result, TurnStreamEvent};

/// Default overall budget for a progressive live turn (wall clock).
pub const DEFAULT_LIVE_TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// How to resolve mid-stream server approval requests during a live turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiveApprovalPolicy {
    /// Write an approve decision immediately (tests / non-interactive).
    #[default]
    AutoApprove,
    /// Write a reject decision immediately.
    AutoReject,
}

impl LiveApprovalPolicy {
    pub fn choice(self) -> ApprovalChoice {
        match self {
            Self::AutoApprove => ApprovalChoice::Approve,
            Self::AutoReject => ApprovalChoice::Reject,
        }
    }
}

/// Bridge for interactive UI: background live turn waits; UI submits a choice.
///
/// Typical desktop flow:
/// 1. Progressive runner hits `ApprovalRequested`, delivers the event, then
///    calls [`LiveApprovalBridge::wait`] (blocks the turn loop).
/// 2. UI shows ApprovalBar from the delivered event.
/// 3. User clicks Approve/Reject → [`LiveApprovalBridge::submit`].
/// 4. Wait returns; runner writes `respond_approval` and continues.
#[derive(Debug, Default)]
pub struct LiveApprovalBridge {
    waiter: Mutex<Option<std::sync::mpsc::Sender<ApprovalChoice>>>,
}

impl LiveApprovalBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Block until [`submit`] provides a choice (or the sender is dropped → Reject).
    pub fn wait(&self) -> ApprovalChoice {
        let (tx, rx) = std::sync::mpsc::channel();
        {
            let mut slot = self.waiter.lock().unwrap_or_else(|e| e.into_inner());
            *slot = Some(tx);
        }
        rx.recv().unwrap_or(ApprovalChoice::Reject)
    }

    /// Unblock a pending [`wait`] with the user's decision.
    ///
    /// Returns `true` when a waiter was present (live progressive path).
    pub fn submit(&self, choice: ApprovalChoice) -> bool {
        let mut slot = self.waiter.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = slot.take() {
            tx.send(choice).is_ok()
        } else {
            false
        }
    }

    /// True when the live turn loop is blocked on a user decision.
    pub fn is_waiting(&self) -> bool {
        self.waiter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
}

/// Outcome of a progressive live turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveTurnOutcome {
    pub event_count: usize,
    pub approvals_answered: usize,
    pub completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveReviewOutcome {
    pub review_thread_id: String,
    pub turn_id: String,
    pub stream: LiveTurnOutcome,
}

/// Run a live turn, invoking `on_event` as soon as each event arrives.
///
/// Mid-stream approvals are answered via `on_approval` **before** the loop
/// continues receiving. The returned future from `on_approval` may block
/// (e.g. wait on [`LiveApprovalBridge`]) so the UI can show an ApprovalBar.
pub async fn run_live_turn_progressive<E, F, Fut>(
    backend: &CodexAppServerBackend,
    thread_id: String,
    text: String,
    on_event: E,
    on_approval: F,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome>
where
    E: FnMut(TurnStreamEvent),
    F: FnMut(PendingApproval) -> Fut,
    Fut: Future<Output = ApprovalChoice>,
{
    run_live_turn_progressive_with_model(
        backend,
        thread_id,
        text,
        None,
        on_event,
        on_approval,
        overall_timeout,
    )
    .await
}

/// Progressive live turn with optional model override for `TurnStartParams.model`.
pub async fn run_live_turn_progressive_with_model<E, F, Fut>(
    backend: &CodexAppServerBackend,
    thread_id: String,
    text: String,
    model: Option<String>,
    mut on_event: E,
    mut on_approval: F,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome>
where
    E: FnMut(TurnStreamEvent),
    F: FnMut(PendingApproval) -> Fut,
    Fut: Future<Output = ApprovalChoice>,
{
    let rx = backend
        .subscribe_turn_events()
        .await
        .ok_or_else(|| AgentError::Other("notification stream unavailable".into()))?;

    let _resp = backend
        .turn_start(TurnStartParams::text_with_model(
            thread_id.clone(),
            text,
            model,
        ))
        .await?;

    consume_live_events(
        backend,
        rx,
        thread_id,
        &mut on_event,
        &mut on_approval,
        overall_timeout,
    )
    .await
}

async fn consume_live_events<E, F, Fut>(
    backend: &CodexAppServerBackend,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<TurnStreamEvent>,
    thread_id: String,
    on_event: &mut E,
    on_approval: &mut F,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome>
where
    E: FnMut(TurnStreamEvent),
    F: FnMut(PendingApproval) -> Fut,
    Fut: Future<Output = ApprovalChoice>,
{
    let mut event_count = 0usize;
    let mut approvals_answered = 0usize;
    let mut completed = false;
    let deadline = tokio::time::Instant::now() + overall_timeout;

    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            break;
        }
        // Cap per-recv wait so long model silence still respects overall timeout.
        let wait = left.min(Duration::from_secs(30));
        match tokio::time::timeout(wait, rx.recv()).await {
            Ok(Some(ev)) => {
                let is_ours = ev.thread_id().map(|t| t == thread_id).unwrap_or(true);
                if !is_ours {
                    debug!("live turn: skip event for other thread");
                    continue;
                }

                let pending = match &ev {
                    TurnStreamEvent::ApprovalRequested(p) => Some(p.clone()),
                    _ => None,
                };
                let done = matches!(ev, TurnStreamEvent::TurnCompleted { .. });

                // Deliver first so UI can surface ApprovalBar before we answer.
                on_event(ev);
                event_count += 1;

                if let Some(pending) = pending {
                    let choice = on_approval(pending.clone()).await;
                    backend.respond_approval(&pending, choice).await?;
                    approvals_answered += 1;
                }

                if done {
                    completed = true;
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => {
                // Per-recv timeout; loop checks overall deadline.
                continue;
            }
        }
    }

    Ok(LiveTurnOutcome {
        event_count,
        approvals_answered,
        completed,
    })
}

/// Start a code-review turn and consume its progressive event stream.
///
/// The subscription is established before `review/start`. Filtering follows the
/// returned `reviewThreadId`, which keeps detached reviews isolated correctly.
pub async fn run_live_review_with_bridge<E>(
    backend: &CodexAppServerBackend,
    params: ReviewStartParams,
    mut on_event: E,
    bridge: Arc<LiveApprovalBridge>,
    overall_timeout: Duration,
) -> Result<LiveReviewOutcome>
where
    E: FnMut(TurnStreamEvent),
{
    let rx = backend
        .subscribe_turn_events()
        .await
        .ok_or_else(|| AgentError::Other("notification stream unavailable".into()))?;
    let response = backend.review_start(params).await?;
    let turn_id = response
        .turn_id()
        .ok_or_else(|| AgentError::Protocol("review/start response is missing turn.id".into()))?
        .to_owned();
    let review_thread_id = response.review_thread_id;
    let mut on_approval = move |_pending: PendingApproval| {
        let bridge = Arc::clone(&bridge);
        async move {
            tokio::task::spawn_blocking(move || bridge.wait())
                .await
                .unwrap_or(ApprovalChoice::Reject)
        }
    };
    let stream = consume_live_events(
        backend,
        rx,
        review_thread_id.clone(),
        &mut on_event,
        &mut on_approval,
        overall_timeout,
    )
    .await?;
    Ok(LiveReviewOutcome {
        review_thread_id,
        turn_id,
        stream,
    })
}

/// Progressive live turn with a fixed non-interactive approval policy.
pub async fn run_live_turn_with_policy<E>(
    backend: &CodexAppServerBackend,
    thread_id: String,
    text: String,
    on_event: E,
    policy: LiveApprovalPolicy,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome>
where
    E: FnMut(TurnStreamEvent),
{
    run_live_turn_with_policy_and_model(
        backend,
        thread_id,
        text,
        None,
        on_event,
        policy,
        overall_timeout,
    )
    .await
}

/// Like [`run_live_turn_with_policy`] but forwards `model` into `turn/start`.
pub async fn run_live_turn_with_policy_and_model<E>(
    backend: &CodexAppServerBackend,
    thread_id: String,
    text: String,
    model: Option<String>,
    on_event: E,
    policy: LiveApprovalPolicy,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome>
where
    E: FnMut(TurnStreamEvent),
{
    let choice = policy.choice();
    run_live_turn_progressive_with_model(
        backend,
        thread_id,
        text,
        model,
        on_event,
        move |_pending| {
            let c = choice;
            async move { c }
        },
        overall_timeout,
    )
    .await
}

/// Progressive live turn that blocks on [`LiveApprovalBridge`] for each approval.
pub async fn run_live_turn_with_bridge<E>(
    backend: &CodexAppServerBackend,
    thread_id: String,
    text: String,
    on_event: E,
    bridge: Arc<LiveApprovalBridge>,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome>
where
    E: FnMut(TurnStreamEvent),
{
    run_live_turn_with_bridge_and_model(
        backend,
        thread_id,
        text,
        None,
        on_event,
        bridge,
        overall_timeout,
    )
    .await
}

/// Like [`run_live_turn_with_bridge`] but forwards selected model into `turn/start`.
pub async fn run_live_turn_with_bridge_and_model<E>(
    backend: &CodexAppServerBackend,
    thread_id: String,
    text: String,
    model: Option<String>,
    on_event: E,
    bridge: Arc<LiveApprovalBridge>,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome>
where
    E: FnMut(TurnStreamEvent),
{
    run_live_turn_progressive_with_model(
        backend,
        thread_id,
        text,
        model,
        on_event,
        move |_pending| {
            let bridge = Arc::clone(&bridge);
            async move {
                // Wait on a blocking channel off the async worker so the runtime
                // can still process stdout while the UI decides.
                tokio::task::spawn_blocking(move || bridge.wait())
                    .await
                    .unwrap_or(ApprovalChoice::Reject)
            }
        },
        overall_timeout,
    )
    .await
}

/// Blocking helper for non-async callers (desktop `background_spawn` + current-thread RT).
pub fn run_live_turn_with_policy_blocking(
    backend: Arc<CodexAppServerBackend>,
    thread_id: String,
    text: String,
    event_tx: std::sync::mpsc::Sender<TurnStreamEvent>,
    policy: LiveApprovalPolicy,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| AgentError::Other(format!("tokio runtime: {e}")))?;
    rt.block_on(run_live_turn_with_policy(
        backend.as_ref(),
        thread_id,
        text,
        |ev| {
            let _ = event_tx.send(ev);
        },
        policy,
        overall_timeout,
    ))
}

/// Blocking progressive live turn with interactive bridge (desktop).
///
/// Events are forwarded on `event_tx` as they arrive. When an approval is
/// needed the turn loop blocks on `bridge` until the UI calls
/// [`LiveApprovalBridge::submit`]; the JSON-RPC approval response is then
/// written before further events are consumed.
pub fn run_live_turn_with_bridge_blocking(
    backend: Arc<CodexAppServerBackend>,
    thread_id: String,
    text: String,
    event_tx: std::sync::mpsc::Sender<TurnStreamEvent>,
    bridge: Arc<LiveApprovalBridge>,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome> {
    run_live_turn_with_bridge_blocking_and_model(
        backend,
        thread_id,
        text,
        None,
        event_tx,
        bridge,
        overall_timeout,
    )
}

/// Desktop live path: pass selected model slug/id into `TurnStartParams.model`.
pub fn run_live_turn_with_bridge_blocking_and_model(
    backend: Arc<CodexAppServerBackend>,
    thread_id: String,
    text: String,
    model: Option<String>,
    event_tx: std::sync::mpsc::Sender<TurnStreamEvent>,
    bridge: Arc<LiveApprovalBridge>,
    overall_timeout: Duration,
) -> Result<LiveTurnOutcome> {
    // Multi-thread so spawn_blocking (approval wait) does not stall stdout reads
    // on the same current-thread runtime.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| AgentError::Other(format!("tokio runtime: {e}")))?;
    rt.block_on(run_live_turn_with_bridge_and_model(
        backend.as_ref(),
        thread_id,
        text,
        model,
        |ev| {
            let _ = event_tx.send(ev);
        },
        bridge,
        overall_timeout,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approvals::ApprovalKind;
    use crate::protocol::{
        InitializeResponse, JsonRpcId, ReviewDelivery, ReviewStartParams, ReviewTarget,
    };
    use serde_json::Value;
    use tokio::io::{duplex, AsyncReadExt};

    async fn read_line_from(reader: &mut (impl AsyncReadExt + Unpin)) -> String {
        let mut buf = vec![0u8; 8192];
        let mut acc = String::new();
        loop {
            let n = reader.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            acc.push_str(&String::from_utf8_lossy(&buf[..n]));
            if acc.contains('\n') {
                break;
            }
        }
        acc.lines().next().unwrap_or("").to_string()
    }

    #[tokio::test]
    async fn detached_review_follows_returned_thread_until_completion() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = Arc::new(CodexAppServerBackend::with_defaults());
        backend.connect_with_mock_writer(client_writer).await;
        backend.mark_ready_for_test(InitializeResponse {
            codex_home: "/tmp".into(),
            platform_family: "unix".into(),
            platform_os: "linux".into(),
            user_agent: "test".into(),
        });

        let server_backend = Arc::clone(&backend);
        let server = tokio::spawn(async move {
            let line = read_line_from(&mut server_reader).await;
            let request: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(request["method"], "review/start");
            assert_eq!(request["params"]["delivery"], "detached");
            let id = request["id"].clone();
            server_backend
                .inject_stdout_line(
                    &serde_json::json!({
                        "id": id,
                        "result": {
                            "reviewThreadId": "review-thread",
                            "turn": {"id": "review-turn", "status": "inProgress"}
                        }
                    })
                    .to_string(),
                )
                .await;
            server_backend
                .inject_stdout_line(
                    r#"{"method":"item/agentMessage/delta","params":{"threadId":"source-thread","turnId":"other","itemId":"other-item","delta":"ignore"}}"#,
                )
                .await;
            for notification in [
                serde_json::json!({"method":"turn/started","params":{"threadId":"review-thread","turn":{"id":"review-turn"}}}),
                serde_json::json!({"method":"item/agentMessage/delta","params":{"threadId":"review-thread","turnId":"review-turn","itemId":"review-message","delta":"finding"}}),
                serde_json::json!({"method":"turn/completed","params":{"threadId":"review-thread","turn":{"id":"review-turn","status":"completed"}}}),
            ] {
                server_backend
                    .inject_stdout_line(&notification.to_string())
                    .await;
            }
        });

        let events = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&events);
        let outcome = run_live_review_with_bridge(
            backend.as_ref(),
            ReviewStartParams {
                thread_id: "source-thread".to_owned(),
                target: ReviewTarget::UncommittedChanges,
                delivery: Some(ReviewDelivery::Detached),
            },
            move |event| captured.lock().unwrap().push(event),
            Arc::new(LiveApprovalBridge::new()),
            Duration::from_secs(2),
        )
        .await
        .unwrap();

        assert_eq!(outcome.review_thread_id, "review-thread");
        assert_eq!(outcome.turn_id, "review-turn");
        assert!(outcome.stream.completed);
        assert_eq!(outcome.stream.event_count, 3);
        assert!(events.lock().unwrap().iter().all(|event| {
            event
                .thread_id()
                .is_none_or(|thread_id| thread_id == "review-thread")
        }));
        server.await.unwrap();
    }

    /// Mock stdio live turn with mid-stream approval: assert approve is written
    /// to the client→server pipe **before** turn/completed is delivered.
    #[tokio::test]
    async fn progressive_live_turn_answers_midstream_approval_before_complete() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = Arc::new(CodexAppServerBackend::with_defaults());
        backend.connect_with_mock_writer(client_writer).await;
        backend.mark_ready_for_test(InitializeResponse {
            codex_home: "/tmp".into(),
            platform_family: "unix".into(),
            platform_os: "linux".into(),
            user_agent: "test".into(),
        });

        let backend_inj = Arc::clone(&backend);
        let server = tokio::spawn(async move {
            // 1) Read turn/start request from client stdin write.
            let line = read_line_from(&mut server_reader).await;
            let req: Value = serde_json::from_str(&line).expect("turn/start json");
            assert_eq!(req["method"], "turn/start");
            let id = req["id"].clone();
            let thread_id = req["params"]["threadId"].as_str().unwrap_or("t-live");

            // 2) Respond to turn/start RPC.
            let turn_resp = serde_json::json!({
                "id": id,
                "result": {
                    "turn": {
                        "id": "turn-live-1",
                        "status": "inProgress"
                    }
                }
            });
            backend_inj.inject_stdout_line(&turn_resp.to_string()).await;

            // 3) Progressive notifications (before approval).
            backend_inj
                .inject_stdout_line(
                    &serde_json::json!({
                        "method": "turn/started",
                        "params": {
                            "threadId": thread_id,
                            "turn": { "id": "turn-live-1", "status": "inProgress" }
                        },
                        "emittedAtMs": 1
                    })
                    .to_string(),
                )
                .await;

            backend_inj
                .inject_stdout_line(
                    &serde_json::json!({
                        "method": "item/agentMessage/delta",
                        "params": {
                            "threadId": thread_id,
                            "turnId": "turn-live-1",
                            "itemId": "msg-1",
                            "delta": "Hello "
                        },
                        "emittedAtMs": 2
                    })
                    .to_string(),
                )
                .await;

            // 4) Mid-stream server approval request (must be answered before complete).
            backend_inj
                .inject_stdout_line(
                    &serde_json::json!({
                        "id": 42,
                        "method": "item/commandExecution/requestApproval",
                        "params": {
                            "threadId": thread_id,
                            "turnId": "turn-live-1",
                            "itemId": "cmd-1",
                            "command": "echo midstream",
                            "cwd": "/tmp",
                            "parsedCmd": []
                        }
                    })
                    .to_string(),
                )
                .await;

            // 5) Read approval response — must arrive before we emit turn/completed.
            let approval_line = read_line_from(&mut server_reader).await;
            let approval: Value =
                serde_json::from_str(&approval_line).expect("approval response json");
            assert_eq!(
                approval["id"], 42,
                "approval response must target request id 42"
            );
            assert_eq!(
                approval["result"]["decision"], "accept",
                "auto-approve policy should write accept"
            );

            // 6) Only after approval: more deltas + turn completed.
            backend_inj
                .inject_stdout_line(
                    &serde_json::json!({
                        "method": "item/agentMessage/delta",
                        "params": {
                            "threadId": thread_id,
                            "turnId": "turn-live-1",
                            "itemId": "msg-1",
                            "delta": "world"
                        },
                        "emittedAtMs": 3
                    })
                    .to_string(),
                )
                .await;

            backend_inj
                .inject_stdout_line(
                    &serde_json::json!({
                        "method": "turn/completed",
                        "params": {
                            "threadId": thread_id,
                            "turn": { "id": "turn-live-1", "status": "completed" }
                        },
                        "emittedAtMs": 4
                    })
                    .to_string(),
                )
                .await;

            approval_line
        });

        let events: Arc<Mutex<Vec<TurnStreamEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let events_cb = Arc::clone(&events);
        let outcome = run_live_turn_with_policy(
            backend.as_ref(),
            "t-live".into(),
            "ping".into(),
            move |ev| {
                events_cb.lock().unwrap_or_else(|e| e.into_inner()).push(ev);
            },
            LiveApprovalPolicy::AutoApprove,
            Duration::from_secs(5),
        )
        .await
        .expect("progressive live turn");

        assert!(outcome.completed, "turn should complete");
        assert_eq!(outcome.approvals_answered, 1);
        assert!(outcome.event_count >= 4);

        // Drain delivered events; approval must appear before TurnCompleted.
        let events = events.lock().unwrap_or_else(|e| e.into_inner()).clone();

        let approval_idx = events
            .iter()
            .position(|e| matches!(e, TurnStreamEvent::ApprovalRequested(_)))
            .expect("ApprovalRequested delivered");
        let complete_idx = events
            .iter()
            .position(|e| matches!(e, TurnStreamEvent::TurnCompleted { .. }))
            .expect("TurnCompleted delivered");
        assert!(
            approval_idx < complete_idx,
            "approval event must be delivered before turn completed"
        );

        if let TurnStreamEvent::ApprovalRequested(p) = &events[approval_idx] {
            assert_eq!(p.kind, ApprovalKind::CommandExecution);
            assert_eq!(p.request_id, JsonRpcId::Number(42));
            assert!(p.summary.contains("echo midstream"));
        }

        let written = server.await.expect("server task");
        assert!(written.contains("\"decision\":\"accept\"") || written.contains("accept"));
    }

    #[tokio::test]
    async fn bridge_submit_unblocks_wait() {
        let bridge = Arc::new(LiveApprovalBridge::new());
        let b = Arc::clone(&bridge);
        let handle = tokio::task::spawn_blocking(move || b.wait());
        // Give waiter time to install sender.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(bridge.is_waiting());
        assert!(bridge.submit(ApprovalChoice::Approve));
        let choice = handle.await.unwrap();
        assert_eq!(choice, ApprovalChoice::Approve);
        assert!(!bridge.is_waiting());
    }

    #[test]
    fn policy_choice_mapping() {
        assert_eq!(
            LiveApprovalPolicy::AutoApprove.choice(),
            ApprovalChoice::Approve
        );
        assert_eq!(
            LiveApprovalPolicy::AutoReject.choice(),
            ApprovalChoice::Reject
        );
    }

    /// Ensure turn/start is still issued (mock) when using progressive helper
    /// after a minimal initialize-style ready mark.
    #[tokio::test]
    async fn progressive_turn_sends_turn_start_request() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = Arc::new(CodexAppServerBackend::with_defaults());
        backend.connect_with_mock_writer(client_writer).await;
        backend.mark_ready_for_test(InitializeResponse {
            codex_home: "/tmp".into(),
            platform_family: "unix".into(),
            platform_os: "linux".into(),
            user_agent: "test".into(),
        });

        // No subscription consumer for full turn — just verify request write + quick cancel via drop.
        let backend2 = Arc::clone(&backend);
        let reader = tokio::spawn(async move {
            let line = read_line_from(&mut server_reader).await;
            let req: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["method"], "turn/start");
            assert_eq!(req["params"]["threadId"], "tid");
            // Answer so request() unblocks; then hang (no notifications → timeout outcome).
            backend2
                .inject_stdout_line(
                    &serde_json::json!({
                        "id": req["id"],
                        "result": { "turn": { "id": "u1", "status": "inProgress" } }
                    })
                    .to_string(),
                )
                .await;
            line
        });

        let outcome = run_live_turn_with_policy(
            backend.as_ref(),
            "tid".into(),
            "hi".into(),
            |_ev| {},
            LiveApprovalPolicy::AutoApprove,
            Duration::from_millis(200),
        )
        .await
        .expect("should return after timeout with no events");

        assert!(!outcome.completed);
        assert_eq!(outcome.event_count, 0);
        let _ = reader.await;
    }

    /// Selected UI model must be serialized as camelCase `model` on turn/start.
    #[tokio::test]
    async fn progressive_turn_forwards_selected_model() {
        let (client_writer, mut server_reader) = duplex(64 * 1024);
        let backend = Arc::new(CodexAppServerBackend::with_defaults());
        backend.connect_with_mock_writer(client_writer).await;
        backend.mark_ready_for_test(InitializeResponse {
            codex_home: "/tmp".into(),
            platform_family: "unix".into(),
            platform_os: "linux".into(),
            user_agent: "test".into(),
        });

        let backend2 = Arc::clone(&backend);
        let reader = tokio::spawn(async move {
            let line = read_line_from(&mut server_reader).await;
            let req: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["method"], "turn/start");
            assert_eq!(req["params"]["threadId"], "tid-model");
            assert_eq!(
                req["params"]["model"], "gpt-5",
                "selected model must be wire-camelCase `model`"
            );
            assert!(
                req["params"].get("thread_id").is_none(),
                "must not emit snake_case thread_id"
            );
            backend2
                .inject_stdout_line(
                    &serde_json::json!({
                        "id": req["id"],
                        "result": { "turn": { "id": "u-model", "status": "inProgress" } }
                    })
                    .to_string(),
                )
                .await;
            line
        });

        let outcome = run_live_turn_with_policy_and_model(
            backend.as_ref(),
            "tid-model".into(),
            "hi".into(),
            Some("gpt-5".into()),
            |_ev| {},
            LiveApprovalPolicy::AutoApprove,
            Duration::from_millis(200),
        )
        .await
        .expect("timeout outcome");

        assert!(!outcome.completed);
        let _ = reader.await;
    }
}
