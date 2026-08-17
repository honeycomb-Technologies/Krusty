use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
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
    /// Canonical workspace contention boundary for write scheduling.
    pub partition: String,
    /// Provider/auth/endpoint capacity boundary. Backpressure in one domain
    /// must not reduce or pause admission for another domain.
    pub capacity_domain: String,
    pub class: SchedulingClass,
    /// Proven-isolated writers may overlap only within the same logical
    /// operation. Different groups targeting the same workspace still fence
    /// one another.
    pub isolation_group: Option<String>,
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
            capacity_domain: "default".to_string(),
            class,
            isolation_group: None,
            control_lane: false,
        }
    }

    pub fn in_capacity_domain(mut self, domain: impl Into<String>) -> Self {
        let domain = domain.into();
        self.capacity_domain = if domain.trim().is_empty() {
            "default".to_string()
        } else {
            domain
        };
        self
    }

    pub fn in_isolation_group(mut self, group_id: impl Into<String>) -> Self {
        self.isolation_group = Some(group_id.into());
        self
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
    pub capacity_domains: Vec<CapacityDomainSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapacityDomainSnapshot {
    pub domain: String,
    pub target_limit: usize,
    pub active: usize,
    pub queued: usize,
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

    /// Process-wide adaptive admission authority for interactive delegated
    /// work. Pools used to create one scheduler per tool invocation, so every
    /// call believed it owned the full provider/host capacity and could not
    /// share backpressure observations. This shared handle makes concurrency
    /// and cooldown decisions span sessions while durable group policy applies
    /// the narrower per-operation ceiling.
    pub fn shared() -> Self {
        // The host task belongs to the Tokio runtime that first initializes
        // the scheduler. Embedded clients and tests may replace that runtime;
        // never retain a permanently closed process-wide sender.
        static SHARED: OnceLock<Mutex<Option<AgentScheduler>>> = OnceLock::new();
        let mut shared = SHARED
            .get_or_init(|| Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let needs_host = match shared.as_ref() {
            Some(scheduler) => scheduler.tx.is_closed(),
            None => true,
        };
        if needs_host {
            *shared = Some(Self::new(AdaptiveConcurrencyPolicy::default()));
        }
        shared
            .as_ref()
            .expect("shared scheduler initialized")
            .clone()
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
    capacity_domains: HashMap<String, CapacityDomainState>,
    last_session: Option<String>,
    demand_observed: bool,
}

struct CapacityDomainState {
    target_limit: usize,
    healthy_streak: usize,
    cooldown_until: Option<Instant>,
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
        capacity_domains: HashMap::new(),
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
                        let domain_active = active_in_capacity_domain(
                            &state,
                            &request.capacity_domain,
                        );
                        let domain = capacity_domain_mut(&mut state, &request.capacity_domain);
                        if domain_active >= domain.target_limit
                            || domain
                                .cooldown_until
                                .is_some_and(|until| until > Instant::now())
                        {
                            domain.demand_observed = true;
                        }
                        state
                            .pending
                            .push_back(self::Pending { request, response });
                        if state.active.len() >= state.target_limit {
                            state.demand_observed = true;
                        }
                    }
                    Command::Complete { permit_id, signal } => {
                        if let Some(request) = state.active.remove(&permit_id) {
                            apply_signal(&mut state, &request.capacity_domain, signal);
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

fn apply_signal(state: &mut SchedulerState, capacity_domain: &str, signal: BackpressureSignal) {
    match signal {
        BackpressureSignal::Healthy => {
            record_healthy_host_completion(state);
            record_healthy_domain_completion(state, capacity_domain);
        }
        BackpressureSignal::Cancelled | BackpressureSignal::Failed => {}
        BackpressureSignal::Timeout => enter_domain_cooldown(state, capacity_domain, None),
        BackpressureSignal::RateLimited { retry_after }
        | BackpressureSignal::ServiceUnavailable { retry_after }
        | BackpressureSignal::Overloaded { retry_after } => {
            enter_domain_cooldown(state, capacity_domain, retry_after)
        }
    }
}

fn record_healthy_host_completion(state: &mut SchedulerState) {
    state.healthy_streak += 1;
    if state.healthy_streak >= state.policy.healthy_completions_before_ramp && state.demand_observed
    {
        state.target_limit = increased_limit(&state.policy, state.target_limit);
        state.healthy_streak = 0;
        state.demand_observed = false;
    }
}

fn record_healthy_domain_completion(state: &mut SchedulerState, capacity_domain: &str) {
    let healthy_completions_before_ramp = state.policy.healthy_completions_before_ramp;
    let policy = state.policy.clone();
    let domain = capacity_domain_mut(state, capacity_domain);
    domain.healthy_streak += 1;
    if domain.healthy_streak >= healthy_completions_before_ramp && domain.demand_observed {
        domain.target_limit = increased_limit(&policy, domain.target_limit);
        domain.healthy_streak = 0;
        domain.demand_observed = false;
    }
}

fn increased_limit(policy: &AdaptiveConcurrencyPolicy, current: usize) -> usize {
    let proposed = current.saturating_add(policy.ramp_step);
    policy
        .maximum_limit
        .map_or(proposed, |maximum| proposed.min(maximum))
}

fn enter_domain_cooldown(
    state: &mut SchedulerState,
    capacity_domain: &str,
    retry_after: Option<Duration>,
) {
    let minimum_limit = state.policy.minimum_limit;
    let default_cooldown = state.policy.default_cooldown;
    let domain = capacity_domain_mut(state, capacity_domain);
    domain.target_limit = (domain.target_limit / 2).max(minimum_limit);
    domain.healthy_streak = 0;
    domain.demand_observed = false;
    domain.cooldown_until = Some(Instant::now() + retry_after.unwrap_or(default_cooldown));
}

fn dispatch(state: &mut SchedulerState) {
    clear_expired_domain_cooldowns(state);

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
        let control_index = state.pending.iter().position(|pending| {
            pending.request.control_lane && request_runnable(state, &pending.request, true)
        });
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
    let runnable = |pending: &&Pending| request_runnable(state, &pending.request, false);
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

fn request_runnable(state: &SchedulerState, request: &ScheduleRequest, control_lane: bool) -> bool {
    if state
        .active
        .values()
        .any(|active| writer_requests_conflict(active, request))
    {
        return false;
    }

    let Some(domain) = state.capacity_domains.get(&request.capacity_domain) else {
        return true;
    };
    if domain
        .cooldown_until
        .is_some_and(|until| until > Instant::now())
    {
        return false;
    }
    let allowance = usize::from(control_lane);
    active_in_capacity_domain(state, &request.capacity_domain)
        < domain.target_limit.saturating_add(allowance)
}

fn capacity_domain_mut<'a>(
    state: &'a mut SchedulerState,
    capacity_domain: &str,
) -> &'a mut CapacityDomainState {
    let initial_limit = state.policy.initial_limit;
    state
        .capacity_domains
        .entry(capacity_domain.to_string())
        .or_insert(CapacityDomainState {
            target_limit: initial_limit,
            healthy_streak: 0,
            cooldown_until: None,
            demand_observed: false,
        })
}

fn active_in_capacity_domain(state: &SchedulerState, capacity_domain: &str) -> usize {
    state
        .active
        .values()
        .filter(|request| request.capacity_domain.as_str() == capacity_domain)
        .count()
}

fn clear_expired_domain_cooldowns(state: &mut SchedulerState) {
    let now = Instant::now();
    for domain in state.capacity_domains.values_mut() {
        if domain.cooldown_until.is_some_and(|until| until <= now) {
            domain.cooldown_until = None;
        }
    }
}

fn writer_requests_conflict(active: &ScheduleRequest, pending: &ScheduleRequest) -> bool {
    let active_is_writer = matches!(
        active.class,
        SchedulingClass::WriteShared | SchedulingClass::WriteIsolated
    );
    let pending_is_writer = matches!(
        pending.class,
        SchedulingClass::WriteShared | SchedulingClass::WriteIsolated
    );
    if !active_is_writer || !pending_is_writer || active.partition != pending.partition {
        return false;
    }

    match (active.class, pending.class) {
        (SchedulingClass::WriteIsolated, SchedulingClass::WriteIsolated) => {
            active.isolation_group.is_none() || active.isolation_group != pending.isolation_group
        }
        _ => true,
    }
}

fn snapshot(state: &SchedulerState) -> SchedulerSnapshot {
    let now = Instant::now();
    let mut capacity_domains = state
        .capacity_domains
        .iter()
        .map(|(name, domain)| {
            let cooldown_remaining_ms = domain
                .cooldown_until
                .and_then(|until| until.checked_duration_since(now))
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0);
            CapacityDomainSnapshot {
                domain: name.clone(),
                target_limit: domain.target_limit,
                active: active_in_capacity_domain(state, name),
                queued: state
                    .pending
                    .iter()
                    .filter(|pending| pending.request.capacity_domain.as_str() == name.as_str())
                    .count(),
                cooling_down: cooldown_remaining_ms > 0,
                cooldown_remaining_ms,
            }
        })
        .collect::<Vec<_>>();
    capacity_domains.sort_by(|left, right| left.domain.cmp(&right.domain));
    let cooldown_remaining_ms = capacity_domains
        .iter()
        .map(|domain| domain.cooldown_remaining_ms)
        .max()
        .unwrap_or(0);
    SchedulerSnapshot {
        target_limit: state.target_limit,
        active: state.active.len(),
        queued: state.pending.len(),
        peak_active: state.peak_active,
        cooling_down: cooldown_remaining_ms > 0,
        cooldown_remaining_ms,
        capacity_domains,
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
    async fn cloned_handles_share_one_process_capacity_authority() {
        let scheduler = AgentScheduler::new(test_policy(1));
        let cloned = scheduler.clone();
        let cancellation = CancellationToken::new();
        let first = scheduler
            .acquire(
                ScheduleRequest::new("first", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("first permit");

        let waiting_cancellation = cancellation.clone();
        let mut waiting = tokio::spawn(async move {
            cloned
                .acquire(
                    ScheduleRequest::new("second", "provider/model", SchedulingClass::ReadOnly),
                    &waiting_cancellation,
                )
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut waiting)
                .await
                .is_err(),
            "a cloned scheduler handle bypassed the shared process capacity"
        );

        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("shared scheduler released capacity")
            .expect("waiter join")
            .expect("second permit");
        drop(second);
    }

    #[tokio::test]
    async fn independent_scheduler_instances_have_independent_capacity_authorities() {
        let first_scheduler = AgentScheduler::new(test_policy(1));
        let second_scheduler = AgentScheduler::new(test_policy(1));
        let cancellation = CancellationToken::new();

        let first = first_scheduler
            .acquire(
                ScheduleRequest::new("first", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("first authority permit");
        let second = second_scheduler
            .acquire(
                ScheduleRequest::new("second", "provider/model", SchedulingClass::ReadOnly),
                &cancellation,
            )
            .await
            .expect("second authority permit");

        assert_eq!(
            first_scheduler
                .snapshot()
                .await
                .expect("first snapshot")
                .active,
            1
        );
        assert_eq!(
            second_scheduler
                .snapshot()
                .await
                .expect("second snapshot")
                .active,
            1
        );
        drop((first, second));
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
                ScheduleRequest::new("a", "provider/model", SchedulingClass::ReadOnly)
                    .in_capacity_domain("provider-a/auth-a/endpoint-a"),
                &cancellation,
            )
            .await
            .expect("permit");
        permit.complete(BackpressureSignal::RateLimited {
            retry_after: Some(Duration::from_millis(50)),
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        let reduced = scheduler.snapshot().await.expect("snapshot");
        assert_eq!(reduced.target_limit, 8, "host ceiling must remain healthy");
        assert!(reduced.cooling_down);
        let domain = reduced
            .capacity_domains
            .iter()
            .find(|domain| domain.domain == "provider-a/auth-a/endpoint-a")
            .expect("capacity domain snapshot");
        assert_eq!(domain.target_limit, 4);
        assert!(domain.cooling_down);
        tokio::time::sleep(Duration::from_millis(60)).await;
        let recovered = scheduler.snapshot().await.expect("snapshot");
        assert!(!recovered.cooling_down);
    }

    #[tokio::test]
    async fn cooldown_in_one_capacity_domain_does_not_pause_another() {
        let scheduler = AgentScheduler::new(test_policy(8));
        let cancellation = CancellationToken::new();
        let limited = scheduler
            .acquire(
                ScheduleRequest::new("limited", "model", SchedulingClass::ReadOnly)
                    .in_capacity_domain("provider-a/auth-a/endpoint-a"),
                &cancellation,
            )
            .await
            .expect("limited provider permit");
        limited.complete(BackpressureSignal::RateLimited {
            retry_after: Some(Duration::from_millis(100)),
        });
        tokio::time::sleep(Duration::from_millis(5)).await;

        let waiting_scheduler = scheduler.clone();
        let waiting_cancellation = cancellation.clone();
        let mut same_domain = tokio::spawn(async move {
            waiting_scheduler
                .acquire(
                    ScheduleRequest::new("same-domain", "model", SchedulingClass::ReadOnly)
                        .in_capacity_domain("provider-a/auth-a/endpoint-a"),
                    &waiting_cancellation,
                )
                .await
        });
        let healthy_domain = scheduler
            .acquire(
                ScheduleRequest::new("healthy", "model", SchedulingClass::ReadOnly)
                    .in_capacity_domain("provider-b/auth-b/endpoint-b"),
                &cancellation,
            )
            .await
            .expect("healthy provider must not inherit another domain's cooldown");
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut same_domain)
                .await
                .is_err(),
            "same capacity domain bypassed its cooldown"
        );

        let snapshot = scheduler.snapshot().await.expect("snapshot");
        assert_eq!(snapshot.target_limit, 8);
        let limited_domain = snapshot
            .capacity_domains
            .iter()
            .find(|domain| domain.domain == "provider-a/auth-a/endpoint-a")
            .expect("limited domain snapshot");
        let healthy_domain_snapshot = snapshot
            .capacity_domains
            .iter()
            .find(|domain| domain.domain == "provider-b/auth-b/endpoint-b")
            .expect("healthy domain snapshot");
        assert!(limited_domain.cooling_down);
        assert!(!healthy_domain_snapshot.cooling_down);
        assert_eq!(healthy_domain_snapshot.active, 1);

        drop(healthy_domain);
        let recovered = tokio::time::timeout(Duration::from_secs(1), same_domain)
            .await
            .expect("limited domain recovered after its own cooldown")
            .expect("same-domain waiter join")
            .expect("same-domain permit");
        drop(recovered);
    }

    #[tokio::test]
    async fn capacity_domains_do_not_bypass_shared_writer_partition_fence() {
        let scheduler = AgentScheduler::new(test_policy(4));
        let cancellation = CancellationToken::new();
        let first = scheduler
            .acquire(
                ScheduleRequest::new("a", "/workspace", SchedulingClass::WriteShared)
                    .in_capacity_domain("provider-a/auth-a/endpoint-a"),
                &cancellation,
            )
            .await
            .expect("first shared writer");

        let waiting_scheduler = scheduler.clone();
        let waiting_cancellation = cancellation.clone();
        let mut second = tokio::spawn(async move {
            waiting_scheduler
                .acquire(
                    ScheduleRequest::new("b", "/workspace", SchedulingClass::WriteShared)
                        .in_capacity_domain("provider-b/auth-b/endpoint-b"),
                    &waiting_cancellation,
                )
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut second)
                .await
                .is_err(),
            "capacity-domain isolation bypassed the shared workspace fence"
        );
        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(1), second)
            .await
            .expect("shared workspace fence released")
            .expect("second writer join")
            .expect("second writer permit");
        drop(second);
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

    #[tokio::test]
    async fn isolated_siblings_overlap_but_other_workspace_groups_are_fenced() {
        let scheduler = AgentScheduler::new(test_policy(4));
        let cancellation = CancellationToken::new();
        let first = scheduler
            .acquire(
                ScheduleRequest::new("session-a", "/workspace", SchedulingClass::WriteIsolated)
                    .in_isolation_group("group-a"),
                &cancellation,
            )
            .await
            .expect("first isolated writer");
        let second = scheduler
            .acquire(
                ScheduleRequest::new("session-a", "/workspace", SchedulingClass::WriteIsolated)
                    .in_isolation_group("group-a"),
                &cancellation,
            )
            .await
            .expect("sibling isolated writer");

        let waiting_scheduler = scheduler.clone();
        let waiting_cancellation = cancellation.clone();
        let mut other_group = tokio::spawn(async move {
            waiting_scheduler
                .acquire(
                    ScheduleRequest::new("session-b", "/workspace", SchedulingClass::WriteIsolated)
                        .in_isolation_group("group-b"),
                    &waiting_cancellation,
                )
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut other_group)
                .await
                .is_err(),
            "an unrelated isolation group bypassed the workspace fence"
        );
        drop((first, second));
        let admitted = tokio::time::timeout(Duration::from_secs(1), other_group)
            .await
            .expect("workspace fence released")
            .expect("waiter join")
            .expect("other group permit");
        drop(admitted);
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
