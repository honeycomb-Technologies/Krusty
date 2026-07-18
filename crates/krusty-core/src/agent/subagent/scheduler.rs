use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulingClass {
    ReadOnly,
    WriteShared,
    WriteIsolated,
    Verification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleRequest {
    pub session_id: String,
    pub partition: String,
    pub class: SchedulingClass,
    pub control_lane: bool,
}

impl ScheduleRequest {
    pub fn new(
        session_id: impl Into<String>,
        partition: impl Into<String>,
        class: SchedulingClass,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            partition: partition.into(),
            class,
            control_lane: false,
        }
    }

    pub fn control(mut self) -> Self {
        self.control_lane = true;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressureSignal {
    Healthy,
    Failed,
    Cancelled,
    Timeout,
    RateLimited { retry_after: Option<Duration> },
    ServiceUnavailable { retry_after: Option<Duration> },
    Overloaded { retry_after: Option<Duration> },
}

impl BackpressureSignal {
    pub fn from_error(error: Option<&str>) -> Self {
        let Some(error) = error else {
            return Self::Healthy;
        };
        let normalized = error.to_ascii_lowercase();
        if normalized.contains("cancel") {
            Self::Cancelled
        } else if normalized.contains("429") || normalized.contains("rate limit") {
            Self::RateLimited {
                retry_after: retry_after_hint(&normalized),
            }
        } else if normalized.contains("529") || normalized.contains("overloaded") {
            Self::Overloaded {
                retry_after: retry_after_hint(&normalized),
            }
        } else if normalized.contains("503") || normalized.contains("service unavailable") {
            Self::ServiceUnavailable {
                retry_after: retry_after_hint(&normalized),
            }
        } else if normalized.contains("timeout") || normalized.contains("timed out") {
            Self::Timeout
        } else {
            Self::Failed
        }
    }
}

fn retry_after_hint(message: &str) -> Option<Duration> {
    let marker = "retry-after";
    let suffix = message.split_once(marker)?.1;
    let seconds = suffix
        .trim_start_matches([':', '=', ' '])
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse::<u64>()
        .ok()?;
    Some(Duration::from_secs(seconds))
}

#[derive(Debug, Clone)]
pub struct AdaptiveConcurrencyPolicy {
    pub initial_limit: usize,
    pub minimum_limit: usize,
    pub maximum_limit: Option<usize>,
    pub ramp_step: usize,
    pub healthy_completions_before_ramp: usize,
    pub default_cooldown: Duration,
}

impl Default for AdaptiveConcurrencyPolicy {
    fn default() -> Self {
        let parallelism = std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(4);
        let initial_limit = parallelism.saturating_mul(2).max(8);
        Self {
            initial_limit,
            minimum_limit: 1,
            maximum_limit: Some(initial_limit.saturating_mul(4).max(32)),
            ramp_step: (parallelism / 2).max(1),
            healthy_completions_before_ramp: initial_limit,
            default_cooldown: Duration::from_secs(2),
        }
    }
}

impl AdaptiveConcurrencyPolicy {
    pub fn with_ceiling(mut self, ceiling: usize) -> Self {
        let ceiling = ceiling.max(1);
        self.maximum_limit = Some(ceiling);
        self.initial_limit = self.initial_limit.min(ceiling);
        self.minimum_limit = self.minimum_limit.min(ceiling);
        self
    }

    fn normalized(mut self) -> Self {
        self.minimum_limit = self.minimum_limit.max(1);
        self.initial_limit = self.initial_limit.max(self.minimum_limit);
        self.ramp_step = self.ramp_step.max(1);
        self.healthy_completions_before_ramp = self.healthy_completions_before_ramp.max(1);
        if let Some(maximum) = self.maximum_limit {
            let maximum = maximum.max(self.minimum_limit);
            self.maximum_limit = Some(maximum);
            self.initial_limit = self.initial_limit.min(maximum);
        }
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerSnapshot {
    pub target_limit: usize,
    pub active: usize,
    pub queued: usize,
    pub peak_active: usize,
    pub cooling_down: bool,
    pub cooldown_remaining_ms: u64,
}

#[derive(Clone)]
pub struct AgentScheduler {
    tx: mpsc::UnboundedSender<Command>,
}

impl AgentScheduler {
    pub fn new(policy: AdaptiveConcurrencyPolicy) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(run_scheduler(rx, policy.normalized()));
        Self { tx }
    }

    pub async fn acquire(
        &self,
        request: ScheduleRequest,
        cancellation: &CancellationToken,
    ) -> Option<SchedulerPermit> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(Command::Acquire {
                request,
                response: response_tx,
            })
            .ok()?;
        tokio::select! {
            result = response_rx => result.ok().map(|permit_id| SchedulerPermit {
                permit_id,
                tx: self.tx.clone(),
                completed: false,
            }),
            _ = cancellation.cancelled() => None,
        }
    }

    pub async fn snapshot(&self) -> Option<SchedulerSnapshot> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx.send(Command::Snapshot(response_tx)).ok()?;
        response_rx.await.ok()
    }
}

pub struct SchedulerPermit {
    permit_id: u64,
    tx: mpsc::UnboundedSender<Command>,
    completed: bool,
}

impl SchedulerPermit {
    pub fn complete(mut self, signal: BackpressureSignal) {
        self.completed = true;
        let _ = self.tx.send(Command::Complete {
            permit_id: self.permit_id,
            signal,
        });
    }
}

impl Drop for SchedulerPermit {
    fn drop(&mut self) {
        if !self.completed {
            let _ = self.tx.send(Command::Complete {
                permit_id: self.permit_id,
                signal: BackpressureSignal::Cancelled,
            });
        }
    }
}

enum Command {
    Acquire {
        request: ScheduleRequest,
        response: oneshot::Sender<u64>,
    },
    Complete {
        permit_id: u64,
        signal: BackpressureSignal,
    },
    Snapshot(oneshot::Sender<SchedulerSnapshot>),
}

struct Pending {
    request: ScheduleRequest,
    response: oneshot::Sender<u64>,
}

struct SchedulerState {
    policy: AdaptiveConcurrencyPolicy,
    target_limit: usize,
    next_permit_id: u64,
    active: HashMap<u64, ScheduleRequest>,
    pending: VecDeque<Pending>,
    peak_active: usize,
    healthy_streak: usize,
    cooldown_until: Option<Instant>,
    last_session: Option<String>,
    demand_observed: bool,
}

async fn run_scheduler(
    mut rx: mpsc::UnboundedReceiver<Command>,
    policy: AdaptiveConcurrencyPolicy,
) {
    let mut state = SchedulerState {
        target_limit: policy.initial_limit,
        policy,
        next_permit_id: 1,
        active: HashMap::new(),
        pending: VecDeque::new(),
        peak_active: 0,
        healthy_streak: 0,
        cooldown_until: None,
        last_session: None,
        demand_observed: false,
    };
    let mut ticker = tokio::time::interval(Duration::from_millis(20));

    loop {
        tokio::select! {
            command = rx.recv() => {
                let Some(command) = command else { return; };
                match command {
                    Command::Acquire { request, response } => {
                        state
                            .pending
                            .push_back(self::Pending { request, response });
                        if state.active.len() >= state.target_limit {
                            state.demand_observed = true;
                        }
                    }
                    Command::Complete { permit_id, signal } => {
                        if state.active.remove(&permit_id).is_some() {
                            apply_signal(&mut state, signal);
                        }
                    }
                    Command::Snapshot(response) => {
                        let _ = response.send(snapshot(&state));
                    }
                }
            }
            _ = ticker.tick() => {}
        }
        dispatch(&mut state);
    }
}

fn apply_signal(state: &mut SchedulerState, signal: BackpressureSignal) {
    match signal {
        BackpressureSignal::Healthy => {
            state.healthy_streak += 1;
            if state.healthy_streak >= state.policy.healthy_completions_before_ramp
                && state.demand_observed
            {
                let proposed = state.target_limit.saturating_add(state.policy.ramp_step);
                state.target_limit = state
                    .policy
                    .maximum_limit
                    .map_or(proposed, |maximum| proposed.min(maximum));
                state.healthy_streak = 0;
                state.demand_observed = false;
            }
        }
        BackpressureSignal::Cancelled | BackpressureSignal::Failed => {}
        BackpressureSignal::Timeout => enter_cooldown(state, None),
        BackpressureSignal::RateLimited { retry_after }
        | BackpressureSignal::ServiceUnavailable { retry_after }
        | BackpressureSignal::Overloaded { retry_after } => enter_cooldown(state, retry_after),
    }
}

fn enter_cooldown(state: &mut SchedulerState, retry_after: Option<Duration>) {
    state.target_limit = (state.target_limit / 2).max(state.policy.minimum_limit);
    state.healthy_streak = 0;
    state.demand_observed = false;
    state.cooldown_until =
        Some(Instant::now() + retry_after.unwrap_or(state.policy.default_cooldown));
}

fn dispatch(state: &mut SchedulerState) {
    if state
        .cooldown_until
        .is_some_and(|until| until > Instant::now())
    {
        return;
    }
    state.cooldown_until = None;

    loop {
        state
            .pending
            .retain(|pending| !pending.response.is_closed());
        if state.pending.is_empty() {
            return;
        }

        let active_regular = state
            .active
            .values()
            .filter(|request| !request.control_lane)
            .count();
        let control_index = state
            .pending
            .iter()
            .position(|pending| pending.request.control_lane);
        let selected = if let Some(index) = control_index {
            (state.active.len() < state.target_limit + 1).then_some(index)
        } else if active_regular < state.target_limit {
            fair_runnable_index(state)
        } else {
            None
        };
        let Some(index) = selected else {
            return;
        };
        let pending = state.pending.remove(index).expect("selected queue item");
        let permit_id = state.next_permit_id;
        state.next_permit_id += 1;
        if pending.response.send(permit_id).is_err() {
            continue;
        }
        state.last_session = Some(pending.request.session_id.clone());
        state.active.insert(permit_id, pending.request);
        state.peak_active = state.peak_active.max(state.active.len());
    }
}

fn fair_runnable_index(state: &SchedulerState) -> Option<usize> {
    let runnable = |pending: &&Pending| {
        if pending.request.class != SchedulingClass::WriteShared {
            return true;
        }
        !state.active.values().any(|active| {
            active.class == SchedulingClass::WriteShared
                && active.partition == pending.request.partition
        })
    };
    state
        .pending
        .iter()
        .enumerate()
        .filter(|(_, pending)| runnable(pending))
        .find(|(_, pending)| {
            state.last_session.as_deref() != Some(pending.request.session_id.as_str())
        })
        .or_else(|| {
            state
                .pending
                .iter()
                .enumerate()
                .find(|(_, pending)| runnable(pending))
        })
        .map(|(index, _)| index)
}

fn snapshot(state: &SchedulerState) -> SchedulerSnapshot {
    let cooldown_remaining_ms = state
        .cooldown_until
        .and_then(|until| until.checked_duration_since(Instant::now()))
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    SchedulerSnapshot {
        target_limit: state.target_limit,
        active: state.active.len(),
        queued: state.pending.len(),
        peak_active: state.peak_active,
        cooling_down: cooldown_remaining_ms > 0,
        cooldown_remaining_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_policy(initial_limit: usize) -> AdaptiveConcurrencyPolicy {
        AdaptiveConcurrencyPolicy {
            initial_limit,
            minimum_limit: 1,
            maximum_limit: Some(64),
            ramp_step: 2,
            healthy_completions_before_ramp: 2,
            default_cooldown: Duration::from_millis(40),
        }
    }

    #[tokio::test]
    async fn twelve_healthy_tasks_can_exceed_four_active() {
        let scheduler = AgentScheduler::new(test_policy(12));
        let cancellation = CancellationToken::new();
        let mut permits = Vec::new();
        for index in 0..12 {
            permits.push(
                scheduler
                    .acquire(
                        ScheduleRequest::new(
                            format!("session-{index}"),
                            "provider/model",
                            SchedulingClass::ReadOnly,
                        ),
                        &cancellation,
                    )
                    .await
                    .expect("permit"),
            );
        }
        let snapshot = scheduler.snapshot().await.expect("snapshot");
        assert_eq!(snapshot.active, 12, "snapshot: {snapshot:?}");
        drop(permits);
    }

    #[tokio::test]
    async fn healthy_backlog_ramps_parallelism_gradually() {
        let scheduler = AgentScheduler::new(test_policy(2));
        let cancellation = CancellationToken::new();
        let first = scheduler
            .acquire(
                ScheduleRequest::new("a", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("first");
        let second = scheduler
            .acquire(
                ScheduleRequest::new("b", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("second");
        let queued_scheduler = scheduler.clone();
        let queued_cancel = cancellation.clone();
        let third = tokio::spawn(async move {
            queued_scheduler
                .acquire(
                    ScheduleRequest::new("c", "provider/model", SchedulingClass::ReadOnly),
                    &queued_cancel,
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        first.complete(BackpressureSignal::Healthy);
        second.complete(BackpressureSignal::Healthy);
        let third = tokio::time::timeout(Duration::from_secs(1), third)
            .await
            .expect("ramped capacity")
            .expect("join")
            .expect("third permit");
        assert_eq!(
            scheduler.snapshot().await.expect("snapshot").target_limit,
            4
        );
        drop(third);
    }

    #[tokio::test]
    async fn queued_sessions_receive_fair_turns() {
        let scheduler = AgentScheduler::new(test_policy(1));
        let cancellation = CancellationToken::new();
        let blocker = scheduler
            .acquire(
                ScheduleRequest::new("session-a", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("blocker");
        let (started_tx, mut started_rx) = mpsc::unbounded_channel();
        for session in ["session-a", "session-b"] {
            let scheduler = scheduler.clone();
            let cancellation = cancellation.clone();
            let started_tx = started_tx.clone();
            tokio::spawn(async move {
                let permit = scheduler
                    .acquire(
                        ScheduleRequest::new(session, "provider/model", SchedulingClass::ReadOnly),
                        &cancellation,
                    )
                    .await
                    .expect("queued permit");
                let _ = started_tx.send(session);
                drop(permit);
            });
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
        drop(blocker);
        let first_session = tokio::time::timeout(Duration::from_secs(1), started_rx.recv())
            .await
            .expect("fair dispatch")
            .expect("session");
        assert_eq!(first_session, "session-b");
    }

    #[tokio::test]
    async fn retry_after_halves_pauses_and_then_recovers() {
        let scheduler = AgentScheduler::new(test_policy(8));
        let cancellation = CancellationToken::new();
        let permit = scheduler
            .acquire(
                ScheduleRequest::new("a", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("permit");
        permit.complete(BackpressureSignal::RateLimited {
            retry_after: Some(Duration::from_millis(50)),
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        let reduced = scheduler.snapshot().await.expect("snapshot");
        assert_eq!(reduced.target_limit, 4);
        assert!(reduced.cooling_down);
        tokio::time::sleep(Duration::from_millis(60)).await;
        let recovered = scheduler.snapshot().await.expect("snapshot");
        assert!(!recovered.cooling_down);
    }

    #[tokio::test]
    async fn cancellation_releases_capacity() {
        let scheduler = AgentScheduler::new(test_policy(1));
        let cancellation = CancellationToken::new();
        let first = scheduler
            .acquire(
                ScheduleRequest::new("a", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("first permit");
        let waiting_scheduler = scheduler.clone();
        let waiting_cancel = cancellation.clone();
        let waiter = tokio::spawn(async move {
            waiting_scheduler
                .acquire(
                    ScheduleRequest::new("b", "provider/model", SchedulingClass::ReadOnly),
                    &waiting_cancel,
                )
                .await
        });
        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(1), waiter)
            .await
            .expect("capacity released")
            .expect("join")
            .expect("second permit");
        second.complete(BackpressureSignal::Healthy);
    }

    #[tokio::test]
    async fn control_lane_remains_available_when_worker_lane_is_full() {
        let scheduler = AgentScheduler::new(test_policy(1));
        let cancellation = CancellationToken::new();
        let worker = scheduler
            .acquire(
                ScheduleRequest::new("worker", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("worker permit");
        let control = scheduler
            .acquire(
                ScheduleRequest::new("root", "provider/model", SchedulingClass::Verification)
                    .control(),
                &cancellation,
            )
            .await
            .expect("reserved control permit");
        assert_eq!(scheduler.snapshot().await.expect("snapshot").active, 2);
        drop((worker, control));
    }

    #[test]
    fn backpressure_errors_are_classified() {
        assert!(matches!(
            BackpressureSignal::from_error(Some("HTTP 429 retry-after: 3")),
            BackpressureSignal::RateLimited { retry_after: Some(value) }
                if value == Duration::from_secs(3)
        ));
        assert_eq!(
            BackpressureSignal::from_error(Some("request timed out")),
            BackpressureSignal::Timeout
        );
    }
}
