use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use mitsuro_core::ai::providers::ProviderId;
use mitsuro_core::hive::{
    canonical_timestamp, DstPolicy, HiveRunStatus, MisfireConfig, MisfireDispatch,
    MisfireResolution, RecurrenceV1, RetryPolicy,
};
use mitsuro_core::storage::{
    ClaimRunRequest, DaemonFence, DaemonLeaseAcquire, Database, HiveDaemonLeaseStore, HiveRunStore,
    HiveScheduleStore, RunCompletion, SessionManager, SessionType, WorkspaceMode,
};
use mitsuro_hive_protocol::{
    Actor, Command, CreateScheduleCommand, DispatchCommand, ExtensionCommand, GroupMessageCommand,
    HiveEvent, MessageCommand, ModelKey, PeerIdentity, ReplaceScheduleCommand, ResponsePayload,
    ScheduleCommand, ScheduleDefinition, SessionCommand, SetPriorityCommand, SteerCommand,
    SubscribeCommand, ToolApprovalCommand, UserResponseCommand,
};
use tempfile::TempDir;
use tokio::sync::Notify;

use crate::{CommandContext, CommandHandler, HandlerReply};

use super::pump::materialize_schedule_transaction;
use super::{
    start_runtime, DurableHiveCommandHandler, ExecutionBackend, ExecutionControl, ExecutionEvent,
    ExecutionOutcome, ExecutionRequest, HiveRuntimeConfig,
};

// Each case below launches a complete scheduler with its own Tokio blocking
// pool and SQLite WAL. Running many of those daemons inside one test process
// can starve their pump tasks on constrained CI hosts, even though production
// runs one scheduler per database. Serialize this integration harness while
// leaving the lightweight protocol/server unit tests parallel.
static RUNTIME_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn runtime_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    RUNTIME_TEST_LOCK.lock().await
}

#[derive(Default)]
struct FakeBackend {
    executions: Mutex<Vec<ExecutionRequest>>,
    controls: Mutex<Vec<(String, ExecutionControl)>>,
    outcomes: Mutex<VecDeque<ExecutionOutcome>>,
}

impl FakeBackend {
    fn with_outcomes(outcomes: impl IntoIterator<Item = ExecutionOutcome>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes.into_iter().collect()),
            ..Self::default()
        }
    }

    fn execution_count(&self) -> usize {
        self.executions.lock().unwrap().len()
    }
}

#[async_trait]
impl ExecutionBackend for FakeBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        self.executions.lock().unwrap().push(request);
        self.outcomes
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| ExecutionOutcome::Succeeded {
                output: serde_json::json!({"ok": true}),
            })
    }

    async fn control(&self, session_id: &str, control: ExecutionControl) -> anyhow::Result<()> {
        self.controls
            .lock()
            .unwrap()
            .push((session_id.to_string(), control));
        Ok(())
    }
}

#[derive(Default)]
struct BlockingBackend {
    executions: AtomicUsize,
    execution_dropped: AtomicBool,
    controls: Mutex<Vec<(String, ExecutionControl)>>,
}

struct ExecutionDropFlag<'a>(&'a AtomicBool);

impl Drop for ExecutionDropFlag<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct CancellableEventBackend {
    executions: AtomicUsize,
    cancel: Notify,
}

#[derive(Default)]
struct JournalExhaustionBackend {
    executions: AtomicUsize,
    controls: Mutex<Vec<(String, ExecutionControl)>>,
}

#[derive(Default)]
struct EventBackend;

#[derive(Default)]
struct PrivacyBoundaryBackend {
    executions: AtomicUsize,
}

#[derive(Default)]
struct DroppedProducerBackend {
    executions: AtomicUsize,
}

struct LateSteerBackend {
    database_path: PathBuf,
    executions: AtomicUsize,
    release_first: Notify,
}

struct AwaitingResponseBoundaryBackend {
    database_path: PathBuf,
    executions: AtomicUsize,
    release_first: Notify,
}

#[derive(Default)]
struct FlakyApprovalBackend {
    executions: AtomicUsize,
    approval_attempts: AtomicUsize,
    delivered: AtomicBool,
}

impl LateSteerBackend {
    fn new(database_path: PathBuf) -> Self {
        Self {
            database_path,
            executions: AtomicUsize::new(0),
            release_first: Notify::new(),
        }
    }
}

impl AwaitingResponseBoundaryBackend {
    fn new(database_path: PathBuf) -> Self {
        Self {
            database_path,
            executions: AtomicUsize::new(0),
            release_first: Notify::new(),
        }
    }
}

#[async_trait]
impl ExecutionBackend for LateSteerBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        let execution = self.executions.fetch_add(1, Ordering::SeqCst);
        if execution == 0 {
            self.release_first.notified().await;
        } else if let Some(session_id) = request.claim.run.session_id.as_deref() {
            SessionManager::new(Database::new(&self.database_path).unwrap())
                .promote_orphaned_pending_steering(session_id)
                .unwrap();
        }
        ExecutionOutcome::Succeeded {
            output: serde_json::json!({"execution": execution}),
        }
    }

    async fn control(&self, _session_id: &str, _control: ExecutionControl) -> anyhow::Result<()> {
        // Model an input channel that accepted the steer while the agent was
        // already crossing its terminal boundary. The durable staging row is
        // the source of truth, not this best-effort acknowledgement.
        Ok(())
    }
}

#[async_trait]
impl ExecutionBackend for AwaitingResponseBoundaryBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        let execution = self.executions.fetch_add(1, Ordering::SeqCst);
        if execution == 0 {
            request
                .events
                .agentic(serde_json::json!({
                    "type": "awaiting_input",
                    "tool_call_id": "question-1",
                }))
                .await
                .expect("awaiting-input event should be durable");
            // Hold the first attempt after it has entered the active state.
            // The response control acknowledges delivery, then this attempt
            // models AskUser crossing its terminal boundary without consuming
            // the input.
            self.release_first.notified().await;
            ExecutionOutcome::AwaitingInput {
                details: serde_json::json!({"tool_call_id": "question-1"}),
            }
        } else {
            if let Some(session_id) = request.claim.run.session_id.as_deref() {
                SessionManager::new(Database::new(&self.database_path).unwrap())
                    .promote_orphaned_pending_steering(session_id)
                    .unwrap();
            }
            ExecutionOutcome::Succeeded {
                output: serde_json::json!({"execution": execution}),
            }
        }
    }

    async fn control(&self, _session_id: &str, control: ExecutionControl) -> anyhow::Result<()> {
        if matches!(
            control,
            ExecutionControl::Steer { .. } | ExecutionControl::UserResponse { .. }
        ) {
            self.release_first.notify_one();
        }
        Ok(())
    }
}

#[async_trait]
impl ExecutionBackend for FlakyApprovalBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        self.executions.fetch_add(1, Ordering::SeqCst);
        request
            .events
            .agentic(serde_json::json!({
                "type": "tool_approval_required",
                "id": "tool-1",
            }))
            .await
            .expect("approval event should be durable");
        std::future::pending().await
    }

    async fn control(&self, _session_id: &str, control: ExecutionControl) -> anyhow::Result<()> {
        if matches!(control, ExecutionControl::ToolApproval { .. }) {
            let attempt = self.approval_attempts.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                anyhow::bail!("simulated host registration race");
            }
            self.delivered.store(true, Ordering::SeqCst);
        }
        Ok(())
    }
}

#[async_trait]
impl ExecutionBackend for EventBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        for index in 0..6 {
            request
                .events
                .agentic(serde_json::json!({
                    "type": "tick_injected",
                    "tick_number": index,
                }))
                .await
                .expect("bounded execution event should be accepted");
        }
        request
            .events
            .agentic(serde_json::json!({"body": "x".repeat(2_048)}))
            .await
            .expect("oversized live event must be replaced by a bounded summary");
        ExecutionOutcome::Succeeded {
            output: serde_json::json!({"ok": true}),
        }
    }

    async fn control(&self, _session_id: &str, _control: ExecutionControl) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ExecutionBackend for PrivacyBoundaryBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        const SENTINEL: &str = "HIVE_PRIVATE_SENTINEL_91f6";
        for payload in [
            serde_json::json!({
                "type": "thinking_complete",
                "thinking": SENTINEL,
                "signature": SENTINEL,
            }),
            serde_json::json!({
                "type": "tool_call_complete",
                "id": "tool-private",
                "name": "bash",
                "arguments": {"command": SENTINEL},
            }),
            serde_json::json!({
                "type": "tool_output_delta",
                "id": "tool-private",
                "delta": SENTINEL,
            }),
            serde_json::json!({
                "type": "web_fetch_result",
                "url": "https://example.invalid/private",
                "body": SENTINEL,
            }),
        ] {
            request.events.agentic(payload).await.unwrap();
        }
        request
            .events
            .agentic(serde_json::json!({
                "type": "tool_result",
                "id": "tool-private",
                "output": SENTINEL,
                "is_error": false,
            }))
            .await
            .unwrap();
        if self.executions.fetch_add(1, Ordering::SeqCst) == 0 {
            ExecutionOutcome::Failed {
                error: format!("provider failure containing {SENTINEL}"),
                retryable: false,
                retry_after: None,
            }
        } else {
            ExecutionOutcome::Succeeded {
                output: serde_json::json!({"private": SENTINEL}),
            }
        }
    }

    async fn control(&self, _session_id: &str, _control: ExecutionControl) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ExecutionBackend for DroppedProducerBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        self.executions.fetch_add(1, Ordering::SeqCst);
        request
            .events
            .agentic(serde_json::json!({
                "type": "tool_result",
                "id": "tool-with-uncertain-side-effect",
                "output": "producer disappeared after the external write",
                "is_error": false,
            }))
            .await
            .expect("tool event should enter the bounded scheduler sink");
        ExecutionOutcome::RecoveryRequired {
            reason: "agent event producer disappeared before Finished; side effects are uncertain"
                .into(),
        }
    }

    async fn control(&self, _session_id: &str, _control: ExecutionControl) -> anyhow::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl ExecutionBackend for BlockingBackend {
    async fn execute(&self, _request: ExecutionRequest) -> ExecutionOutcome {
        self.executions.fetch_add(1, Ordering::SeqCst);
        let _drop_flag = ExecutionDropFlag(&self.execution_dropped);
        std::future::pending().await
    }

    async fn control(&self, session_id: &str, control: ExecutionControl) -> anyhow::Result<()> {
        self.controls
            .lock()
            .unwrap()
            .push((session_id.to_string(), control));
        Ok(())
    }
}

#[async_trait]
impl ExecutionBackend for CancellableEventBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        self.executions.fetch_add(1, Ordering::SeqCst);
        self.cancel.notified().await;
        request
            .events
            .agentic(serde_json::json!({
                "type": "finish",
                "session_id": request.claim.run.session_id.as_deref(),
                "stop_reason": "user_abort",
            }))
            .await
            .expect("the current cancelled run may persist its terminal event");
        // Deliberately model a backend that acknowledged cancellation but
        // raced back success. The durable CancelSession commit must win.
        ExecutionOutcome::Succeeded {
            output: serde_json::json!({"ignored_cancel": true}),
        }
    }

    async fn control(&self, _session_id: &str, control: ExecutionControl) -> anyhow::Result<()> {
        if matches!(control, ExecutionControl::Cancel { .. }) {
            self.cancel.notify_one();
        }
        Ok(())
    }
}

