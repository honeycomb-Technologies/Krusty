use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use krusty_core::mako::{canonical_timestamp, MisfireDispatch, MisfireResolution};
use krusty_core::storage::{
    DaemonFence, DaemonLeaseAcquire, Database, MakoDaemonLeaseStore, MakoScheduleStore,
};
use krusty_mako_protocol::{
    Actor, Command, DispatchCommand, MakoEvent, MessageCommand, PeerIdentity, ResponsePayload,
    ScheduleCommand, SessionCommand, SetPriorityCommand, SubscribeCommand, UserResponseCommand,
};
use tempfile::TempDir;

use crate::{CommandContext, CommandHandler, HandlerReply};

use super::pump::materialize_schedule_transaction;
use super::{
    start_runtime, ExecutionBackend, ExecutionControl, ExecutionOutcome, ExecutionRequest,
    MakoRuntimeConfig,
};

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
    controls: Mutex<Vec<(String, ExecutionControl)>>,
}

#[derive(Default)]
struct EventBackend {
    oversize_rejected: AtomicBool,
}

#[async_trait]
impl ExecutionBackend for EventBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        for index in 0..6 {
            request
                .events
                .agentic(serde_json::json!({
                    "event": "test_agentic_event",
                    "index": index,
                }))
                .await
                .expect("bounded execution event should be accepted");
        }
        let error = request
            .events
            .agentic(serde_json::json!({"body": "x".repeat(2_048)}))
            .await
            .expect_err("oversized execution event must be rejected");
        self.oversize_rejected.store(
            matches!(
                error,
                super::ExecutionEventSendError::PayloadTooLarge { .. }
            ),
            Ordering::SeqCst,
        );
        ExecutionOutcome::Succeeded {
            output: serde_json::json!({"ok": true}),
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

fn config(temp: &TempDir) -> MakoRuntimeConfig {
    let mut config = MakoRuntimeConfig::for_database(temp.path().join("runtime.db"));
    config.scheduler_poll_interval = Duration::from_millis(20);
    config.daemon_lease_duration = Duration::from_millis(500);
    config.worker_lease_duration = Duration::from_millis(200);
    config.worker_heartbeat_interval = Duration::from_millis(40);
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
    tokio::time::timeout(Duration::from_secs(3), async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn duplicate_dispatch_replays_without_creating_a_second_run() {
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
            .query_row("SELECT COUNT(*) FROM mako_runs", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn message_after_completion_queues_exactly_one_idempotent_followup_run() {
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
                "SELECT COUNT(*) FROM mako_runs WHERE status = 'succeeded'",
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
                "SELECT COUNT(*) FROM mako_runs WHERE session_id = ?1",
                [&session_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
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
    runtime.shutdown().await;
}

#[tokio::test]
async fn queue_claim_execution_and_once_schedule_survive_the_process_boundary() {
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

    let db = Database::new(&config(&temp).database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM mako_runs WHERE status = 'succeeded'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        2
    );
    assert_eq!(
        db.conn()
            .query_row("SELECT status FROM mako_schedules LIMIT 1", [], |row| {
                row.get::<_, String>(0)
            },)
            .unwrap(),
        "completed"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn restart_marks_expired_running_work_recovery_required_without_replay() {
    let temp = TempDir::new().unwrap();
    let runtime_config = config(&temp);
    let db = Database::new(&runtime_config.database_path).unwrap();
    let now = canonical_timestamp(chrono::Utc::now());
    let expired = canonical_timestamp(chrono::Utc::now() - chrono::Duration::seconds(1));
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO sessions (id, title, created_at, updated_at, session_type)
             VALUES ('session-1', 'Mako', '{now}', '{now}', 'mako');
             INSERT INTO mako_controllers (
                 id, scope_key, session_id, status, timezone, max_concurrent_runs, created_at, updated_at
             ) VALUES ('controller-1', 'session:session-1', 'session-1', 'active', 'UTC', 1, '{now}', '{now}');
             INSERT INTO mako_runs (
                 id, controller_id, session_id, kind, objective, config_json, status,
                 priority, available_at, attempt_count, max_attempts, lease_owner,
                 lease_token, lease_epoch, lease_expires_at, heartbeat_at, created_at, updated_at
             ) VALUES ('run-1', 'controller-1', 'session-1', 'dispatch', 'work', '{{}}',
                 'running', 0, '{now}', 1, 3, 'old-daemon', 'old-token', 1, '{expired}',
                 '{expired}', '{now}', '{now}');
             INSERT INTO mako_run_attempts (
                 id, run_id, attempt_no, worker_id, lease_token, lease_epoch, started_at, outcome
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
                "SELECT status FROM mako_runs WHERE id = 'run-1'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
            == "recovery_required"
    })
    .await;
    assert_eq!(backend.execution_count(), 0);
    runtime.shutdown().await;
}

#[tokio::test]
async fn replay_and_live_events_remain_monotonic_and_report_a_retention_gap() {
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
            "DELETE FROM mako_controller_events
             WHERE sequence = (SELECT MIN(sequence) FROM mako_controller_events)",
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
        MakoEvent::ReplayGap(_)
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
        serde_json::from_str::<Vec<krusty_core::Content>>(&latest_content)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM mako_runs WHERE session_id = ?1",
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
                "SELECT status FROM mako_runs WHERE session_id = ?1",
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
async fn server_created_session_gets_controller_and_waiting_runs_resume_durably() {
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
    let initial_content = serde_json::to_string(&vec![krusty_core::Content::Text {
        text: "legacy objective".into(),
    }])
    .unwrap();
    let db = Database::new(&runtime_config.database_path).unwrap();
    db.conn()
        .execute(
            "INSERT INTO sessions (
                id, title, created_at, updated_at, working_dir, project_dir, session_type
             ) VALUES ('legacy-session', 'Legacy', ?1, ?1, '/work', '/work', 'mako')",
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
                "SELECT COUNT(*) FROM mako_controllers WHERE session_id = 'legacy-session'",
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
            "UPDATE mako_runs SET status = 'sleeping', wake_at = ?1,
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
                "SELECT status FROM mako_runs WHERE session_id = 'legacy-session'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "queued"
    );
    db.conn()
        .execute(
            "UPDATE mako_runs SET status = 'awaiting_input' WHERE session_id = 'legacy-session'",
            [],
        )
        .unwrap();
    drop(db);
    response(
        handler.as_ref(),
        context(Actor::local("test"), "wake-response"),
        Command::UserResponse(UserResponseCommand {
            session_id: "legacy-session".into(),
            tool_call_id: "question-1".into(),
            response: "continue".into(),
        }),
    )
    .await;
    let db = Database::new(&runtime_config.database_path).unwrap();
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT status FROM mako_runs WHERE session_id = 'legacy-session'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        "queued"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn replay_limit_zero_is_live_only_with_an_atomic_high_water() {
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
                "SELECT COUNT(*) FROM mako_runs WHERE status = 'succeeded'",
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
    assert!(matches!(event.event, MakoEvent::Runtime(_)));
    runtime.shutdown().await;
}

#[tokio::test]
async fn execution_events_are_bounded_ordered_replayable_and_drained_before_completion() {
    let temp = TempDir::new().unwrap();
    let mut runtime_config = config(&temp);
    runtime_config.execution_event_capacity = 1;
    runtime_config.max_execution_event_bytes = 512;
    let backend = Arc::new(EventBackend::default());
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
                "SELECT COUNT(*) FROM mako_runs WHERE status = 'succeeded'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap()
            == 1
    })
    .await;
    assert!(backend.oversize_rejected.load(Ordering::SeqCst));

    let db = Database::new(&runtime_config.database_path).unwrap();
    let rows = {
        let mut statement = db
            .conn()
            .prepare(
                "SELECT sequence, event_type, payload_json
                 FROM mako_controller_events ORDER BY sequence",
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
            serde_json::from_str::<serde_json::Value>(payload).unwrap()["index"].as_u64(),
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
            MakoEvent::Extension(ref extension) if extension.name == "agentic_event"
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
async fn stale_daemon_cannot_materialize_or_advance_a_schedule_after_takeover() {
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
        MakoDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
            .get("mako-scheduler")
            .unwrap()
            .is_some()
    })
    .await;
    let stale_lease =
        MakoDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap())
            .get("mako-scheduler")
            .unwrap()
            .unwrap();
    runtime.shutdown().await;

    let lease_store =
        MakoDaemonLeaseStore::new(Database::new(&runtime_config.database_path).unwrap());
    let current = match lease_store
        .acquire(
            "mako-scheduler",
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
        .query_row("SELECT id FROM mako_schedules LIMIT 1", [], |row| {
            row.get::<_, String>(0)
        })
        .unwrap();
    drop(db);
    let schedule = MakoScheduleStore::new(Database::new(&runtime_config.database_path).unwrap())
        .get_schedule(&schedule_id)
        .unwrap()
        .unwrap();
    let scheduled_for =
        krusty_core::mako::parse_utc_timestamp(schedule.next_fire_at.as_deref().unwrap()).unwrap();
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
                "SELECT revision FROM mako_schedules WHERE id = ?1",
                [&schedule_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        db.conn()
            .query_row(
                "SELECT COUNT(*) FROM mako_schedule_occurrences",
                [],
                |row| { row.get::<_, i64>(0) }
            )
            .unwrap(),
        0
    );
}