#[async_trait]
impl ExecutionBackend for JournalExhaustionBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        self.executions.fetch_add(1, Ordering::SeqCst);
        request
            .events
            .emit(ExecutionEvent {
                event_type: "agentic_event".into(),
                payload: serde_json::json!({"type": "finish"}),
                // This fits the execution channel's configured byte bound but
                // deliberately exceeds the tighter durable journal contract
                // after allow-list sanitization.
                durable_payload: Some(serde_json::json!({
                    "type": "finish",
                    "session_id": "s".repeat(16 * 1024),
                })),
            })
            .await
            .expect("the event should reach the scheduler persistence boundary");
        std::future::pending().await
    }

    async fn control(&self, session_id: &str, control: ExecutionControl) -> anyhow::Result<()> {
        self.controls
            .lock()
            .unwrap()
            .push((session_id.to_string(), control));
        Ok(())
    }
}

fn config(temp: &TempDir) -> HiveRuntimeConfig {
    let mut config = HiveRuntimeConfig::for_database(temp.path().join("runtime.db"));
    config.scheduler_poll_interval = Duration::from_millis(20);
    // Keep the shared integration-test leases short enough for fast feedback,
    // but long enough that normal CI scheduling and SQLite retention work do
    // not impersonate a crashed daemon. Tests that exercise real expiry use
    // explicit, tighter durations below.
    config.daemon_lease_duration = Duration::from_secs(2);
    config.worker_lease_duration = Duration::from_secs(2);
    config.worker_heartbeat_interval = Duration::from_millis(100);
    config.cancellation_grace_period = Duration::from_millis(200);
    config.live_event_capacity = 8;
    config.subscriber_capacity = 8;
    config
}

fn context(actor: Actor, key: &str) -> CommandContext {
    CommandContext {
        request_id: format!("request-{key}"),
        idempotency_key: key.to_string(),
        actor,
        deadline_unix_ms: i64::MAX,
        peer: PeerIdentity {
            uid: 1,
            gid: 1,
            pid: None,
        },
        daemon_instance_id: "daemon-test".into(),
    }
}

fn dispatch_command() -> Command {
    Command::Dispatch(DispatchCommand {
        task: "Audit the repository".into(),
        working_dir: "/work/repo".into(),
        project_dir: Some("/work/repo".into()),
        model: Some("test:model".into()),
        model_key: Some(ModelKey {
            provider: "grok".into(),
            model_id: "test:model".into(),
            auth_scope: Some("oauth".into()),
            api_format: "open_ai_responses".into(),
        }),
        model_catalog_revision: Some("catalog-42".into()),
        start_at_unix_ms: None,
        priority: Some("normal".into()),
        crew_slug: None,
    })
}

async fn response(
    handler: &dyn CommandHandler,
    context: CommandContext,
    command: Command,
) -> ResponsePayload {
    match handler.handle(context, command).await.unwrap() {
        HandlerReply::Response(response) => response,
        HandlerReply::Subscription { .. } => panic!("expected response"),
    }
}

fn dispatch_session_id(response: &ResponsePayload) -> String {
    match response {
        ResponsePayload::Dispatch(response) => response.session_id.clone(),
        response => panic!("expected dispatch response, got {response:?}"),
    }
}

async fn wait_for(condition: impl Fn() -> bool) {
    // The runtime polls every 20ms in tests, but a full workspace test run can
    // initialize many isolated SQLite databases concurrently. Keep this bound
    // finite while allowing slow CI hosts to make durable scheduler progress.
    tokio::time::timeout(Duration::from_secs(30), async {
        while !condition() {
            // Most predicates reopen SQLite. Poll slowly enough that the
            // observation connection cannot starve the writer transaction it
            // is waiting for when tests run in parallel.
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
}

async fn wait_for_runtime_health(
    handler: &DurableHiveCommandHandler,
    pump_alive: bool,
    scheduler_ready: bool,
) {
    tokio::time::timeout(Duration::from_secs(10), async {
        let actor = Actor::local("health-test");
        loop {
            let stats = handler.runtime_stats(&actor).await;
            if stats.pump_alive == pump_alive && stats.scheduler_ready == scheduler_ready {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("runtime health did not reach the expected state");
}

#[tokio::test]
async fn foreign_scheduler_lease_keeps_runtime_not_ready_until_takeover() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let lease =
        match HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
            .acquire(
                "hive-scheduler",
                "foreign-daemon",
                chrono::Utc::now(),
                Duration::from_secs(30),
            )
            .unwrap()
        {
            DaemonLeaseAcquire::Acquired(lease) => lease,
            other => panic!("expected foreign lease acquisition, got {other:?}"),
        };

    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-b",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    wait_for_runtime_health(handler.as_ref(), true, false).await;

    assert!(
        HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
            .release("hive-scheduler", "foreign-daemon", lease.fencing_token,)
            .unwrap()
    );
    wait_for_runtime_health(handler.as_ref(), true, true).await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn forced_pump_exit_marks_health_down_and_surfaces_supervision_failure() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let mut runtime = start_runtime(config(&temp), "daemon-a", Arc::new(FakeBackend::default()))
        .await
        .unwrap();
    let handler = runtime.handler();
    wait_for_runtime_health(handler.as_ref(), true, true).await;

    runtime.force_pump_exit_for_test();
    wait_for_runtime_health(handler.as_ref(), false, false).await;
    let failure =
        tokio::time::timeout(Duration::from_secs(1), runtime.wait_for_scheduler_failure())
            .await
            .expect("scheduler supervisor did not observe the forced exit");
    assert!(failure.to_string().contains("scheduler pump stopped"));
}

#[tokio::test]
async fn generic_wake_is_ambiguous_but_user_response_resumes_only_its_exact_run() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let foreign =
        match HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
            .acquire(
                "hive-scheduler",
                "foreign-daemon",
                chrono::Utc::now(),
                Duration::from_secs(30),
            )
            .unwrap()
        {
            DaemonLeaseAcquire::Acquired(lease) => lease,
            other => panic!("expected foreign lease, got {other:?}"),
        };
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-b",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "two-waiting-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    let db = Database::new(&runtime_config.database_path).unwrap();
    let (controller_id, run_a): (String, String) = db
        .conn()
        .query_row(
            "SELECT controller_id, id FROM hive_runs WHERE session_id = ?1",
            [&session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let now = canonical_timestamp(chrono::Utc::now());
    db.conn()
        .execute(
            "UPDATE hive_runs SET status = 'awaiting_input', updated_at = ?2 WHERE id = ?1",
            rusqlite::params![run_a, now],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO hive_runs (
                id, controller_id, session_id, kind, objective, config_json,
                status, available_at, max_attempts, created_at, updated_at
             ) VALUES ('run-b', ?1, ?2, 'dispatch', 'second waiting run',
                       '{\"model\":\"test:model\",\"permission_mode\":\"autonomous\"}',
                       'awaiting_input', ?3, 5, ?3, ?3)",
            rusqlite::params![controller_id, session_id, now],
        )
        .unwrap();
    let run_b = "run-b".to_string();
    for (run_id, tool_call_id) in [(&run_a, "question-a"), (&run_b, "question-b")] {
        let sequence = db
            .conn()
            .query_row(
                "SELECT COALESCE(MAX(sequence), 0) + 1 FROM hive_controller_events
                 WHERE controller_id = ?1",
                [&controller_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO hive_controller_events (
                    controller_id, sequence, event_type, run_id, payload_json, created_at
                 ) VALUES (?1, ?2, 'agentic_event', ?3, ?4, ?5)",
                rusqlite::params![
                    controller_id,
                    sequence,
                    run_id,
                    serde_json::json!({
                        "type": "awaiting_input",
                        "tool_call_id": tool_call_id,
                    })
                    .to_string(),
                    now,
                ],
            )
            .unwrap();
    }
    drop(db);

    let ambiguous = handler
        .handle(
            context(Actor::local("test"), "ambiguous-message"),
            Command::SendMessage(MessageCommand {
                session_id: session_id.clone(),
                message: "MESSAGE_PRIVATE_SENTINEL".into(),
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(ambiguous.code, "state_conflict");

    response(
        handler.as_ref(),
        context(Actor::local("test"), "exact-response"),
        Command::UserResponse(UserResponseCommand {
            session_id: session_id.clone(),
            run_id: run_a.clone(),
            tool_call_id: "question-a".into(),
            response: "RESPONSE_PRIVATE_SENTINEL".into(),
        }),
    )
    .await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "private-extension"),
        Command::Extension(ExtensionCommand {
            name: "privacy-test".into(),
            payload: serde_json::json!({
                "session_id": session_id,
                "secret": "EXTENSION_PRIVATE_SENTINEL",
            }),
        }),
    )
    .await;

    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE id = ?1",
                [&run_a],
                |row| { row.get::<_, String>(0) }
            )
            .unwrap(),
        "queued"
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE id = 'run-b'",
                [],
                |row| { row.get::<_, String>(0) }
            )
            .unwrap(),
        "awaiting_input"
    );
    let event_leaks: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_controller_events
             WHERE payload_json LIKE '%MESSAGE_PRIVATE_SENTINEL%'
                OR payload_json LIKE '%RESPONSE_PRIVATE_SENTINEL%'
                OR payload_json LIKE '%EXTENSION_PRIVATE_SENTINEL%'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(event_leaks, 0);
    drop(db);

    HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
        .release("hive-scheduler", "foreign-daemon", foreign.fencing_token)
        .unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn graceful_runtime_shutdown_releases_lease_without_supervision_failure() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-a",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    wait_for_runtime_health(handler.as_ref(), true, true).await;
    let owner = handler.shared.instance_id.clone();

    runtime.shutdown().await;

    let lease = HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
        .get("hive-scheduler")
        .unwrap()
        .expect("scheduler lease row should remain as a fencing record");
    assert_eq!(lease.owner_id, owner);
    assert!(lease.expires_at <= canonical_timestamp(chrono::Utc::now()));
}

#[tokio::test]
async fn duplicate_dispatch_replays_without_creating_a_second_run() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let runtime = start_runtime(config(&temp), "daemon-a", backend)
        .await
        .unwrap();
    let handler = runtime.handler();
    let first = response(
        handler.as_ref(),
        context(Actor::local("test"), "same-key"),
        dispatch_command(),
    )
    .await;
    let replay = response(
        handler.as_ref(),
        context(Actor::local("test"), "same-key"),
        dispatch_command(),
    )
    .await;
    assert_eq!(first, replay);

    let db = Database::new(&config(&temp).database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row("SELECT COUNT(*) FROM hive_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn recurring_schedule_inherits_frozen_session_config_and_rejects_stale_revision() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-a",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "schedule-parent"),
            dispatch_command(),
        )
        .await,
    );
    Database::new(&runtime_config.database_path)
        .unwrap()
        .conn()
        .execute(
            "UPDATE hive_runtime_state SET crew_slug = 'ops' WHERE session_id = ?1",
            [&session_id],
        )
        .unwrap();

    let definition = ScheduleDefinition {
        title: "Daily repository audit".into(),
        summary: "Verify the backend remains healthy".into(),
        objective: "Audit the repository and report durable findings".into(),
        recurrence: serde_json::to_value(RecurrenceV1::Once {
            at: chrono::Utc::now() + chrono::Duration::days(1),
        })
        .unwrap(),
        timezone: "UTC".into(),
        dst_policy: serde_json::to_value(DstPolicy::default()).unwrap(),
        priority: 0,
        project_dir: None,
        model: None,
        model_key: None,
        model_catalog_revision: None,
        crew_slug: None,
        misfire: serde_json::to_value(MisfireConfig::default()).unwrap(),
        overlap_policy: "queue_one".into(),
        retry: serde_json::to_value(RetryPolicy::default()).unwrap(),
    };
    let mut relative_definition = definition.clone();
    relative_definition.project_dir = Some("relative/workspace".into());
    let relative_error = handler
        .handle(
            context(Actor::local("test"), "relative-recurring-schedule"),
            Command::CreateSchedule(CreateScheduleCommand {
                session_id: session_id.clone(),
                definition: relative_definition,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(relative_error.code, "invalid_command");
    let mut unsafe_retry_definition = definition.clone();
    unsafe_retry_definition.retry = serde_json::json!({
        "max_attempts": 101,
        "base_delay_secs": 0,
        "max_delay_secs": 604_801,
        "jitter": "full",
    });
    let unsafe_retry_error = handler
        .handle(
            context(Actor::local("test"), "unsafe-retry-schedule"),
            Command::CreateSchedule(CreateScheduleCommand {
                session_id: session_id.clone(),
                definition: unsafe_retry_definition,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(unsafe_retry_error.code, "invalid_command");
    let created = response(
        handler.as_ref(),
        context(Actor::local("test"), "create-recurring-schedule"),
        Command::CreateSchedule(CreateScheduleCommand {
            session_id: session_id.clone(),
            definition: definition.clone(),
        }),
    )
    .await;
    let (schedule_id, revision) = match created {
        ResponsePayload::Schedule(response) => (response.schedule_id, response.revision),
        other => panic!("expected schedule response, got {other:?}"),
    };
    assert_eq!(revision, 0);

    let schedule = HiveScheduleStore::new(Database::new(&runtime_config.database_path).unwrap())
        .get_schedule(&schedule_id)
        .unwrap()
        .unwrap();
    assert_eq!(schedule.project_dir.as_deref(), Some("/work/repo"));
    assert_eq!(schedule.model.as_deref(), Some("test:model"));
    assert_eq!(
        schedule.model_key.as_ref().map(|key| key.provider),
        Some(ProviderId::Grok)
    );
    assert_eq!(
        schedule.model_catalog_revision.as_deref(),
        Some("catalog-42")
    );
    assert_eq!(schedule.crew_slug.as_deref(), Some("ops"));

    let error = handler
        .handle(
            context(Actor::local("test"), "replace-stale-schedule"),
            Command::ReplaceSchedule(ReplaceScheduleCommand {
                session_id,
                schedule_id,
                expected_revision: revision + 1,
                definition,
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "revision_conflict");
    runtime.shutdown().await;
}

#[tokio::test]
async fn daemon_authority_rejects_oversized_or_ambiguous_inputs() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime = start_runtime(config(&temp), "daemon-a", Arc::new(FakeBackend::default()))
        .await
        .unwrap();
    let handler = runtime.handler();
    let mut command = match dispatch_command() {
        Command::Dispatch(command) => command,
        _ => unreachable!(),
    };
    command.task = "x".repeat(64 * 1024 + 1);
    let error = handler
        .handle(
            context(Actor::local("test"), "oversized-dispatch"),
            Command::Dispatch(command),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_command");

    let mut relative_working_dir = match dispatch_command() {
        Command::Dispatch(command) => command,
        _ => unreachable!(),
    };
    relative_working_dir.working_dir = "relative/workspace".into();
    relative_working_dir.project_dir = None;
    let error = handler
        .handle(
            context(Actor::local("test"), "relative-working-directory"),
            Command::Dispatch(relative_working_dir),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_command");

    let mut relative_project_dir = match dispatch_command() {
        Command::Dispatch(command) => command,
        _ => unreachable!(),
    };
    relative_project_dir.project_dir = Some("relative/project".into());
    let error = handler
        .handle(
            context(Actor::local("test"), "relative-project-directory"),
            Command::Dispatch(relative_project_dir),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "invalid_command");

    for (key, command) in [
        (
            "oversized-user-response",
            Command::UserResponse(UserResponseCommand {
                session_id: "missing-session".into(),
                run_id: "run-1".into(),
                tool_call_id: "question-1".into(),
                response: "x".repeat(64 * 1024 + 1),
            }),
        ),
        (
            "oversized-pending-id",
            Command::Steer(SteerCommand {
                session_id: "missing-session".into(),
                pending_id: Some("x".repeat(257)),
                content: serde_json::json!([{"type": "text", "text": "continue"}]),
            }),
        ),
        (
            "oversized-extension-payload",
            Command::Extension(ExtensionCommand {
                name: "test.extension".into(),
                payload: serde_json::json!({
                    "session_id": "missing-session",
                    "content": "x".repeat(256 * 1024 + 1),
                }),
            }),
        ),
    ] {
        let error = handler
            .handle(context(Actor::local("test"), key), command)
            .await
            .unwrap_err();
        assert_eq!(error.code, "invalid_command");
    }
    runtime.shutdown().await;
}

#[tokio::test]
async fn message_after_completion_queues_exactly_one_idempotent_followup_run() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let backend = Arc::new(FakeBackend::default());
    let runtime = start_runtime(config(&temp), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "first-turn"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.execution_count() == 1).await;
    wait_for(|| {
        let db = Database::new(&config(&temp).database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE status = 'succeeded'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    let followup = Command::SendMessage(MessageCommand {
        session_id: session_id.clone(),
        message: "Now do the second turn".into(),
    });
    response(
        handler.as_ref(),
        context(Actor::local("test"), "second-turn"),
        followup.clone(),
    )
    .await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "second-turn"),
        followup,
    )
    .await;
    wait_for(|| backend.execution_count() == 2).await;
    let db = Database::new(&config(&temp).database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM sessions
                 WHERE id = ?1
                   AND json_extract(model_key_json, '$.provider') = 'grok'
                   AND model_catalog_revision = 'catalog-42'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1,
        "dispatch must freeze exact identity on the session",
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs
                 WHERE json_extract(config_json, '$.model') = 'test:model'
                   AND json_extract(config_json, '$.model_key.provider') = 'grok'
                   AND json_extract(config_json, '$.model_catalog_revision') = 'catalog-42'
                   AND json_extract(config_json, '$.permission_mode') = 'autonomous'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2,
        "dispatch and message follow-up must freeze model and permission mode",
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = 'user'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn every_session_command_reloads_and_enforces_exact_actor_ownership() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime = start_runtime(config(&temp), "daemon-a", Arc::new(FakeBackend::default()))
        .await
        .unwrap();
    let db = Database::new(&config(&temp).database_path).unwrap();
    db.conn()
        .execute_batch(
            "INSERT INTO users (id, email) VALUES ('alice', 'alice@example.com');
             INSERT INTO users (id, email) VALUES ('bob', 'bob@example.com');",
        )
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(
                Actor {
                    user_id: Some("alice".into()),
                    client_kind: "test".into(),
                },
                "alice-dispatch",
            ),
            dispatch_command(),
        )
        .await,
    );
    let error = handler
        .handle(
            context(
                Actor {
                    user_id: Some("bob".into()),
                    client_kind: "test".into(),
                },
                "bob-pause",
            ),
            Command::PauseSession(SessionCommand { session_id }),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "ownership_denied");

    let sessions = SessionManager::new(Database::new(&config(&temp).database_path).unwrap());
    let code_session_id = sessions
        .create_session_for_user_with_config(
            "Code session",
            Some("test:model"),
            Some("/work/repo"),
            Some("/work/repo"),
            WorkspaceMode::Selected,
            None,
            None,
            SessionType::Code,
        )
        .unwrap();
    for (key, command) in [
        (
            "reject-code-start",
            Command::StartSession(SessionCommand {
                session_id: code_session_id.clone(),
            }),
        ),
        (
            "reject-code-delete",
            Command::DeleteSession(SessionCommand {
                session_id: code_session_id.clone(),
            }),
        ),
    ] {
        let error = handler
            .handle(context(Actor::local("test"), key), command)
            .await
            .unwrap_err();
        assert_eq!(error.code, "ownership_denied");
    }
    assert!(sessions.get_session(&code_session_id).unwrap().is_some());
    assert_eq!(
        Database::new(&config(&temp).database_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controllers WHERE session_id = ?1",
                [&code_session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn dropped_producer_after_tool_event_is_recovery_required_and_unclaimable() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let backend = Arc::new(DroppedProducerBackend::default());
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "uncertain-tool-dispatch"),
            dispatch_command(),
        )
        .await,
    );

    wait_for(|| {
        Database::new(&runtime_config.database_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, String>(0),
            )
            .is_ok_and(|status| status == "recovery_required")
    })
    .await;
    assert_eq!(backend.executions.load(Ordering::SeqCst), 1);

    let claim = HiveRunStore::new(Database::new(&runtime_config.database_path).unwrap())
        .claim_next(&ClaimRunRequest {
            executor_id: "replacement-executor".into(),
            lease_epoch: 999,
            now: chrono::Utc::now(),
            lease_duration: Duration::from_secs(10),
            global_concurrency_limit: 8,
        })
        .unwrap();
    assert!(
        claim.is_none(),
        "recovery-required work must not auto-replay"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn queue_claim_execution_and_once_schedule_survive_the_process_boundary() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let backend = Arc::new(FakeBackend::with_outcomes([
        ExecutionOutcome::Succeeded {
            output: serde_json::json!({"dispatch": true}),
        },
        ExecutionOutcome::Succeeded {
            output: serde_json::json!({"scheduled": true}),
        },
    ]));
    let runtime = start_runtime(config(&temp), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.execution_count() >= 1).await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "schedule"),
        Command::ScheduleSession(ScheduleCommand {
            session_id: session_id.clone(),
            wake_at_unix_ms: chrono::Utc::now().timestamp_millis(),
            reason: "Follow-up health check".into(),
        }),
    )
    .await;
    wait_for(|| backend.execution_count() >= 2).await;
    wait_for(|| {
        let db = Database::new(&config(&temp).database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE status = 'succeeded'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 2
    })
    .await;

    let db = Database::new(&config(&temp).database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE status = 'succeeded'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
    assert_eq!(
        db.conn()
            .query_row("SELECT status FROM hive_schedules LIMIT 1", [], |row| {
                row.get::<_, String>(0)
            },)
            .unwrap(),
        "completed"
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs
                 WHERE json_extract(config_json, '$.model') = 'test:model'
                   AND json_extract(config_json, '$.model_key.provider') = 'grok'
                   AND json_extract(config_json, '$.model_catalog_revision') = 'catalog-42'
                   AND json_extract(config_json, '$.permission_mode') = 'autonomous'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2,
        "dispatch and one-shot materialization must freeze model and permission mode",
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn restart_marks_expired_running_work_recovery_required_without_replay() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let db = Database::new(&runtime_config.database_path).unwrap();
    let now = canonical_timestamp(chrono::Utc::now());
    let expired = canonical_timestamp(chrono::Utc::now() - chrono::Duration::seconds(1));
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO sessions (id, title, created_at, updated_at, working_dir, project_dir, model, session_type)
             VALUES ('session-1', 'Hive', '{now}', '{now}', '/work', '/work', 'test:model', 'hive');
             INSERT INTO hive_controllers (
                 id, scope_key, session_id, status, timezone, max_concurrent_runs, created_at, updated_at
             ) VALUES ('controller-1', 'session:session-1', 'session-1', 'active', 'UTC', 1, '{now}', '{now}');
             INSERT INTO hive_runs (
                 id, controller_id, session_id, kind, objective, config_json, status,
                 priority, available_at, attempt_count, max_attempts, lease_owner,
                 lease_token, lease_epoch, lease_expires_at, heartbeat_at, created_at, updated_at
             ) VALUES ('run-1', 'controller-1', 'session-1', 'dispatch', 'work', '{{}}',
                 'running', 0, '{now}', 1, 3, 'old-daemon', 'old-token', 1, '{expired}',
                 '{expired}', '{now}', '{now}');
             INSERT INTO hive_run_attempts (
                 id, run_id, attempt_no, executor_id, lease_token, lease_epoch, started_at, outcome
             ) VALUES ('attempt-1', 'run-1', 1, 'old-daemon', 'old-token', 1, '{now}', 'leased');"
        ))
        .unwrap();
    drop(db);

    let backend = Arc::new(FakeBackend::default());
    let runtime = start_runtime(runtime_config.clone(), "daemon-new", backend.clone())
        .await
        .unwrap();
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE id = 'run-1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
            == "recovery_required"
    })
    .await;
    assert_eq!(backend.execution_count(), 0);

    let handler = runtime.handler();
    for (key, command) in [
        (
            "blocked-start",
            Command::StartSession(SessionCommand {
                session_id: "session-1".into(),
            }),
        ),
        (
            "blocked-resume",
            Command::ResumeSession(SessionCommand {
                session_id: "session-1".into(),
            }),
        ),
    ] {
        let blocked = response(
            handler.as_ref(),
            context(Actor::local("test"), key),
            command,
        )
        .await;
        assert!(matches!(
            blocked,
            ResponsePayload::Session(ref response)
                if response.state["status"] == "recovery_required"
        ));
    }
    let db = Database::new(&runtime_config.database_path).unwrap();
    let (runs, run_status, runtime_status, controller_status): (i64, String, String, String) = db
        .conn()
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM hive_runs WHERE session_id = 'session-1'),
                 (SELECT status FROM hive_runs WHERE id = 'run-1'),
                 (SELECT status FROM hive_runtime_state WHERE session_id = 'session-1'),
                 (SELECT status FROM hive_controllers WHERE id = 'controller-1')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(runs, 1);
    assert_eq!(run_status, "recovery_required");
    assert_eq!(runtime_status, "error");
    assert_eq!(controller_status, "paused");
    drop(db);

    // Cancellation is the explicit abandon decision. Only after that durable
    // resolution may StartSession create fresh work for the session.
    response(
        handler.as_ref(),
        context(Actor::local("test"), "abandon-uncertain-run"),
        Command::CancelSession(SessionCommand {
            session_id: "session-1".into(),
        }),
    )
    .await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "start-after-abandon"),
        Command::StartSession(SessionCommand {
            session_id: "session-1".into(),
        }),
    )
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE session_id = 'session-1'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE id = 'run-1'",
                [],
                |row| { row.get::<_, String>(0) }
            )
            .unwrap(),
        "cancelled"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn takeover_reconciles_a_worker_lease_that_expires_after_acquisition() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let mut runtime_config = config(&temp);
    // Shutdown releases the daemon lease, so the replacement still acquires
    // immediately. Keep the worker lease longer than ordinary CI scheduling
    // jitter while leaving ample room for it to expire inside `wait_for`.
    runtime_config.daemon_lease_duration = Duration::from_secs(2);
    runtime_config.worker_lease_duration = Duration::from_secs(1);
    runtime_config.worker_heartbeat_interval = Duration::from_millis(100);

    let first_backend = Arc::new(BlockingBackend::default());
    let first = start_runtime(runtime_config.clone(), "daemon-a", first_backend.clone())
        .await
        .unwrap();
    let handler = first.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "late-expiry-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| first_backend.executions.load(Ordering::SeqCst) == 1).await;
    first.shutdown().await;

    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "running"
    );
    drop(db);

    let second = start_runtime(
        runtime_config.clone(),
        "daemon-b",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    // Acquisition happens immediately, before the 500ms worker lease has
    // expired. Periodic reconciliation must catch it on a later tick.
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
            == "recovery_required"
    })
    .await;
    second.shutdown().await;
}

#[tokio::test]
async fn replay_and_live_events_remain_monotonic_and_report_a_retention_gap() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime = start_runtime(config(&temp), "daemon-a", Arc::new(FakeBackend::default()))
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "dispatch"),
            dispatch_command(),
        )
        .await,
    );
    response(
        handler.as_ref(),
        context(Actor::local("test"), "pause"),
        Command::PauseSession(SessionCommand {
            session_id: session_id.clone(),
        }),
    )
    .await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "priority"),
        Command::SetPriority(SetPriorityCommand {
            session_id: session_id.clone(),
            priority: "high".into(),
        }),
    )
    .await;
    let db = Database::new(&config(&temp).database_path).unwrap();
    db.conn()
        .execute(
            "DELETE FROM hive_controller_events
             WHERE sequence = (SELECT MIN(sequence) FROM hive_controller_events)",
            [],
        )
        .unwrap();
    drop(db);

    let reply = handler
        .handle(
            context(Actor::local("test"), "subscribe"),
            Command::Subscribe(SubscribeCommand {
                session_id,
                after_sequence: Some(0),
                replay_limit: Some(100),
            }),
        )
        .await
        .unwrap();
    let HandlerReply::Subscription {
        accepted,
        mut events,
    } = reply
    else {
        panic!("expected subscription");
    };
    assert!(accepted.high_water_sequence.is_some());
    assert!(matches!(
        events.recv().await.unwrap().event,
        HiveEvent::ReplayGap(_)
    ));
    let mut previous = 0;
    while let Ok(Some(event)) =
        tokio::time::timeout(Duration::from_millis(100), events.recv()).await
    {
        if let Some(sequence) = event.sequence {
            assert!(sequence > previous);
            previous = sequence;
        }
        if previous == accepted.high_water_sequence.unwrap_or_default() {
            break;
        }
    }
    runtime.shutdown().await;
}

#[tokio::test]
async fn active_message_is_canonical_exactly_once_and_pause_fences_execution() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let backend = Arc::new(BlockingBackend::default());
    let runtime = start_runtime(config(&temp), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "blocking-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.executions.load(Ordering::SeqCst) == 1).await;
    let message_command = Command::SendMessage(MessageCommand {
        session_id: session_id.clone(),
        message: "Remember this exact message".into(),
    });
    response(
        handler.as_ref(),
        context(Actor::local("test"), "one-message"),
        message_command.clone(),
    )
    .await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "one-message"),
        message_command,
    )
    .await;

    let db = Database::new(&config(&temp).database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role = 'user'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE session_id = ?1 AND role LIKE 'pending_user:%'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM conversation_episodes WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    let latest_content = db
        .conn()
        .query_row(
            "SELECT content FROM messages WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
            [&session_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Vec<mitsuro_core::Content>>(&latest_content)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    drop(db);
    let message_controls = backend
        .controls
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, control)| {
            matches!(
                control,
                ExecutionControl::Steer {
                    pending_id: Some(_),
                    ..
                }
            )
        })
        .count();
    assert_eq!(message_controls, 1);

    response(
        handler.as_ref(),
        context(Actor::local("test"), "pause-active"),
        Command::PauseSession(SessionCommand {
            session_id: session_id.clone(),
        }),
    )
    .await;
    let db = Database::new(&config(&temp).database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "recovery_required"
    );
    assert!(backend.controls.lock().unwrap().iter().any(|(_, control)| {
        matches!(control, ExecutionControl::Cancel { reason } if reason == "paused by user")
    }));
    runtime.shutdown().await;
}

#[tokio::test]
async fn steering_missed_at_terminal_boundary_is_resumed_and_promoted_exactly_once() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let backend = Arc::new(LateSteerBackend::new(runtime_config.database_path.clone()));
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "late-steer-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.executions.load(Ordering::SeqCst) == 1).await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "late-steer-message"),
        Command::SendMessage(MessageCommand {
            session_id: session_id.clone(),
            message: "Do not lose this boundary message".into(),
        }),
    )
    .await;
    backend.release_first.notify_one();
    wait_for(|| backend.executions.load(Ordering::SeqCst) >= 2).await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE status = 'succeeded'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;

    let db = Database::new(&runtime_config.database_path).unwrap();
    let (canonical, pending): (i64, i64) = db
        .conn()
        .query_row(
            "SELECT
                 SUM(CASE WHEN role = 'user' THEN 1 ELSE 0 END),
                 SUM(CASE WHEN role LIKE 'pending_user:%' THEN 1 ELSE 0 END)
             FROM messages WHERE session_id = ?1",
            [&session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(canonical, 2);
    assert_eq!(pending, 0);
    runtime.shutdown().await;
}

#[tokio::test]
async fn generic_steer_without_a_client_pending_id_is_durable() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let backend = Arc::new(LateSteerBackend::new(runtime_config.database_path.clone()));
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "generic-steer-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.executions.load(Ordering::SeqCst) == 1).await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "generic-steer"),
        Command::Steer(SteerCommand {
            session_id: session_id.clone(),
            pending_id: None,
            content: serde_json::json!([{
                "type": "text",
                "text": "Preserve this generic steer",
            }]),
        }),
    )
    .await;
    backend.release_first.notify_one();
    wait_for(|| backend.executions.load(Ordering::SeqCst) >= 2).await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE session_id = ?1 AND role = 'user' AND content LIKE '%generic steer%'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE session_id = ?1 AND role LIKE 'pending_user:%'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn user_response_at_awaiting_boundary_is_not_stranded() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let backend = Arc::new(AwaitingResponseBoundaryBackend::new(
        runtime_config.database_path.clone(),
    ));
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "response-boundary-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.executions.load(Ordering::SeqCst) == 1).await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controller_events e
                 JOIN hive_controllers c ON c.id = e.controller_id
                 WHERE c.session_id = ?1 AND e.event_type = 'agentic_event'
                   AND json_extract(e.payload_json, '$.type') = 'awaiting_input'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    let run_id = Database::new(&runtime_config.database_path)
        .unwrap()
        .conn()
        .query_row(
            "SELECT id FROM hive_runs WHERE session_id = ?1 ORDER BY created_at LIMIT 1",
            [&session_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap();

    response(
        handler.as_ref(),
        context(Actor::local("test"), "response-boundary-answer"),
        Command::UserResponse(UserResponseCommand {
            session_id: session_id.clone(),
            run_id,
            tool_call_id: "question-1".into(),
            response: "continue".into(),
        }),
    )
    .await;

    wait_for(|| backend.executions.load(Ordering::SeqCst) >= 2).await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE session_id = ?1 AND status = 'succeeded'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;

    let db = Database::new(&runtime_config.database_path).unwrap();
    let (canonical_responses, pending_responses, indexed_responses): (i64, i64, i64) = db
        .conn()
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM messages
                  WHERE session_id = ?1 AND role = 'user' AND content LIKE '%continue%'),
                 (SELECT COUNT(*) FROM messages
                  WHERE session_id = ?1 AND role LIKE 'pending_user:%'),
                 (SELECT COUNT(*) FROM conversation_episodes
                  WHERE session_id = ?1 AND role = 'user' AND body LIKE '%continue%')",
            [&session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(canonical_responses, 1);
    assert_eq!(pending_responses, 0);
    assert_eq!(indexed_responses, 1);
    runtime.shutdown().await;
}

#[tokio::test]
async fn tool_approval_outbox_retries_until_the_active_host_accepts_it() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let backend = Arc::new(FlakyApprovalBackend::default());
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "approval-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.executions.load(Ordering::SeqCst) == 1).await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controller_events e
                 JOIN hive_controllers c ON c.id = e.controller_id
                 WHERE c.session_id = ?1 AND e.event_type = 'agentic_event'
                   AND json_extract(e.payload_json, '$.type') = 'tool_approval_required'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    let run_id = Database::new(&runtime_config.database_path)
        .unwrap()
        .conn()
        .query_row(
            "SELECT id FROM hive_runs WHERE session_id = ?1 ORDER BY created_at LIMIT 1",
            [&session_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap();

    let approval = Command::ToolApproval(ToolApprovalCommand {
        session_id: session_id.clone(),
        run_id: run_id.clone(),
        tool_call_id: "tool-1".into(),
        approved: true,
    });
    response(
        handler.as_ref(),
        context(Actor::local("test"), "approval-tool-1"),
        approval.clone(),
    )
    .await;
    // Idempotent transport replay must neither create another outbox row nor
    // bypass the scheduler-owned delivery contract.
    response(
        handler.as_ref(),
        context(Actor::local("test"), "approval-tool-1"),
        approval,
    )
    .await;

    wait_for(|| backend.delivered.load(Ordering::SeqCst)).await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_control_outbox
                 WHERE session_id = ?1 AND dedupe_key = ?2 AND status = 'delivered'",
                rusqlite::params![session_id, format!("{run_id}:tool-1")],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    let (rows, status, attempts): (i64, String, i64) = db
        .conn()
        .query_row(
            "SELECT COUNT(*), MAX(status), MAX(attempt_count)
             FROM hive_control_outbox WHERE session_id = ?1 AND dedupe_key = ?2",
            rusqlite::params![session_id, format!("{run_id}:tool-1")],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(rows, 1);
    assert_eq!(status, "delivered");
    assert!(attempts >= 2);
    drop(db);
    runtime.shutdown().await;
}

#[tokio::test]
async fn durable_event_exhaustion_cancels_exact_run_and_requires_recovery_immediately() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let mut runtime_config = config(&temp);
    runtime_config.max_execution_event_bytes = 32 * 1024;
    let backend = Arc::new(JournalExhaustionBackend::default());
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "journal-exhaustion-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.executions.load(Ordering::SeqCst) == 1).await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs
                 WHERE session_id = ?1 AND status = 'recovery_required'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;

    let db = Database::new(&runtime_config.database_path).unwrap();
    let (run_id, lease_token, attempt_outcome, finished_at): (
        String,
        Option<String>,
        String,
        Option<String>,
    ) = db
        .conn()
        .query_row(
            "SELECT r.id, r.lease_token, a.outcome, a.finished_at
             FROM hive_runs r JOIN hive_run_attempts a ON a.run_id = r.id
             WHERE r.session_id = ?1",
            [&session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert!(lease_token.is_none());
    assert_eq!(attempt_outcome, "recovery_required");
    assert!(finished_at.is_some());
    let oversized_finish_events = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM hive_controller_events e
             JOIN hive_controllers c ON c.id = e.controller_id
             WHERE c.session_id = ?1 AND e.event_type = 'agentic_event'
               AND json_extract(e.payload_json, '$.type') = 'finish'",
            [&session_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(oversized_finish_events, 0);
    drop(db);

    assert!(backend.controls.lock().unwrap().iter().any(
        |(controlled_session, control)| matches!(
            control,
            ExecutionControl::CancelRun { run_id: controlled_run, reason }
                if controlled_session == &session_id
                    && controlled_run == &run_id
                    && reason == "durable event journal exhausted; execution side effects may be uncertain"
        )
    ));
    runtime.shutdown().await;
}

#[tokio::test]
async fn active_cancel_persists_terminal_event_then_quiesces_for_delete() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let backend = Arc::new(CancellableEventBackend::default());
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "cancel-terminal-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.executions.load(Ordering::SeqCst) == 1).await;

    response(
        handler.as_ref(),
        context(Actor::local("test"), "cancel-terminal-request"),
        Command::CancelSession(SessionCommand {
            session_id: session_id.clone(),
        }),
    )
    .await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs
                 WHERE session_id = ?1 AND status = 'cancelled'",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;

    let db = Database::new(&runtime_config.database_path).unwrap();
    let (open_attempts, cancelled_attempts, finish_events): (i64, i64, i64) = db
        .conn()
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM hive_run_attempts a
                  JOIN hive_runs r ON r.id = a.run_id
                  WHERE r.session_id = ?1 AND a.finished_at IS NULL),
                 (SELECT COUNT(*) FROM hive_run_attempts a
                  JOIN hive_runs r ON r.id = a.run_id
                  WHERE r.session_id = ?1 AND a.outcome = 'cancelled'),
                 (SELECT COUNT(*) FROM hive_controller_events e
                  JOIN hive_controllers c ON c.id = e.controller_id
                  WHERE c.session_id = ?1 AND e.event_type = 'agentic_event'
                    AND json_extract(e.payload_json, '$.type') = 'finish')",
            [&session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(open_attempts, 0);
    assert_eq!(cancelled_attempts, 1);
    assert_eq!(finish_events, 1);
    let outcome_kind = db
        .conn()
        .query_row(
            "SELECT json_extract(outcome_json, '$.kind')
             FROM hive_runs WHERE session_id = ?1",
            [&session_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap();
    assert_eq!(outcome_kind, "cancelled");
    drop(db);

    response(
        handler.as_ref(),
        context(Actor::local("test"), "cancel-terminal-delete"),
        Command::DeleteSession(SessionCommand {
            session_id: session_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        Database::new(&runtime_config.database_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn leased_cancel_closes_unstarted_attempt_immediately() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let foreign =
        match HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
            .acquire(
                "hive-scheduler",
                "foreign-daemon",
                chrono::Utc::now(),
                Duration::from_secs(30),
            )
            .unwrap()
        {
            DaemonLeaseAcquire::Acquired(lease) => lease,
            other => panic!("expected foreign scheduler lease, got {other:?}"),
        };
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-b",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "leased-cancel-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    let claimed = HiveRunStore::new(Database::new(&runtime_config.database_path).unwrap())
        .claim_next(&ClaimRunRequest {
            executor_id: "test-executor".into(),
            lease_epoch: 9,
            now: chrono::Utc::now(),
            lease_duration: Duration::from_secs(30),
            global_concurrency_limit: 1,
        })
        .unwrap()
        .expect("queued run should enter the leased pre-execution state");

    response(
        handler.as_ref(),
        context(Actor::local("test"), "leased-cancel-request"),
        Command::CancelSession(SessionCommand {
            session_id: session_id.clone(),
        }),
    )
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    let (status, lease_token, attempt_outcome, finished_at): (
        String,
        Option<String>,
        String,
        Option<String>,
    ) = db
        .conn()
        .query_row(
            "SELECT r.status, r.lease_token, a.outcome, a.finished_at
             FROM hive_runs r JOIN hive_run_attempts a ON a.run_id = r.id
             WHERE r.id = ?1 AND a.attempt_no = ?2",
            rusqlite::params![claimed.run.id, claimed.attempt_no],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(status, "cancelled");
    assert!(lease_token.is_none());
    assert_eq!(attempt_outcome, "cancelled");
    assert!(finished_at.is_some());
    drop(db);

    response(
        handler.as_ref(),
        context(Actor::local("test"), "leased-cancel-delete"),
        Command::DeleteSession(SessionCommand { session_id }),
    )
    .await;
    HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
        .release("hive-scheduler", "foreign-daemon", foreign.fencing_token)
        .unwrap();
    runtime.shutdown().await;
}

#[tokio::test]
async fn noncooperative_cancel_terminalizes_after_grace_and_rejects_late_worker_writes() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let backend = Arc::new(BlockingBackend::default());
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "cancel-race-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| backend.executions.load(Ordering::SeqCst) == 1).await;

    let (run_id, lease_token, lease_epoch): (String, String, u64) = {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT id, lease_token, lease_epoch FROM hive_runs
                 WHERE session_id = ?1 AND status = 'running'",
                [&session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)? as u64)),
            )
            .unwrap()
    };
    let cancellation_started = tokio::time::Instant::now();

    response(
        handler.as_ref(),
        context(Actor::local("test"), "cancel-race-first"),
        Command::CancelSession(SessionCommand {
            session_id: session_id.clone(),
        }),
    )
    .await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT
                     EXISTS(SELECT 1 FROM hive_runs
                            WHERE id = ?1 AND status = 'cancelled'),
                     EXISTS(SELECT 1 FROM hive_controller_events
                            WHERE run_id = ?1 AND event_type = 'run_cancelled')",
                [&run_id],
                |row| Ok(row.get::<_, bool>(0)? && row.get::<_, bool>(1)?),
            )
            .unwrap()
    })
    .await;
    assert!(
        cancellation_started.elapsed() >= runtime_config.cancellation_grace_period,
        "noncooperative execution was terminalized before its cooperative grace elapsed"
    );

    let db = Database::new(&runtime_config.database_path).unwrap();
    let terminal: (
        String,
        Option<String>,
        Option<i64>,
        Option<String>,
        String,
        String,
        i64,
        i64,
        i64,
    ) = db
        .conn()
        .query_row(
            "SELECT status, lease_token, lease_epoch, finished_at,
                    last_stop_reason, last_error,
                    json_extract(outcome_json, '$.forced'),
                    json_extract(outcome_json, '$.side_effects_may_be_uncertain'),
                    json_extract(outcome_json, '$.abort_delivery_confirmed')
             FROM hive_runs WHERE id = ?1",
            [&run_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .unwrap();
    assert_eq!(terminal.0, "cancelled");
    assert!(terminal.1.is_none());
    assert!(terminal.2.is_none());
    assert!(terminal.3.is_some());
    assert_eq!(terminal.4, "cancellation grace elapsed");
    assert!(terminal.5.contains("side effects may be uncertain"));
    assert_eq!(terminal.6, 1);
    assert_eq!(terminal.7, 1);
    assert_eq!(terminal.8, 1);

    let attempt: (String, Option<String>, String, String) = db
        .conn()
        .query_row(
            "SELECT outcome, finished_at, stop_reason, error
             FROM hive_run_attempts WHERE run_id = ?1",
            [&run_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(attempt.0, "cancelled");
    assert!(attempt.1.is_some());
    assert_eq!(attempt.2, "cancellation grace elapsed");
    assert!(attempt.3.contains("side effects may be uncertain"));

    let runtime_projection: (String, String) = db
        .conn()
        .query_row(
            "SELECT status, current_run_id FROM hive_runtime_state WHERE session_id = ?1",
            [&session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(runtime_projection, ("cancelled".into(), run_id.clone()));
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controller_events
                 WHERE run_id = ?1 AND event_type = 'run_cancelled'
                   AND dedupe_key = ?2",
                rusqlite::params![run_id, format!("transition:{run_id}:1:cancelled")],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
    drop(db);

    let store = HiveRunStore::new(Database::new(&runtime_config.database_path).unwrap());
    let late_success = RunCompletion {
        target_status: HiveRunStatus::Succeeded,
        now: chrono::Utc::now(),
        available_at: None,
        wake_at: None,
        stop_reason: Some("late success".into()),
        error: None,
        outcome: Some(serde_json::json!({"late": true})),
        trace_sequence_end: None,
    };
    assert_eq!(
        store
            .finish_claimed(&run_id, &lease_token, lease_epoch, &late_success)
            .unwrap(),
        None
    );
    assert!(!store
        .heartbeat(
            &run_id,
            &lease_token,
            lease_epoch,
            chrono::Utc::now(),
            runtime_config.worker_lease_duration,
        )
        .unwrap());
    assert_eq!(
        store.get_run(&run_id).unwrap().unwrap().status,
        HiveRunStatus::Cancelled
    );
    wait_for(|| backend.execution_dropped.load(Ordering::SeqCst)).await;

    {
        let controls = backend.controls.lock().unwrap();
        assert!(controls.iter().any(|(controlled_session, control)| {
            controlled_session == &session_id
                && matches!(control, ExecutionControl::Cancel { reason } if reason == "cancelled by user")
        }));
        assert!(controls.iter().any(|(controlled_session, control)| {
            controlled_session == &session_id
                && matches!(control, ExecutionControl::CancelRun { run_id: controlled_run, reason }
                    if controlled_run == &run_id && reason == "cancelled by user")
        }));
        assert!(controls.iter().any(|(controlled_session, control)| {
            controlled_session == &session_id
                && matches!(control, ExecutionControl::AbortRun { run_id: controlled_run, reason }
                    if controlled_run == &run_id && reason == "cancellation grace elapsed")
        }));
    }

    response(
        handler.as_ref(),
        context(Actor::local("test"), "delete-after-cancel-grace"),
        Command::DeleteSession(SessionCommand {
            session_id: session_id.clone(),
        }),
    )
    .await;
    assert_eq!(
        Database::new(&runtime_config.database_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn workspace_less_legacy_session_rejects_enqueue_instead_of_dooming_runs() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-a",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    let now = canonical_timestamp(chrono::Utc::now());
    let db = Database::new(&runtime_config.database_path).unwrap();
    db.conn()
        .execute(
            "INSERT INTO sessions (
                id, title, created_at, updated_at, model, session_type
             ) VALUES ('workspace-less', 'Legacy', ?1, ?1, 'test:model', 'hive')",
            [&now],
        )
        .unwrap();
    drop(db);
    let handler = runtime.handler();

    // The execution host refuses claims without an explicit workspace, so the
    // enqueue surfaces must fail with an actionable error instead of
    // manufacturing runs that instantly fail with a redacted generic error.
    let start = handler
        .handle(
            context(Actor::local("test"), "workspace-less-start"),
            Command::StartSession(SessionCommand {
                session_id: "workspace-less".into(),
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(start.code, "state_conflict");
    assert!(
        start.message.contains("working or project directory"),
        "unexpected start message: {}",
        start.message
    );

    let message = handler
        .handle(
            context(Actor::local("test"), "workspace-less-message"),
            Command::SendMessage(MessageCommand {
                session_id: "workspace-less".into(),
                message: "please run".into(),
            }),
        )
        .await
        .unwrap_err();
    assert_eq!(message.code, "state_conflict");
    assert!(
        message.message.contains("working or project directory"),
        "unexpected message-turn message: {}",
        message.message
    );

    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE session_id = 'workspace-less'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0,
        "no run may be enqueued for a workspace-less session"
    );
    drop(db);
    runtime.shutdown().await;
}

#[tokio::test]
async fn server_created_session_gets_controller_and_waiting_runs_resume_durably() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-a",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    let now = canonical_timestamp(chrono::Utc::now());
    let initial_content = serde_json::to_string(&vec![mitsuro_core::Content::Text {
        text: "legacy objective".into(),
    }])
    .unwrap();
    let db = Database::new(&runtime_config.database_path).unwrap();
    db.conn()
        .execute(
            "INSERT INTO sessions (
                id, title, created_at, updated_at, working_dir, project_dir, model, session_type
             ) VALUES ('legacy-session', 'Legacy', ?1, ?1, '/work', '/work', 'test:model', 'hive')",
            [&now],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO messages (session_id, role, content, created_at)
             VALUES ('legacy-session', 'user', ?1, ?2)",
            rusqlite::params![initial_content, now],
        )
        .unwrap();
    drop(db);
    let handler = runtime.handler();
    response(
        handler.as_ref(),
        context(Actor::local("test"), "legacy-start"),
        Command::StartSession(SessionCommand {
            session_id: "legacy-session".into(),
        }),
    )
    .await;
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controllers WHERE session_id = 'legacy-session'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    response(
        handler.as_ref(),
        context(Actor::local("test"), "legacy-pause"),
        Command::PauseSession(SessionCommand {
            session_id: "legacy-session".into(),
        }),
    )
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    db.conn()
        .execute(
            "UPDATE hive_runs SET status = 'sleeping', wake_at = ?1,
                 finished_at = NULL, updated_at = ?2
             WHERE session_id = 'legacy-session'",
            rusqlite::params![
                canonical_timestamp(chrono::Utc::now() + chrono::Duration::hours(1)),
                canonical_timestamp(chrono::Utc::now())
            ],
        )
        .unwrap();
    drop(db);
    response(
        handler.as_ref(),
        context(Actor::local("test"), "wake-message"),
        Command::SendMessage(MessageCommand {
            session_id: "legacy-session".into(),
            message: "wake now".into(),
        }),
    )
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE session_id = 'legacy-session'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "queued"
    );
    db.conn()
        .execute(
            "UPDATE hive_runs SET status = 'awaiting_input' WHERE session_id = 'legacy-session'",
            [],
        )
        .unwrap();
    let (run_id, controller_id): (String, String) = db
        .conn()
        .query_row(
            "SELECT id, controller_id FROM hive_runs WHERE session_id = 'legacy-session'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let sequence = db
        .conn()
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1
             FROM hive_controller_events WHERE controller_id = ?1",
            [&controller_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO hive_controller_events (
                controller_id, sequence, event_type, run_id, payload_json, created_at
             ) VALUES (?1, ?2, 'agentic_event', ?3, ?4, ?5)",
            rusqlite::params![
                controller_id,
                sequence,
                run_id,
                serde_json::json!({
                    "type": "awaiting_input",
                    "tool_call_id": "question-1",
                })
                .to_string(),
                canonical_timestamp(chrono::Utc::now()),
            ],
        )
        .unwrap();
    drop(db);
    response(
        handler.as_ref(),
        context(Actor::local("test"), "wake-response"),
        Command::UserResponse(UserResponseCommand {
            session_id: "legacy-session".into(),
            run_id,
            tool_call_id: "question-1".into(),
            response: "continue".into(),
        }),
    )
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT status FROM hive_runs WHERE session_id = 'legacy-session'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "queued"
    );
    let (canonical_response, indexed_response): (i64, i64) = db
        .conn()
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM messages
                  WHERE session_id = 'legacy-session' AND role = 'user'
                    AND content LIKE '%continue%'),
                 (SELECT COUNT(*) FROM conversation_episodes
                  WHERE session_id = 'legacy-session' AND role = 'user'
                    AND body LIKE '%continue%')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(canonical_response, 1);
    assert_eq!(indexed_response, 1);
    runtime.shutdown().await;
}

#[tokio::test]
async fn replay_limit_zero_is_live_only_with_an_atomic_high_water() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime = start_runtime(config(&temp), "daemon-a", Arc::new(FakeBackend::default()))
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "live-only-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| {
        let db = Database::new(&config(&temp).database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE status = 'succeeded'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    let reply = handler
        .handle(
            context(Actor::local("test"), "live-only-subscribe"),
            Command::Subscribe(SubscribeCommand {
                session_id: session_id.clone(),
                after_sequence: Some(0),
                replay_limit: Some(0),
            }),
        )
        .await
        .unwrap();
    let HandlerReply::Subscription {
        accepted,
        mut events,
    } = reply
    else {
        panic!("expected subscription");
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(75), events.recv())
            .await
            .is_err()
    );
    response(
        handler.as_ref(),
        context(Actor::local("test"), "live-only-priority"),
        Command::SetPriority(SetPriorityCommand {
            session_id,
            priority: "high".into(),
        }),
    )
    .await;
    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(event.sequence > accepted.high_water_sequence);
    assert!(matches!(event.event, HiveEvent::Runtime(_)));
    runtime.shutdown().await;
}

#[tokio::test]
async fn live_only_unsequenced_events_are_forwarded_without_advancing_cursor() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime = start_runtime(config(&temp), "daemon-a", Arc::new(FakeBackend::default()))
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "ephemeral-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    let HandlerReply::Subscription { mut events, .. } = handler
        .handle(
            context(Actor::local("test"), "ephemeral-subscribe"),
            Command::Subscribe(SubscribeCommand {
                session_id: session_id.clone(),
                after_sequence: None,
                replay_limit: Some(0),
            }),
        )
        .await
        .unwrap()
    else {
        panic!("expected subscription");
    };
    handler
        .shared
        .events
        .publish(mitsuro_hive_protocol::EventEnvelope {
            version: mitsuro_hive_protocol::ProtocolVersion::CURRENT,
            session_id: Some(session_id),
            run_id: Some("run-ephemeral".into()),
            sequence: None,
            emitted_at_unix_ms: 0,
            event: HiveEvent::Runtime(mitsuro_hive_protocol::RuntimeEvent {
                event_type: "live_delta".into(),
                payload: serde_json::json!({"delta": "authenticated live content"}),
            }),
        });
    let event = tokio::time::timeout(Duration::from_secs(1), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event.sequence, None);
    assert!(matches!(
        event.event,
        HiveEvent::Runtime(runtime) if runtime.event_type == "live_delta"
    ));
    runtime.shutdown().await;
}

#[tokio::test]
async fn execution_events_are_bounded_ordered_replayable_and_drained_before_completion() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let mut runtime_config = config(&temp);
    runtime_config.execution_event_capacity = 1;
    runtime_config.max_execution_event_bytes = 512;
    let backend = Arc::new(EventBackend);
    let runtime = start_runtime(runtime_config.clone(), "daemon-a", backend.clone())
        .await
        .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "eventful-dispatch"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| {
        let db = Database::new(&runtime_config.database_path).unwrap();
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE status = 'succeeded'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    let rows = {
        let mut statement = db
            .conn()
            .prepare(
                "SELECT sequence, event_type, payload_json
                 FROM hive_controller_events ORDER BY sequence",
            )
            .unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    assert!(rows.windows(2).all(|pair| pair[0].0 < pair[1].0));
    let agentic = rows
        .iter()
        .filter(|(_, event_type, _)| event_type == "agentic_event")
        .collect::<Vec<_>>();
    assert_eq!(agentic.len(), 6);
    for (index, (_, _, payload)) in agentic.iter().enumerate() {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(payload).unwrap()["tick_number"].as_u64(),
            Some(index as u64)
        );
    }
    assert_eq!(rows.last().unwrap().1, "run_completed");
    drop(db);

    let reply = handler
        .handle(
            context(Actor::local("test"), "eventful-replay"),
            Command::Subscribe(SubscribeCommand {
                session_id,
                after_sequence: Some(0),
                replay_limit: Some(100),
            }),
        )
        .await
        .unwrap();
    let HandlerReply::Subscription {
        accepted,
        mut events,
    } = reply
    else {
        panic!("expected subscription");
    };
    let high_water = accepted.high_water_sequence.unwrap();
    let mut replayed_agentic = 0;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
        if matches!(
            event.event,
            HiveEvent::Extension(ref extension) if extension.name == "agentic_event"
        ) {
            replayed_agentic += 1;
        }
        if event.sequence == Some(high_water) {
            break;
        }
    }
    assert_eq!(replayed_agentic, 6);
    runtime.shutdown().await;
}

#[tokio::test]
async fn backend_private_error_output_and_tool_payload_never_enter_durable_state() {
    const SENTINEL: &str = "HIVE_PRIVATE_SENTINEL_91f6";
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-a",
        Arc::new(PrivacyBoundaryBackend::default()),
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    let failed_session = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "private-failure"),
            dispatch_command(),
        )
        .await,
    );
    let succeeded_session = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "private-success"),
            dispatch_command(),
        )
        .await,
    );
    wait_for(|| {
        Database::new(&runtime_config.database_path)
            .unwrap()
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_runs WHERE status IN ('failed', 'succeeded')",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 2
    })
    .await;

    let db = Database::new(&runtime_config.database_path).unwrap();
    let pattern = format!("%{SENTINEL}%");
    let leaked: i64 = db
        .conn()
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM hive_runs
                  WHERE COALESCE(last_error, '') LIKE ?1
                     OR COALESCE(outcome_json, '') LIKE ?1)
               + (SELECT COUNT(*) FROM hive_run_attempts
                  WHERE COALESCE(error, '') LIKE ?1
                     OR COALESCE(stop_reason, '') LIKE ?1)
               + (SELECT COUNT(*) FROM hive_controller_events
                  WHERE payload_json LIKE ?1)",
            [&pattern],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(leaked, 0);
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controller_events
                 WHERE event_type = 'agentic_event'
                   AND json_extract(payload_json, '$.type') = 'tool_result'
                   AND json_extract(payload_json, '$.output_redacted') = 1",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
    drop(db);

    for (index, session_id) in [failed_session, succeeded_session].into_iter().enumerate() {
        let reply = handler
            .handle(
                context(Actor::local("test"), &format!("privacy-replay-{index}")),
                Command::Subscribe(SubscribeCommand {
                    session_id,
                    after_sequence: Some(0),
                    replay_limit: Some(1_000),
                }),
            )
            .await
            .unwrap();
        let HandlerReply::Subscription {
            accepted,
            mut events,
        } = reply
        else {
            panic!("expected privacy replay subscription");
        };
        let high_water = accepted.high_water_sequence.unwrap_or_default();
        while let Ok(Some(event)) =
            tokio::time::timeout(Duration::from_secs(1), events.recv()).await
        {
            assert!(!serde_json::to_string(&event).unwrap().contains(SENTINEL));
            if event.sequence == Some(high_water) {
                break;
            }
        }
    }
    runtime.shutdown().await;

    for path in [
        runtime_config.database_path.clone(),
        runtime_config.database_path.with_extension("db-wal"),
        runtime_config.database_path.with_extension("db-shm"),
    ] {
        if let Ok(bytes) = std::fs::read(&path) {
            assert!(
                !bytes
                    .windows(SENTINEL.len())
                    .any(|window| window == SENTINEL.as_bytes()),
                "private execution payload reached {}",
                path.display()
            );
        }
    }
}

#[tokio::test]
async fn stale_daemon_cannot_materialize_or_advance_a_schedule_after_takeover() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-a",
        Arc::new(FakeBackend::default()),
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    let session_id = dispatch_session_id(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "dispatch-fenced-schedule"),
            dispatch_command(),
        )
        .await,
    );
    let fire_at = chrono::Utc::now() + chrono::Duration::hours(1);
    response(
        handler.as_ref(),
        context(Actor::local("test"), "future-schedule"),
        Command::ScheduleSession(ScheduleCommand {
            session_id,
            wake_at_unix_ms: fire_at.timestamp_millis(),
            reason: "future fenced work".into(),
        }),
    )
    .await;
    wait_for(|| {
        HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
            .get("hive-scheduler")
            .unwrap()
            .is_some()
    })
    .await;
    let stale_lease =
        HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
            .get("hive-scheduler")
            .unwrap()
            .unwrap();
    runtime.shutdown().await;

    let lease_store =
        HiveDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap());
    let current = match lease_store
        .acquire(
            "hive-scheduler",
            "daemon-b:boot:test",
            chrono::Utc::now(),
            Duration::from_secs(60),
        )
        .unwrap()
    {
        DaemonLeaseAcquire::Acquired(lease) => lease,
        value => panic!("takeover failed: {value:?}"),
    };
    assert!(current.fencing_token > stale_lease.fencing_token);

    let db = Database::new(&runtime_config.database_path).unwrap();
    let schedule_id = db
        .conn()
        .query_row("SELECT id FROM hive_schedules LIMIT 1", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    drop(db);
    let schedule = HiveScheduleStore::new(Database::new(&runtime_config.database_path).unwrap())
        .get_schedule(&schedule_id)
        .unwrap()
        .unwrap();
    let scheduled_for =
        mitsuro_core::hive::parse_utc_timestamp(schedule.next_fire_at.as_deref().unwrap()).unwrap();
    let events = materialize_schedule_transaction(
        runtime_config.database_path.clone(),
        schedule,
        MisfireResolution {
            enqueue: vec![MisfireDispatch {
                scheduled_for,
                coalesced_count: 0,
            }],
            skipped: Vec::new(),
        },
        scheduled_for,
        None,
        DaemonFence {
            lease_name: stale_lease.lease_name,
            owner_id: stale_lease.owner_id,
            fencing_token: stale_lease.fencing_token,
        },
    )
    .unwrap();
    assert!(events.is_empty());
    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT revision FROM hive_schedules WHERE id = ?1",
                [&schedule_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_schedule_occurrences",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .unwrap(),
        0
    );
}

fn seed_retention_controller(
    db: &Database,
    suffix: &str,
    run_status: Option<&str>,
) -> super::persistence::ControllerRecord {
    let session_id = format!("session-{suffix}");
    let controller_id = format!("controller-{suffix}");
    let run_id = format!("run-{suffix}");
    let now = "2026-07-17T00:00:00.000000Z";
    db.conn()
        .execute(
            "INSERT INTO sessions (
                 id, title, created_at, updated_at, model, session_type
             ) VALUES (?1, 'Hive', ?2, ?2, 'test:model', 'hive')",
            rusqlite::params![session_id, now],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO hive_controllers (
                 id, scope_key, session_id, status, timezone,
                 max_concurrent_runs, created_at, updated_at
             ) VALUES (?1, ?2, ?3, 'active', 'UTC', 1, ?4, ?4)",
            rusqlite::params![
                controller_id,
                format!("session:{session_id}"),
                session_id,
                now
            ],
        )
        .unwrap();
    if let Some(run_status) = run_status {
        db.conn()
            .execute(
                "INSERT INTO hive_runs (
                     id, controller_id, session_id, kind, objective, config_json,
                     status, available_at, attempt_count, max_attempts, created_at, updated_at
                 ) VALUES (?1, ?2, ?3, 'dispatch', 'retention test', '{}',
                           ?4, ?5, 1, 3, ?5, ?5)",
                rusqlite::params![run_id, controller_id, session_id, run_status, now],
            )
            .unwrap();
    }
    super::persistence::ControllerRecord {
        id: controller_id,
        session_id,
        status: "active".into(),
        timezone: "UTC".into(),
    }
}

#[tokio::test]
async fn durable_runtime_stats_count_each_scheduler_state_exactly() {
    let temp = TempDir::new().unwrap();
    let database_path = temp.path().join("stats.db");
    let db = Database::new(&database_path).unwrap();
    for (suffix, status) in [
        ("leased", "leased"),
        ("running", "running"),
        ("queued-a", "queued"),
        ("queued-b", "queued"),
        ("recovery", "recovery_required"),
        ("sleeping", "sleeping"),
        ("inactive", "succeeded"),
        ("alice-running", "running"),
    ] {
        seed_retention_controller(&db, suffix, Some(status));
    }
    db.conn()
        .execute(
            "UPDATE hive_controllers SET status = 'paused' WHERE id = 'controller-inactive'",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "INSERT INTO users (id, email) VALUES ('alice', 'alice@example.invalid')",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "UPDATE hive_controllers SET user_id = 'alice'
             WHERE id = 'controller-alice-running'",
            [],
        )
        .unwrap();
    db.conn()
        .execute(
            "UPDATE sessions SET user_id = 'alice' WHERE id = 'session-alice-running'",
            [],
        )
        .unwrap();
    drop(db);

    let persistence =
        super::persistence::RuntimePersistence::new(database_path, Duration::from_secs(60));
    let stats = persistence
        .stats(&Actor::local("stats-test"))
        .await
        .unwrap();

    assert_eq!(stats.active_controllers, 6);
    assert_eq!(stats.active_runs, 2);
    assert_eq!(stats.queued_runs, 2);
    assert_eq!(stats.recovery_required, 1);
    assert!(!stats.pump_alive);
    assert!(!stats.scheduler_ready);

    let alice = persistence
        .stats(&Actor {
            user_id: Some("alice".to_string()),
            client_kind: "stats-test".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(alice.active_controllers, 1);
    assert_eq!(alice.active_runs, 1);
    assert_eq!(alice.queued_runs, 0);
    assert_eq!(alice.recovery_required, 0);
}

#[tokio::test]
async fn old_recovery_run_cannot_defeat_the_hard_event_retention_cap() {
    // This test deliberately performs thousands of SQLite retention passes.
    // Keep it behind the same guard as the lease-sensitive runtime tests so
    // the test harness cannot manufacture worker lease expiry through local
    // I/O contention on slower CI hosts.
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("retention-recovery.db")).unwrap();
    let controller = seed_retention_controller(&db, "old-recovery", Some("recovery_required"));
    let run_id = "run-old-recovery";
    let tx =
        rusqlite::Transaction::new_unchecked(db.conn(), rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    super::persistence::append_event(
        &tx,
        &controller,
        "recovery_required",
        Some(run_id),
        None,
        None,
        serde_json::json!({"run_id": run_id, "status": "recovery_required"}),
        "2026-07-17T00:00:00.000000Z",
    )
    .unwrap();
    for index in 0..3_000 {
        super::persistence::append_event(
            &tx,
            &controller,
            "newer_event",
            None,
            None,
            None,
            serde_json::json!({"status": "completed", "attempt": index}),
            "2026-07-17T00:00:00.000000Z",
        )
        .unwrap();
    }
    tx.commit().unwrap();

    let (count, bytes, minimum, maximum): (i64, i64, i64, i64) = db
        .conn()
        .query_row(
            "SELECT COUNT(*),
                    COALESCE(SUM(
                      length(CAST(payload_json AS BLOB))
                      + length(CAST(event_type AS BLOB))
                      + COALESCE(length(CAST(dedupe_key AS BLOB)), 0)
                    ), 0),
                    MIN(sequence), MAX(sequence)
               FROM hive_controller_events
              WHERE controller_id = ?1",
            [&controller.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(count, 2_048);
    assert!(bytes <= 2 * 1024 * 1024);
    assert!(minimum > 1, "old recovery history must be prefix-prunable");
    assert_eq!(maximum, 3_001, "the high-water sequence must survive");
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controller_events
                  WHERE controller_id = ?1 AND sequence = 1",
                [&controller.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn unresolved_interaction_spam_is_rejected_without_losing_pending_truth() {
    // See the retention-cap test above: this intentionally expensive fixture
    // shares the runtime serialization guard to avoid cross-test lease races.
    let _test_guard = runtime_test_guard().await;
    const SENTINEL: &str = "HIVE_RETENTION_ARGUMENT_SENTINEL";
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("retention-pending.db")).unwrap();
    let controller = seed_retention_controller(&db, "pending", Some("running"));
    let run_id = "run-pending";
    let tx =
        rusqlite::Transaction::new_unchecked(db.conn(), rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    for interaction in 0..32 {
        super::persistence::append_event(
            &tx,
            &controller,
            "agentic_event",
            Some(run_id),
            None,
            None,
            serde_json::json!({
                "type": "tool_approval_required",
                "id": format!("tool-pending-{interaction}"),
                "name": "bash",
                "arguments": {"command": SENTINEL},
            }),
            "2026-07-17T00:00:00.000000Z",
        )
        .unwrap();
    }
    for index in 0..1_984 {
        super::persistence::append_event(
            &tx,
            &controller,
            "newer_event",
            None,
            None,
            None,
            serde_json::json!({"status": "completed", "attempt": index}),
            "2026-07-17T00:00:00.000000Z",
        )
        .unwrap();
    }
    tx.commit().unwrap();

    let rejected =
        rusqlite::Transaction::new_unchecked(db.conn(), rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    let error = super::persistence::append_event(
        &rejected,
        &controller,
        "agentic_event",
        Some(run_id),
        None,
        None,
        serde_json::json!({
            "type": "tool_approval_required",
            "id": "tool-pending-overflow",
            "name": "bash",
            "arguments": {},
        }),
        "2026-07-17T00:00:00.000000Z",
    )
    .unwrap_err();
    assert!(matches!(
        error,
        super::persistence::RuntimeStoreError::ResourceExhausted(_)
    ));
    drop(rejected);

    let (count, pending): (i64, String) = db
        .conn()
        .query_row(
            "SELECT COUNT(*),
                    MAX(CASE WHEN sequence = 1 THEN payload_json END)
               FROM hive_controller_events
              WHERE controller_id = ?1",
            [&controller.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(count, 2_016);
    assert!(!pending.contains(SENTINEL));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&pending).unwrap()["id"],
        "tool-pending-0"
    );

    let unrelated =
        rusqlite::Transaction::new_unchecked(db.conn(), rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    let unrelated_error = super::persistence::append_event(
        &unrelated,
        &controller,
        "unrelated_overflow",
        None,
        None,
        None,
        serde_json::json!({"status": "completed"}),
        "2026-07-17T00:00:00.000000Z",
    )
    .unwrap_err();
    assert!(matches!(
        unrelated_error,
        super::persistence::RuntimeStoreError::ResourceExhausted(_)
    ));
    drop(unrelated);

    let resolving =
        rusqlite::Transaction::new_unchecked(db.conn(), rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    let resolution = super::persistence::append_event(
        &resolving,
        &controller,
        "tool_approval_queued",
        Some(run_id),
        None,
        None,
        serde_json::json!({
            "run_id": run_id,
            "tool_call_id": "tool-pending-31",
            "approved": false,
        }),
        "2026-07-17T00:00:00.000000Z",
    )
    .unwrap();
    assert_eq!(resolution.sequence, 2_017);
    resolving.commit().unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controller_events WHERE controller_id = ?1",
                [&controller.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2_017
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT MIN(sequence) FROM hive_controller_events WHERE controller_id = ?1",
                [&controller.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        1
    );
}

#[test]
fn dedupe_hit_runs_retention_and_terminal_outbox_maintenance() {
    let temp = TempDir::new().unwrap();
    let db = Database::new(&temp.path().join("retention-dedupe.db")).unwrap();
    let controller = seed_retention_controller(&db, "dedupe", Some("succeeded"));
    let tx =
        rusqlite::Transaction::new_unchecked(db.conn(), rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    for sequence in 1_i64..=2_050 {
        tx.execute(
            "INSERT INTO hive_controller_events (
                 controller_id, sequence, event_type, dedupe_key, payload_json, created_at
             ) VALUES (?1, ?2, 'historical_event', ?3, '{\"status\":\"completed\"}', ?4)",
            rusqlite::params![
                controller.id,
                sequence,
                (sequence == 2_050).then_some("dedupe-latest"),
                "2026-07-17T00:00:00.000000Z",
            ],
        )
        .unwrap();
    }
    tx.execute(
        "INSERT INTO hive_control_outbox (
             id, controller_id, session_id, run_id, control_kind, dedupe_key,
             payload_json, status, available_at, delivered_at, created_at, updated_at
         ) VALUES (
             'outbox-old', ?1, ?2, 'run-dedupe', 'tool_approval', 'old-control',
             '{\"tool_call_id\":\"old-tool\",\"approved\":true}', 'delivered',
             '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z',
             '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z'
         )",
        rusqlite::params![controller.id, controller.session_id],
    )
    .unwrap();
    tx.commit().unwrap();

    let maintenance =
        rusqlite::Transaction::new_unchecked(db.conn(), rusqlite::TransactionBehavior::Immediate)
            .unwrap();
    let existing = super::persistence::append_event(
        &maintenance,
        &controller,
        "historical_event",
        None,
        None,
        Some("dedupe-latest"),
        serde_json::json!({"ignored": "duplicate"}),
        "2026-07-17T00:00:00.000000Z",
    )
    .unwrap();
    assert_eq!(existing.sequence, 2_050);
    maintenance.commit().unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_controller_events WHERE controller_id = ?1",
                [&controller.id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2_048
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM hive_control_outbox WHERE id = 'outbox-old'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

/// Maps each Worker DM session to a fixed outcome so group fan-out tests are
/// deterministic regardless of claim order.
#[derive(Default)]
struct GroupBackend {
    executions: Mutex<Vec<(String, String)>>,
    outcomes_by_session: Mutex<std::collections::HashMap<String, ExecutionOutcome>>,
    controls: Mutex<Vec<(String, ExecutionControl)>>,
}

impl GroupBackend {
    fn fail_session(&self, session_id: &str) {
        self.outcomes_by_session.lock().unwrap().insert(
            session_id.to_string(),
            ExecutionOutcome::Failed {
                error: "provider unavailable".into(),
                retryable: false,
                retry_after: None,
            },
        );
    }

    fn execution_sessions(&self) -> Vec<String> {
        self.executions
            .lock()
            .unwrap()
            .iter()
            .map(|(session, _)| session.clone())
            .collect()
    }
}

#[async_trait]
impl ExecutionBackend for GroupBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        let session_id = request.claim.run.session_id.clone().unwrap_or_default();
        self.executions
            .lock()
            .unwrap()
            .push((session_id.clone(), request.claim.run.id.clone()));
        self.outcomes_by_session
            .lock()
            .unwrap()
            .get(&session_id)
            .cloned()
            .unwrap_or(ExecutionOutcome::Succeeded {
                output: serde_json::json!({"ok": true}),
            })
    }

    async fn control(&self, session_id: &str, control: ExecutionControl) -> anyhow::Result<()> {
        self.controls
            .lock()
            .unwrap()
            .push((session_id.to_string(), control));
        Ok(())
    }
}

struct GroupFixture {
    group_id: String,
    /// (worker_id, dm_session_id) in roster order.
    members: Vec<(String, String)>,
}

fn seed_group(
    db_path: &std::path::Path,
    slugs: &[&str],
    mode: mitsuro_core::storage::HiveGroupExecutionMode,
    max_rounds: u32,
) -> GroupFixture {
    let session_manager = SessionManager::new(Database::new(db_path).unwrap());
    let worker_store = mitsuro_core::storage::HiveWorkerStore::new(Database::new(db_path).unwrap());
    let mut members = Vec::new();
    for slug in slugs {
        let dm_session_id = session_manager
            .create_session_for_user_with_config(
                &format!("{slug} DM"),
                Some("test:model"),
                Some("/work/repo"),
                None,
                WorkspaceMode::Neutral,
                None,
                None,
                SessionType::Hive,
            )
            .unwrap();
        let worker = worker_store
            .create(&mitsuro_core::storage::NewHiveWorker {
                model: Some("test:model".into()),
                dm_session_id: Some(dm_session_id.clone()),
                ..mitsuro_core::storage::NewHiveWorker::new(*slug)
            })
            .unwrap();
        members.push((worker.id, dm_session_id));
    }
    let group = mitsuro_core::storage::HiveGroupStore::new(Database::new(db_path).unwrap())
        .create(&mitsuro_core::storage::NewHiveGroup {
            user_id: None,
            title: "Integration Room".into(),
            execution_mode: mode,
            max_rounds: Some(max_rounds),
            max_member_messages_per_turn: Some(2),
            parallelism: Some(2),
            context_window_messages: Some(24),
            default_assignee_worker_id: None,
            member_worker_ids: members.iter().map(|(id, _)| id.clone()).collect(),
        })
        .unwrap();
    GroupFixture {
        group_id: group.id,
        members,
    }
}

fn group_turn_response(response: &ResponsePayload) -> mitsuro_hive_protocol::GroupTurnResponse {
    match response {
        ResponsePayload::GroupTurn(turn) => turn.clone(),
        other => panic!("expected group turn response, got {other:?}"),
    }
}

fn load_turn_status(
    db_path: &std::path::Path,
    turn_id: &str,
) -> (
    mitsuro_core::storage::HiveGroupTurnStatus,
    Option<serde_json::Value>,
    u32,
) {
    let db = Database::new(db_path).unwrap();
    let turn = mitsuro_core::storage::hive_groups::load_turn(db.conn(), turn_id)
        .unwrap()
        .unwrap();
    (turn.status, turn.member_outcomes, turn.next_speaker_index)
}

#[tokio::test]
async fn group_workbench_turn_executes_members_and_aggregates_partial_failure() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let fixture = seed_group(
        &runtime_config.database_path,
        &["healthy", "flaky"],
        mitsuro_core::storage::HiveGroupExecutionMode::Workbench,
        3,
    );
    let backend = Arc::new(GroupBackend::default());
    backend.fail_session(&fixture.members[1].1);

    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-a",
        Arc::clone(&backend) as Arc<dyn ExecutionBackend>,
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    let turn = group_turn_response(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "group-workbench-1"),
            Command::GroupMessage(GroupMessageCommand {
                group_id: fixture.group_id.clone(),
                message: "everyone take a look".into(),
                mentions_override: None,
            }),
        )
        .await,
    );
    assert_eq!(turn.status, "running");
    assert_eq!(turn.target_worker_ids.len(), 2);

    let db_path = runtime_config.database_path.clone();
    let turn_id = turn.turn_id.clone();
    wait_for(move || {
        load_turn_status(&db_path, &turn_id).0
            == mitsuro_core::storage::HiveGroupTurnStatus::Partial
    })
    .await;

    // Both members executed on their own DM lanes; the healthy sibling was
    // never cancelled by the flaky one.
    let sessions = backend.execution_sessions();
    assert_eq!(sessions.len(), 2);
    assert!(sessions.contains(&fixture.members[0].1));
    assert!(sessions.contains(&fixture.members[1].1));

    let (_, outcomes, _) = load_turn_status(&runtime_config.database_path, &turn.turn_id);
    let outcomes = outcomes.unwrap();
    assert_eq!(outcomes[&fixture.members[0].0]["status"], "succeeded");
    assert_eq!(outcomes[&fixture.members[1].0]["status"], "failed");
    runtime.shutdown().await;
}

#[tokio::test]
async fn group_roundtable_advances_rotating_speakers_to_completion() {
    let _test_guard = runtime_test_guard().await;
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let fixture = seed_group(
        &runtime_config.database_path,
        &["alpha", "beta"],
        mitsuro_core::storage::HiveGroupExecutionMode::Roundtable,
        2,
    );
    let backend = Arc::new(GroupBackend::default());
    let runtime = start_runtime(
        runtime_config.clone(),
        "daemon-a",
        Arc::clone(&backend) as Arc<dyn ExecutionBackend>,
    )
    .await
    .unwrap();
    let handler = runtime.handler();
    let turn = group_turn_response(
        &response(
            handler.as_ref(),
            context(Actor::local("test"), "group-roundtable-1"),
            Command::GroupMessage(GroupMessageCommand {
                group_id: fixture.group_id.clone(),
                message: "round table please".into(),
                mentions_override: None,
            }),
        )
        .await,
    );

    let db_path = runtime_config.database_path.clone();
    let turn_id = turn.turn_id.clone();
    wait_for(move || {
        load_turn_status(&db_path, &turn_id).0
            == mitsuro_core::storage::HiveGroupTurnStatus::Completed
    })
    .await;

    // Two rounds over [alpha, beta] rotate the speaker order per round and
    // execute strictly one at a time.
    let alpha = fixture.members[0].1.clone();
    let beta = fixture.members[1].1.clone();
    assert_eq!(
        backend.execution_sessions(),
        vec![alpha.clone(), beta.clone(), beta, alpha]
    );
    let (_, outcomes, next_speaker_index) =
        load_turn_status(&runtime_config.database_path, &turn.turn_id);
    assert_eq!(next_speaker_index, 4);
    let outcomes = outcomes.unwrap();
    assert_eq!(outcomes[&fixture.members[0].0]["status"], "succeeded");
    assert_eq!(outcomes[&fixture.members[1].0]["status"], "succeeded");
    runtime.shutdown().await;
}
