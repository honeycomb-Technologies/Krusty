//! Session-level durable delegation coordination.
//!
//! The coordinator composes the process-wide adaptive scheduler with durable
//! group/task leases. UI surfaces and tool call implementations should submit
//! logical groups here instead of constructing isolated child pools.

use anyhow::{ensure, Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::agent::subagent::{
    AgentScheduler, BackpressureSignal, ScheduleRequest, SchedulerPermit, SchedulingClass,
};
use crate::ai::providers::ReasoningEffort;
use crate::storage::{
    Database, DelegatedRunRole, DelegationCapacityClass, DelegationCapacityFeedback,
    DelegationCapacityPolicy, DelegationCapacityRequest, DelegationGroupRecord,
    DelegationGroupStartInput, DelegationGroupState, DelegationStore, DelegationSynthesisLease,
    DelegationSynthesisLeaseRenewal, DelegationTaskLease, DelegationTaskLeaseRenewal,
    DelegationTaskRecord, DelegationTaskState,
};
use crate::tools::registry::DelegationPolicy;

const DEFAULT_TASK_LEASE_TTL_MS: i64 = 120_000;
const DEFAULT_SYNTHESIS_LEASE_TTL_MS: i64 = 120_000;
const DEFAULT_CAPACITY_AUTHORITY: &str = "delegation-host-v1";
const RENEWAL_BATCH_COALESCE: Duration = Duration::from_millis(2);
// Ordinary task/group transactions can briefly own SQLite's single writer.
// Give the dedicated renewal connection a small bounded window to absorb that
// expected contention without producing noisy false alarms or shortening the
// effective lease. This remains tiny relative to the minimum renewal slack.
// Lease renewal shares SQLite with trace and workflow writes. A 25 ms window
// produced avoidable lock warnings during healthy parallel groups; this stays
// far below the lease interval while tolerating an ordinary write burst.
const RENEWAL_CONNECTION_BUSY_TIMEOUT: Duration = Duration::from_millis(250);

static RENEWAL_SERVICES: OnceLock<Mutex<HashMap<PathBuf, Weak<LeaseRenewalService>>>> =
    OnceLock::new();
static NEXT_RENEWAL_REGISTRATION_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(test)]
static NEXT_RENEWAL_SERVICE_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct LeaseRenewalServiceHandle {
    db_path: PathBuf,
}

impl LeaseRenewalServiceHandle {
    fn new(db_path: PathBuf) -> Self {
        Self {
            db_path: normalized_db_path(db_path),
        }
    }

    fn register_task(
        &self,
        delegation_task_id: String,
        lease_owner_id: String,
        lease_ttl_ms: i64,
        lease_expires_at_ms: i64,
        lease_lost: CancellationToken,
    ) -> Result<LeaseRenewalRegistration> {
        self.register(
            RenewalTarget::Task {
                delegation_task_id,
                lease_owner_id,
            },
            lease_ttl_ms,
            lease_expires_at_ms,
            lease_lost,
        )
    }

    fn register_synthesis(
        &self,
        delegation_group_id: String,
        lease_owner_id: String,
        lease_ttl_ms: i64,
        lease_expires_at_ms: i64,
        lease_lost: CancellationToken,
    ) -> Result<LeaseRenewalRegistration> {
        self.register(
            RenewalTarget::Synthesis {
                delegation_group_id,
                lease_owner_id,
            },
            lease_ttl_ms,
            lease_expires_at_ms,
            lease_lost,
        )
    }

    fn register(
        &self,
        target: RenewalTarget,
        lease_ttl_ms: i64,
        lease_expires_at_ms: i64,
        lease_lost: CancellationToken,
    ) -> Result<LeaseRenewalRegistration> {
        ensure!(lease_ttl_ms > 0, "delegation lease TTL must be positive");
        let mut last_error = None;
        for _ in 0..2 {
            let service = renewal_service_for(&self.db_path)?;
            match service.register(
                target.clone(),
                lease_ttl_ms,
                lease_expires_at_ms,
                lease_lost.clone(),
            ) {
                Ok(registration) => return Ok(registration),
                Err(error) => {
                    service.alive.store(false, Ordering::Release);
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("renewal service unavailable")))
    }
}

#[derive(Debug, Clone)]
enum RenewalTarget {
    Task {
        delegation_task_id: String,
        lease_owner_id: String,
    },
    Synthesis {
        delegation_group_id: String,
        lease_owner_id: String,
    },
}

struct RenewalEntry {
    target: RenewalTarget,
    lease_ttl_ms: i64,
    interval: Duration,
    covered_until: Instant,
    next_due: Instant,
    lease_lost: CancellationToken,
}

enum RenewalCommand {
    Register { id: u64, entry: RenewalEntry },
    Deregister { id: u64 },
}

#[derive(Default)]
struct LeaseRenewalServiceStats {
    connections_opened: AtomicUsize,
    batch_cycles: AtomicUsize,
    batch_errors: AtomicUsize,
    renewed_items: AtomicUsize,
}

struct LeaseRenewalService {
    sender: mpsc::Sender<RenewalCommand>,
    alive: Arc<AtomicBool>,
    registered_tokens: Arc<Mutex<HashMap<u64, CancellationToken>>>,
    #[cfg(test)]
    generation: u64,
    #[cfg(test)]
    stats: Arc<LeaseRenewalServiceStats>,
}

impl LeaseRenewalService {
    fn spawn(db_path: PathBuf) -> Result<Arc<Self>> {
        let (sender, receiver) = mpsc::channel();
        let alive = Arc::new(AtomicBool::new(true));
        let stats = Arc::new(LeaseRenewalServiceStats::default());
        let registered_tokens = Arc::new(Mutex::new(HashMap::new()));
        let service = Arc::new(Self {
            sender,
            alive: alive.clone(),
            registered_tokens: registered_tokens.clone(),
            #[cfg(test)]
            generation: NEXT_RENEWAL_SERVICE_GENERATION.fetch_add(1, Ordering::Relaxed),
            #[cfg(test)]
            stats: stats.clone(),
        });
        std::thread::Builder::new()
            .name("mitsuro-lease-renewal".to_string())
            .spawn(move || {
                let actor_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_lease_renewal_actor(db_path, receiver, &stats);
                }));
                if actor_result.is_err() {
                    tracing::error!("Durable lease renewal actor panicked; cancelling owners");
                }
                alive.store(false, Ordering::Release);
                if let Ok(mut tokens) = registered_tokens.lock() {
                    for (_, token) in tokens.drain() {
                        token.cancel();
                    }
                }
            })
            .context("spawning durable lease renewal actor")?;
        Ok(service)
    }

    fn register(
        self: &Arc<Self>,
        target: RenewalTarget,
        lease_ttl_ms: i64,
        lease_expires_at_ms: i64,
        lease_lost: CancellationToken,
    ) -> Result<LeaseRenewalRegistration> {
        ensure!(
            self.alive.load(Ordering::Acquire),
            "durable lease renewal actor stopped"
        );
        let id = NEXT_RENEWAL_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed);
        let interval = renewal_interval(lease_ttl_ms);
        let now = Instant::now();
        let remaining_ms = lease_expires_at_ms.saturating_sub(Utc::now().timestamp_millis());
        let covered_until = now + Duration::from_millis(remaining_ms.max(0) as u64);
        self.registered_tokens
            .lock()
            .map_err(|_| anyhow::anyhow!("renewal registration token registry was poisoned"))?
            .insert(id, lease_lost.clone());
        if let Err(error) = self.sender.send(RenewalCommand::Register {
            id,
            entry: RenewalEntry {
                target,
                lease_ttl_ms,
                interval,
                covered_until,
                next_due: (now + interval).min(covered_until),
                lease_lost,
            },
        }) {
            if let Ok(mut tokens) = self.registered_tokens.lock() {
                tokens.remove(&id);
            }
            return Err(error).context("registering durable lease renewal");
        }
        Ok(LeaseRenewalRegistration {
            id,
            service: self.clone(),
            active: true,
        })
    }
}

struct LeaseRenewalRegistration {
    id: u64,
    service: Arc<LeaseRenewalService>,
    active: bool,
}

impl LeaseRenewalRegistration {
    fn deregister(&mut self) {
        if self.active {
            if let Ok(mut tokens) = self.service.registered_tokens.lock() {
                tokens.remove(&self.id);
            }
            let _ = self
                .service
                .sender
                .send(RenewalCommand::Deregister { id: self.id });
            self.active = false;
        }
    }
}

impl Drop for LeaseRenewalRegistration {
    fn drop(&mut self) {
        self.deregister();
    }
}

#[derive(Debug, Clone)]
pub enum DelegationTaskOutcome {
    Complete(Value),
    Degraded { artifact: Value, reason: String },
    Failed { error: String },
    Cancelled,
}

pub struct CoordinatedSynthesisPermit {
    db_path: PathBuf,
    lease: DelegationSynthesisLease,
    lease_ttl_ms: i64,
    lease_lost: CancellationToken,
    renewal_registration: Option<LeaseRenewalRegistration>,
    completed: bool,
}

/// Cloneable exact-owner fence for a synchronous repository publication
/// boundary. This is deliberately separate from background heartbeat work and
/// is intended for a `spawn_blocking` closure immediately before side effects.
#[derive(Clone)]
pub struct CoordinatedSynthesisOwnerFence {
    db_path: PathBuf,
    delegation_group_id: String,
    lease_owner_id: String,
    lease_ttl_ms: i64,
}

impl CoordinatedSynthesisOwnerFence {
    pub fn renew_current(&self) -> Result<()> {
        let renewed = DelegationStore::new(Database::new(&self.db_path)?).renew_synthesis_lease(
            &self.delegation_group_id,
            &self.lease_owner_id,
            self.lease_ttl_ms,
        )?;
        ensure!(renewed, "delegation synthesis owner fence was lost");
        Ok(())
    }
}

impl CoordinatedSynthesisPermit {
    pub fn group(&self) -> &DelegationGroupRecord {
        &self.lease.group
    }

    /// Cancels when durable synthesis ownership is lost. Long-running patch
    /// integration or aggregate construction should stop at safe boundaries.
    pub fn cancellation(&self) -> CancellationToken {
        self.lease_lost.clone()
    }

    pub fn owner_fence(&self) -> CoordinatedSynthesisOwnerFence {
        CoordinatedSynthesisOwnerFence {
            db_path: self.db_path.clone(),
            delegation_group_id: self.lease.group.delegation_group_id.clone(),
            lease_owner_id: self.lease.lease_owner_id.clone(),
            lease_ttl_ms: self.lease_ttl_ms,
        }
    }

    pub fn finalize(
        mut self,
        terminal_state: DelegationGroupState,
    ) -> Result<DelegationGroupRecord> {
        ensure!(
            terminal_state.is_terminal(),
            "delegation synthesis finalization requires a terminal state"
        );
        let store = DelegationStore::new(Database::new(&self.db_path)?);
        let persisted = store.complete_synthesis(
            &self.lease.group.delegation_group_id,
            &self.lease.lease_owner_id,
            terminal_state,
        )?;
        // Keep renewing through the fenced publication transaction. Stopping
        // first creates a lock-contention window where the lease can expire
        // after side effects are complete but before their aggregate is owned.
        self.stop_heartbeat();
        ensure!(
            persisted,
            "delegation synthesis lost its durable completion fence"
        );
        self.completed = true;
        store
            .get_group(&self.lease.group.delegation_group_id)?
            .context("finalized delegation group disappeared")
    }

    fn stop_heartbeat(&mut self) {
        self.renewal_registration.take();
    }
}

impl Drop for CoordinatedSynthesisPermit {
    fn drop(&mut self) {
        if !self.completed {
            self.stop_heartbeat();
        }
    }
}

impl DelegationTaskOutcome {
    fn task_state(&self) -> DelegationTaskState {
        match self {
            Self::Complete(_) => DelegationTaskState::Complete,
            Self::Degraded { .. } => DelegationTaskState::Degraded,
            Self::Failed { .. } => DelegationTaskState::Failed,
            Self::Cancelled => DelegationTaskState::Cancelled,
        }
    }

    fn artifact(&self) -> Option<&Value> {
        match self {
            Self::Complete(artifact) | Self::Degraded { artifact, .. } => Some(artifact),
            Self::Failed { .. } | Self::Cancelled => None,
        }
    }

    fn error_summary(&self) -> Option<&str> {
        match self {
            Self::Degraded { reason, .. } => Some(reason),
            Self::Failed { error } => Some(error),
            Self::Complete(_) | Self::Cancelled => None,
        }
    }

    fn backpressure_signal(&self) -> BackpressureSignal {
        match self {
            Self::Complete(_) | Self::Degraded { .. } => BackpressureSignal::Healthy,
            Self::Failed { error } => BackpressureSignal::from_error(Some(error)),
            Self::Cancelled => BackpressureSignal::Cancelled,
        }
    }
}

#[derive(Clone)]
pub struct DelegationCoordinator {
    db_path: PathBuf,
    scheduler: AgentScheduler,
    renewal_service: LeaseRenewalServiceHandle,
    task_lease_ttl_ms: i64,
    synthesis_lease_ttl_ms: i64,
    durable_capacity_policy: DelegationCapacityPolicy,
}

impl DelegationCoordinator {
    pub fn new(db_path: impl Into<PathBuf>) -> Self {
        let db_path = db_path.into();
        Self {
            renewal_service: LeaseRenewalServiceHandle::new(db_path.clone()),
            db_path,
            scheduler: AgentScheduler::shared(),
            task_lease_ttl_ms: DEFAULT_TASK_LEASE_TTL_MS,
            synthesis_lease_ttl_ms: DEFAULT_SYNTHESIS_LEASE_TTL_MS,
            durable_capacity_policy: DelegationCapacityPolicy::default(),
        }
    }

    pub fn with_scheduler(db_path: impl Into<PathBuf>, scheduler: AgentScheduler) -> Self {
        let db_path = db_path.into();
        Self {
            renewal_service: LeaseRenewalServiceHandle::new(db_path.clone()),
            db_path,
            scheduler,
            task_lease_ttl_ms: DEFAULT_TASK_LEASE_TTL_MS,
            synthesis_lease_ttl_ms: DEFAULT_SYNTHESIS_LEASE_TTL_MS,
            durable_capacity_policy: DelegationCapacityPolicy::default(),
        }
    }

    pub fn with_task_lease_ttl_ms(mut self, task_lease_ttl_ms: i64) -> Self {
        self.task_lease_ttl_ms = task_lease_ttl_ms.max(1);
        self
    }

    pub fn with_synthesis_lease_ttl_ms(mut self, synthesis_lease_ttl_ms: i64) -> Self {
        self.synthesis_lease_ttl_ms = synthesis_lease_ttl_ms.max(1);
        self
    }

    pub fn with_durable_capacity_policy(
        mut self,
        durable_capacity_policy: DelegationCapacityPolicy,
    ) -> Self {
        self.durable_capacity_policy = durable_capacity_policy;
        self
    }

    fn store(&self) -> Result<DelegationStore> {
        Ok(DelegationStore::new(Database::new(&self.db_path)?))
    }

    pub fn create_group(&self, input: &DelegationGroupStartInput) -> Result<DelegationGroupRecord> {
        let store = self.store()?;
        store.create_group(input)?;
        store.queue_group(&input.delegation_group_id)
    }

    pub fn get_group(&self, delegation_group_id: &str) -> Result<Option<DelegationGroupRecord>> {
        self.store()?.get_group(delegation_group_id)
    }

    pub fn complete_task_integration(
        &self,
        delegation_task_id: &str,
        succeeded: bool,
        error_summary: Option<&str>,
    ) -> Result<bool> {
        self.store()?
            .complete_task_integration(delegation_task_id, succeeded, error_summary)
    }

    pub fn fail_unstarted_tasks(
        &self,
        delegation_group_id: &str,
        delegation_task_ids: &[String],
        error_summary: &str,
    ) -> Result<usize> {
        self.store()?
            .fail_unstarted_tasks(delegation_group_id, delegation_task_ids, error_summary)
    }

    pub fn refresh_isolated_task_baseline(
        &self,
        delegation_task_id: &str,
        expected_workspace: &Path,
        workspace_baseline: &str,
    ) -> Result<bool> {
        self.store()?.refresh_isolated_task_baseline(
            delegation_task_id,
            &expected_workspace.display().to_string(),
            workspace_baseline,
        )
    }

    pub fn validate_task_runtime(
        &self,
        delegation_task_id: &str,
        runtime_policy: Option<&DelegationPolicy>,
        runtime_reasoning_effort: Option<ReasoningEffort>,
        working_dir: &Path,
    ) -> Result<()> {
        let store = self.store()?;
        let task = store
            .get_task(delegation_task_id)?
            .with_context(|| format!("unknown delegation task '{delegation_task_id}'"))?;
        let group = store
            .get_group(&task.delegation_group_id)?
            .context("delegation task group disappeared")?;
        let runtime_policy = runtime_policy
            .context("coordinated delegated execution requires runtime policy metadata")?;
        let expected_policy = task
            .specification
            .task_policy
            .as_ref()
            .unwrap_or(&group.contract.governance.delegation_policy);
        ensure!(
            runtime_policy == expected_policy,
            "runtime delegation policy exceeds or differs from the immutable task contract"
        );
        ensure!(
            expected_policy.is_within(&group.contract.governance.delegation_policy),
            "immutable task policy exceeds its group governance"
        );
        ensure!(
            runtime_policy.inherited_permission_mode == group.contract.governance.permission_mode,
            "runtime permission mode differs from immutable delegation governance"
        );
        ensure!(
            runtime_reasoning_effort == group.contract.governance.reasoning_effort,
            "runtime reasoning effort differs from immutable delegation governance"
        );
        if let Some(group_budget) = group.contract.governance.delegated_turn_budget {
            ensure!(
                runtime_policy.max_turns.unwrap_or(group_budget) <= group_budget,
                "runtime turn budget exceeds immutable delegation governance"
            );
        }
        match task.specification.writer_mode {
            crate::storage::DelegationWriterMode::Isolated => {
                let root = task
                    .specification
                    .attempt_workspace
                    .as_deref()
                    .context("isolated writer has no durable attempt workspace")?;
                ensure!(
                    working_dir.starts_with(root),
                    "isolated writer runtime escaped its durable attempt workspace"
                );
            }
            crate::storage::DelegationWriterMode::Shared => {
                if let Some(workspace) = task
                    .specification
                    .target_scope
                    .iter()
                    .find(|scope| scope.kind == "workspace")
                {
                    ensure!(
                        working_dir.starts_with(&workspace.path),
                        "shared writer runtime escaped its governed workspace"
                    );
                }
            }
        }
        Ok(())
    }

    /// Immediate recovery finalization for an aggregate artifact that is
    /// already durable. Normal execution should retain the permit returned by
    /// `begin_synthesis` across patch integration and aggregate persistence.
    pub fn finalize_group(
        &self,
        delegation_group_id: &str,
        terminal_state: DelegationGroupState,
    ) -> Result<DelegationGroupRecord> {
        ensure!(
            terminal_state.is_terminal(),
            "delegation group finalization requires a terminal state"
        );
        let store = self.store()?;
        let mut group = store
            .get_group(delegation_group_id)?
            .with_context(|| format!("unknown delegation group '{delegation_group_id}'"))?;
        if group.state.is_terminal() {
            return Ok(group);
        }
        if group.state != DelegationGroupState::ReadyForParent {
            let _ = store.reconcile_group(delegation_group_id)?;
            group = store
                .get_group(delegation_group_id)?
                .context("reconciled delegation group disappeared")?;
        }
        if group.state.is_terminal() {
            return Ok(group);
        }
        let owner_id = Uuid::new_v4().to_string();
        let lease = store
            .claim_synthesis(delegation_group_id, &owner_id, self.synthesis_lease_ttl_ms)?
            .context("delegation synthesis is owned by another live coordinator")?;
        ensure!(
            store.complete_synthesis(delegation_group_id, &lease.lease_owner_id, terminal_state,)?,
            "delegation synthesis lost its durable completion fence"
        );
        store
            .get_group(delegation_group_id)?
            .context("finalized delegation group disappeared")
    }

    /// Claim the aggregate synthesis boundary after logical task settlement.
    /// A subsequent crash leaves an explicit recoverable Synthesizing state
    /// rather than an ambiguous ReadyForParent snapshot.
    pub fn begin_synthesis(&self, delegation_group_id: &str) -> Result<CoordinatedSynthesisPermit> {
        let store = self.store()?;
        let mut group = store
            .get_group(delegation_group_id)?
            .with_context(|| format!("unknown delegation group '{delegation_group_id}'"))?;
        if group.state == DelegationGroupState::Running {
            let _ = store.reconcile_group(delegation_group_id)?;
            group = store
                .get_group(delegation_group_id)?
                .context("reconciled delegation group disappeared")?;
        }
        ensure!(
            group.state == DelegationGroupState::ReadyForParent
                || group.state == DelegationGroupState::Synthesizing,
            "delegation group is not ready for synthesis"
        );
        let lease_owner_id = Uuid::new_v4().to_string();
        let lease = store
            .claim_synthesis(
                delegation_group_id,
                &lease_owner_id,
                self.synthesis_lease_ttl_ms,
            )?
            .context("delegation synthesis is owned by another live coordinator")?;
        let lease_lost = CancellationToken::new();
        let renewal_registration = self.renewal_service.register_synthesis(
            delegation_group_id.to_string(),
            lease_owner_id,
            self.synthesis_lease_ttl_ms,
            lease.lease_expires_at_ms,
            lease_lost.clone(),
        )?;
        Ok(CoordinatedSynthesisPermit {
            db_path: self.db_path.clone(),
            lease,
            lease_ttl_ms: self.synthesis_lease_ttl_ms,
            lease_lost,
            renewal_registration: Some(renewal_registration),
            completed: false,
        })
    }

    /// Acquire one durable task and one process-wide provider permit. Durable
    /// group admission is evaluated first; a cancelled scheduler wait releases
    /// an unstarted claim immediately.
    pub async fn acquire_next(
        &self,
        delegation_group_id: &str,
        resolved_model: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<CoordinatedTaskPermit>> {
        let lease_owner_id = Uuid::new_v4().to_string();
        let store = self.store()?;
        let mut claimed = store.claim_tasks(
            delegation_group_id,
            &lease_owner_id,
            1,
            self.task_lease_ttl_ms,
        )?;
        let Some(lease) = claimed.pop() else {
            return Ok(None);
        };
        self.activate_lease(lease, resolved_model, cancellation)
            .await
    }

    /// Wait for capacity for one already-materialized logical task. The
    /// durable state is checked on each bounded wait so cancellation, terminal
    /// policy, or another owner winning the task cannot strand this worker.
    pub async fn acquire_task(
        &self,
        delegation_task_id: &str,
        resolved_model: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<CoordinatedTaskPermit>> {
        self.acquire_task_with_lifecycle(delegation_task_id, resolved_model, cancellation, |_| {})
            .await
    }

    /// Acquire one durable logical task while reporting exact persisted task
    /// lifecycle transitions to a live observer. The callback is advisory UI
    /// projection only; the delegation store remains the lifecycle authority.
    pub async fn acquire_task_with_lifecycle<F>(
        &self,
        delegation_task_id: &str,
        resolved_model: &str,
        cancellation: &CancellationToken,
        mut on_state: F,
    ) -> Result<Option<CoordinatedTaskPermit>>
    where
        F: FnMut(DelegationTaskState) + Send,
    {
        let mut last_observed_state = None;
        let mut poll_delay = Duration::from_millis(50);
        loop {
            let lease_owner_id = Uuid::new_v4().to_string();
            let store = self.store()?;
            if let Some(lease) =
                store.claim_task(delegation_task_id, &lease_owner_id, self.task_lease_ttl_ms)?
            {
                on_state(DelegationTaskState::Leased);
                let permit = self
                    .activate_lease(lease, resolved_model, cancellation)
                    .await?;
                if permit.is_some() {
                    on_state(DelegationTaskState::Running);
                }
                return Ok(permit);
            }
            let task = store
                .get_task(delegation_task_id)?
                .with_context(|| format!("unknown delegation task '{delegation_task_id}'"))?;
            if last_observed_state != Some(task.state) {
                on_state(task.state);
                last_observed_state = Some(task.state);
                poll_delay = Duration::from_millis(50);
            }
            let group = store
                .get_group(&task.delegation_group_id)?
                .context("delegation task group disappeared")?;
            if task.state.is_terminal()
                || group.state.is_terminal()
                || group.state == DelegationGroupState::ReadyForParent
            {
                let _ = store.reconcile_group(&task.delegation_group_id)?;
                return Ok(None);
            }
            tokio::select! {
                _ = cancellation.cancelled() => return Ok(None),
                _ = tokio::time::sleep(poll_delay) => {
                    poll_delay = poll_delay.saturating_mul(2).min(Duration::from_secs(1));
                }
            }
        }
    }

    async fn activate_lease(
        &self,
        mut lease: DelegationTaskLease,
        resolved_model: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<CoordinatedTaskPermit>> {
        // Start renewal before any database/configuration work. Short test
        // leases and a busy SQLite migration must not create an unprotected
        // gap between the durable claim and scheduler/capacity admission.
        let execution_cancellation = cancellation.child_token();
        let mut renewal_registration = match self.renewal_service.register_task(
            lease.task.specification.delegation_task_id.clone(),
            lease.lease_owner_id.clone(),
            self.task_lease_ttl_ms,
            lease.lease_expires_at_ms,
            execution_cancellation.clone(),
        ) {
            Ok(registration) => Some(registration),
            Err(error) => {
                self.store()?.release_task_claim(
                    &lease.task.specification.delegation_task_id,
                    &lease.lease_owner_id,
                )?;
                return Err(error);
            }
        };
        let store = match self.store() {
            Ok(store) => store,
            Err(error) => {
                renewal_registration.take();
                return Err(error);
            }
        };
        let group = match store.get_group(&lease.task.delegation_group_id) {
            Ok(Some(group)) => group,
            Ok(None) => {
                renewal_registration.take();
                anyhow::bail!("claimed delegation group disappeared");
            }
            Err(error) => {
                renewal_registration.take();
                return Err(error);
            }
        };
        let request = ScheduleRequest::new(
            group.parent_session_id,
            scheduler_partition(&lease.task, resolved_model),
            scheduling_class(&lease.task),
        )
        // The resolved model is the strongest provider-capacity identity the
        // coordinator currently receives. Keep its adaptive backpressure
        // independent from other model/provider domains; a later transport
        // contract can refine this to provider/auth/endpoint without changing
        // durable task identity.
        .in_capacity_domain(resolved_model.to_string())
        .in_isolation_group(lease.task.delegation_group_id.clone());
        let durable_request = DelegationCapacityRequest {
            authority_key: DEFAULT_CAPACITY_AUTHORITY.to_string(),
            // Provisional domain identity: the coordinator currently receives
            // only the resolved model. A provider/auth/endpoint key should
            // replace this once the transport contract exposes one.
            domain_key: resolved_model.to_string(),
            partition_key: scheduler_partition(&lease.task, resolved_model),
            scheduling_class: durable_scheduling_class(&lease.task),
            isolation_group: Some(lease.task.delegation_group_id.clone()),
        };
        // The durable lease starts before global admission. Acquire a local
        // permit for each database admission attempt, but release it whenever
        // the durable authority says to wait. Otherwise a cooldown written by
        // another process could fill every local slot with sleeping tasks and
        // starve unrelated healthy domains.
        let mut admission_delay = Duration::from_millis(50);
        let scheduler_permit = loop {
            let Some(scheduler_permit) = self
                .scheduler
                .acquire(request.clone(), &execution_cancellation)
                .await
            else {
                renewal_registration.take();
                store.release_task_claim(
                    &lease.task.specification.delegation_task_id,
                    &lease.lease_owner_id,
                )?;
                return Ok(None);
            };
            match store.try_admit_and_start_task(
                &lease.task.specification.delegation_task_id,
                &lease.lease_owner_id,
                resolved_model,
                &durable_request,
                self.durable_capacity_policy,
            ) {
                Ok(true) => break scheduler_permit,
                Ok(false) => drop(scheduler_permit),
                Err(error) => {
                    renewal_registration.take();
                    drop(scheduler_permit);
                    return Err(error);
                }
            }
            tokio::select! {
                _ = execution_cancellation.cancelled() => {
                    renewal_registration.take();
                    store.release_task_claim(
                        &lease.task.specification.delegation_task_id,
                        &lease.lease_owner_id,
                    )?;
                    return Ok(None);
                },
                _ = tokio::time::sleep(admission_delay) => {
                    admission_delay = admission_delay
                        .saturating_mul(2)
                        .min(Duration::from_secs(1));
                }
            }
        };
        lease.task = store
            .get_task(&lease.task.specification.delegation_task_id)?
            .context("started delegation task disappeared")?;

        Ok(Some(CoordinatedTaskPermit {
            db_path: self.db_path.clone(),
            lease,
            scheduler_permit: Some(scheduler_permit),
            execution_cancellation,
            renewal_registration,
            completed: false,
        }))
    }

    /// Startup/reconnect recovery entrypoint. Claiming performs expired-lease
    /// reconciliation transactionally; returning no permit means the group is
    /// either capacity-bound, waiting on live owners, or ready for its parent.
    pub async fn recover_next(
        &self,
        delegation_group_id: &str,
        resolved_model: &str,
        cancellation: &CancellationToken,
    ) -> Result<Option<CoordinatedTaskPermit>> {
        self.acquire_next(delegation_group_id, resolved_model, cancellation)
            .await
    }
}

pub struct CoordinatedTaskPermit {
    db_path: PathBuf,
    lease: DelegationTaskLease,
    scheduler_permit: Option<SchedulerPermit>,
    execution_cancellation: CancellationToken,
    renewal_registration: Option<LeaseRenewalRegistration>,
    completed: bool,
}

impl CoordinatedTaskPermit {
    pub fn task(&self) -> &DelegationTaskRecord {
        &self.lease.task
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.execution_cancellation.clone()
    }

    pub fn complete(mut self, outcome: DelegationTaskOutcome) -> Result<DelegationGroupState> {
        let store = DelegationStore::new(Database::new(&self.db_path)?);
        let signal = outcome.backpressure_signal();
        let persisted = store.complete_task_with_capacity_feedback(
            &self.lease.task.specification.delegation_task_id,
            &self.lease.lease_owner_id,
            outcome.task_state(),
            outcome.artifact(),
            outcome.error_summary(),
            durable_capacity_feedback(signal),
        )?;
        // The heartbeat remains live through the completion CAS so a busy
        // database cannot turn a finished side effect into an expired attempt
        // that another owner is allowed to replay.
        self.stop_heartbeat();
        if let Some(permit) = self.scheduler_permit.take() {
            permit.complete(signal);
        }
        ensure!(
            persisted,
            "delegation task lost its durable completion fence"
        );
        self.completed = true;
        Ok(store
            .get_group(&self.lease.task.delegation_group_id)?
            .context("completed delegation group disappeared")?
            .state)
    }
}

impl Drop for CoordinatedTaskPermit {
    fn drop(&mut self) {
        // Scheduler capacity is always released by SchedulerPermit's Drop. A
        // started durable task deliberately remains fenced until lease expiry;
        // another process cannot assume that its side effects stopped merely
        // because this Rust future was dropped.
        if !self.completed {
            self.stop_heartbeat();
            self.scheduler_permit.take();
        }
    }
}

impl CoordinatedTaskPermit {
    fn stop_heartbeat(&mut self) {
        self.renewal_registration.take();
    }
}

fn normalized_db_path(db_path: PathBuf) -> PathBuf {
    if let Ok(canonical) = db_path.canonicalize() {
        return canonical;
    }
    if db_path.is_absolute() {
        db_path
    } else {
        std::env::current_dir()
            .map(|current| current.join(&db_path))
            .unwrap_or(db_path)
    }
}

fn renewal_service_for(db_path: &Path) -> Result<Arc<LeaseRenewalService>> {
    let key = normalized_db_path(db_path.to_path_buf());
    let registry = RENEWAL_SERVICES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = registry
        .lock()
        .map_err(|_| anyhow::anyhow!("durable lease renewal registry was poisoned"))?;
    if let Some(service) = registry.get(&key).and_then(Weak::upgrade) {
        if service.alive.load(Ordering::Acquire) {
            return Ok(service);
        }
    }
    let service = LeaseRenewalService::spawn(key.clone())?;
    registry.insert(key, Arc::downgrade(&service));
    Ok(service)
}

fn renewal_interval(lease_ttl_ms: i64) -> Duration {
    Duration::from_millis((lease_ttl_ms.max(1) as u64 / 3).clamp(1, 2_000))
}

fn run_lease_renewal_actor(
    db_path: PathBuf,
    receiver: mpsc::Receiver<RenewalCommand>,
    stats: &LeaseRenewalServiceStats,
) {
    let mut entries = HashMap::<u64, RenewalEntry>::new();
    let mut database = None;
    loop {
        let command = if entries.is_empty() {
            match receiver.recv() {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        } else {
            let now = Instant::now();
            let next_due = entries
                .values()
                .map(|entry| entry.next_due)
                .min()
                .unwrap_or(now);
            match receiver.recv_timeout(next_due.saturating_duration_since(now)) {
                Ok(command) => Some(command),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return,
            }
        };
        if let Some(command) = command {
            apply_renewal_command(&mut entries, command);
            while let Ok(command) = receiver.try_recv() {
                apply_renewal_command(&mut entries, command);
            }
        }
        if entries
            .values()
            .any(|entry| entry.next_due <= Instant::now() + RENEWAL_BATCH_COALESCE)
        {
            renew_due_entries(&db_path, &mut database, &mut entries, stats);
        }
    }
}

fn apply_renewal_command(entries: &mut HashMap<u64, RenewalEntry>, command: RenewalCommand) {
    match command {
        RenewalCommand::Register { id, entry } => {
            entries.insert(id, entry);
        }
        RenewalCommand::Deregister { id } => {
            entries.remove(&id);
        }
    }
}

fn renew_due_entries(
    db_path: &Path,
    connection: &mut Option<Connection>,
    entries: &mut HashMap<u64, RenewalEntry>,
    stats: &LeaseRenewalServiceStats,
) {
    let now = Instant::now();
    let cutoff = now + RENEWAL_BATCH_COALESCE;
    let due = entries
        .iter()
        .filter_map(|(id, entry)| (entry.next_due <= cutoff).then_some((*id, entry.target.clone())))
        .collect::<Vec<_>>();
    if due.is_empty() {
        return;
    }
    let mut task_ids = Vec::new();
    let mut task_renewals = Vec::new();
    let mut synthesis_ids = Vec::new();
    let mut synthesis_renewals = Vec::new();
    for (id, target) in &due {
        let Some(entry) = entries.get(id) else {
            continue;
        };
        match target {
            RenewalTarget::Task {
                delegation_task_id,
                lease_owner_id,
            } => {
                task_ids.push(*id);
                task_renewals.push(DelegationTaskLeaseRenewal {
                    delegation_task_id: delegation_task_id.clone(),
                    lease_owner_id: lease_owner_id.clone(),
                    lease_ttl_ms: entry.lease_ttl_ms,
                });
            }
            RenewalTarget::Synthesis {
                delegation_group_id,
                lease_owner_id,
            } => {
                synthesis_ids.push(*id);
                synthesis_renewals.push(DelegationSynthesisLeaseRenewal {
                    delegation_group_id: delegation_group_id.clone(),
                    lease_owner_id: lease_owner_id.clone(),
                    lease_ttl_ms: entry.lease_ttl_ms,
                });
            }
        }
    }
    stats.batch_cycles.fetch_add(1, Ordering::Relaxed);
    let result = (|| -> Result<_> {
        if connection.is_none() {
            let db = Connection::open(db_path).context("opening renewal actor database")?;
            db.busy_timeout(RENEWAL_CONNECTION_BUSY_TIMEOUT)
                .context("configuring renewal actor busy timeout")?;
            db.pragma_update(None, "foreign_keys", "ON")
                .context("enabling renewal actor foreign keys")?;
            stats.connections_opened.fetch_add(1, Ordering::Relaxed);
            *connection = Some(db);
        }
        DelegationStore::renew_lease_batch_on_connection(
            connection
                .as_ref()
                .context("renewal actor database disappeared")?,
            &task_renewals,
            &synthesis_renewals,
        )
    })();

    match result {
        Ok(result) => {
            for (id, renewed) in task_ids
                .into_iter()
                .zip(result.task_renewed)
                .chain(synthesis_ids.into_iter().zip(result.synthesis_renewed))
            {
                apply_renewal_result(entries, id, renewed, now, stats);
            }
        }
        Err(error) => {
            stats.batch_errors.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(%error, due = due.len(), "Transient batched delegation lease renewal failure");
            let failed_at = Instant::now();
            for (id, _) in due {
                apply_renewal_error(entries, id, failed_at);
            }
        }
    }
}

fn apply_renewal_result(
    entries: &mut HashMap<u64, RenewalEntry>,
    id: u64,
    renewed: bool,
    renewal_started_at: Instant,
    stats: &LeaseRenewalServiceStats,
) {
    let Some(entry) = entries.get_mut(&id) else {
        return;
    };
    if renewed {
        entry.covered_until =
            renewal_started_at + Duration::from_millis(entry.lease_ttl_ms.max(1) as u64);
        entry.next_due = renewal_started_at + entry.interval;
        stats.renewed_items.fetch_add(1, Ordering::Relaxed);
    } else if let Some(entry) = entries.remove(&id) {
        entry.lease_lost.cancel();
    }
}

fn apply_renewal_error(entries: &mut HashMap<u64, RenewalEntry>, id: u64, now: Instant) {
    let Some(entry) = entries.get_mut(&id) else {
        return;
    };
    if now >= entry.covered_until {
        if let Some(entry) = entries.remove(&id) {
            entry.lease_lost.cancel();
        }
    } else {
        let remaining = entry.covered_until.saturating_duration_since(now);
        entry.next_due = now + entry.interval.min(remaining).max(Duration::from_millis(1));
    }
}

fn scheduling_class(task: &DelegationTaskRecord) -> SchedulingClass {
    match task.specification.role.clone() {
        DelegatedRunRole::Explore | DelegatedRunRole::Planner => SchedulingClass::ReadOnly,
        DelegatedRunRole::Build
            if task.specification.writer_mode == crate::storage::DelegationWriterMode::Isolated =>
        {
            SchedulingClass::WriteIsolated
        }
        DelegatedRunRole::Build => SchedulingClass::WriteShared,
        DelegatedRunRole::Verifier => SchedulingClass::Verification,
    }
}

fn durable_scheduling_class(task: &DelegationTaskRecord) -> DelegationCapacityClass {
    match scheduling_class(task) {
        SchedulingClass::ReadOnly => DelegationCapacityClass::ReadOnly,
        SchedulingClass::WriteShared => DelegationCapacityClass::WriteShared,
        SchedulingClass::WriteIsolated => DelegationCapacityClass::WriteIsolated,
        SchedulingClass::Verification => DelegationCapacityClass::Verification,
    }
}

fn durable_capacity_feedback(signal: BackpressureSignal) -> DelegationCapacityFeedback {
    let millis = |duration: Option<Duration>| {
        duration.map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
    };
    match signal {
        BackpressureSignal::Healthy => DelegationCapacityFeedback::Healthy,
        BackpressureSignal::Failed | BackpressureSignal::Cancelled => {
            DelegationCapacityFeedback::Neutral
        }
        BackpressureSignal::Timeout => DelegationCapacityFeedback::Timeout,
        BackpressureSignal::RateLimited { retry_after } => {
            DelegationCapacityFeedback::RateLimited {
                retry_after_ms: millis(retry_after),
            }
        }
        BackpressureSignal::ServiceUnavailable { retry_after } => {
            DelegationCapacityFeedback::ServiceUnavailable {
                retry_after_ms: millis(retry_after),
            }
        }
        BackpressureSignal::Overloaded { retry_after } => DelegationCapacityFeedback::Overloaded {
            retry_after_ms: millis(retry_after),
        },
    }
}

fn scheduler_partition(task: &DelegationTaskRecord, resolved_model: &str) -> String {
    match task.specification.role.clone() {
        DelegatedRunRole::Build => task
            .specification
            .target_scope
            .iter()
            .find(|scope| scope.kind == "workspace")
            .map(|scope| scope.path.clone())
            .unwrap_or_else(|| task.delegation_group_id.clone()),
        DelegatedRunRole::Explore | DelegatedRunRole::Planner | DelegatedRunRole::Verifier => {
            resolved_model.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::subagent::AdaptiveConcurrencyPolicy;
    use crate::storage::{
        DelegationCompletionPolicy, DelegationExecutionMode, DelegationFailurePolicy,
        DelegationGovernance, DelegationGroupContract, DelegationTaskSpec,
    };
    use crate::tools::registry::PermissionMode;
    use chrono::Utc;
    use rusqlite::params;
    use std::time::Duration;
    use tempfile::TempDir;

    fn coordinator() -> (DelegationCoordinator, TempDir) {
        let temp_dir = TempDir::new().expect("temp dir");
        let db_path = temp_dir.path().join("coordinator.db");
        let db = Database::new(&db_path).expect("db");
        let now = Utc::now().to_rfc3339();
        db.conn()
            .execute(
                "INSERT INTO sessions (id, title, created_at, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params!["session-1", "Coordinator", now, now],
            )
            .expect("seed session");
        drop(db);
        let scheduler = AgentScheduler::new(AdaptiveConcurrencyPolicy {
            initial_limit: 4,
            minimum_limit: 1,
            maximum_limit: Some(4),
            ramp_step: 1,
            healthy_completions_before_ramp: 4,
            default_cooldown: Duration::from_millis(10),
        });
        (
            DelegationCoordinator::with_scheduler(db_path, scheduler),
            temp_dir,
        )
    }

    fn group_input() -> DelegationGroupStartInput {
        DelegationGroupStartInput {
            delegation_group_id: "group-1".to_string(),
            parent_session_id: "session-1".to_string(),
            parent_tool_call_id: Some("tool-1".to_string()),
            contract: DelegationGroupContract {
                execution_mode: DelegationExecutionMode::Detached,
                completion_policy: DelegationCompletionPolicy::AllSettled,
                failure_policy: DelegationFailurePolicy::Continue,
                governance: DelegationGovernance {
                    permission_mode: PermissionMode::Autonomous,
                    reasoning_effort: None,
                    delegated_turn_budget: Some(8),
                    max_parallelism: 2,
                    execution_tool_allowlist: None,
                    delegation_policy:
                        crate::tools::registry::DelegationPolicy::for_subagent_explore(
                            PermissionMode::Autonomous,
                            Some(8),
                        ),
                },
            },
            tasks: (0..3)
                .map(|index| DelegationTaskSpec {
                    delegation_task_id: format!("task-{index}"),
                    task_key: format!("task-{index}"),
                    objective: format!("Objective {index}"),
                    role: DelegatedRunRole::Explore,
                    target_scope: Vec::new(),
                    max_attempts: 2,
                    depends_on: Vec::new(),
                    write_intent: Vec::new(),
                    task_policy: None,
                    writer_mode: crate::storage::DelegationWriterMode::Shared,
                    attempt_workspace: None,
                    workspace_baseline: None,
                    executor_envelope: None,
                })
                .collect(),
        }
    }

    fn group_input_for(group_id: &str, task_count: usize) -> DelegationGroupStartInput {
        let mut input = group_input();
        input.delegation_group_id = group_id.to_string();
        input.contract.governance.max_parallelism = task_count.max(1);
        input.tasks = (0..task_count)
            .map(|index| DelegationTaskSpec {
                delegation_task_id: format!("{group_id}-task-{index}"),
                task_key: format!("task-{index}"),
                objective: format!("Objective {index}"),
                role: DelegatedRunRole::Explore,
                target_scope: Vec::new(),
                max_attempts: 2,
                depends_on: Vec::new(),
                write_intent: Vec::new(),
                task_policy: None,
                writer_mode: crate::storage::DelegationWriterMode::Shared,
                attempt_workspace: None,
                workspace_baseline: None,
                executor_envelope: None,
            })
            .collect();
        input
    }

    fn claim_test_leases(
        coordinator: &DelegationCoordinator,
        group_id: &str,
        task_count: usize,
        owner_id: &str,
        lease_ttl_ms: i64,
    ) -> Vec<DelegationTaskLease> {
        coordinator
            .create_group(&group_input_for(group_id, task_count))
            .expect("create renewal test group");
        coordinator
            .store()
            .expect("renewal test store")
            .claim_tasks(group_id, owner_id, task_count, lease_ttl_ms)
            .expect("claim renewal test tasks")
    }

    async fn wait_until(mut predicate: impl FnMut() -> bool) {
        tokio::time::timeout(Duration::from_secs(10), async {
            while !predicate() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("timed out waiting for renewal actor state");
    }

    #[tokio::test]
    async fn durable_group_ceiling_and_shared_scheduler_compose() {
        let (coordinator, _temp_dir) = coordinator();
        coordinator
            .create_group(&group_input())
            .expect("create group");
        let cancellation = CancellationToken::new();

        let first = coordinator
            .acquire_next("group-1", "provider/model", &cancellation)
            .await
            .expect("first claim")
            .expect("first permit");
        let second = coordinator
            .acquire_next("group-1", "provider/model", &cancellation)
            .await
            .expect("second claim")
            .expect("second permit");
        assert!(coordinator
            .acquire_next("group-1", "provider/model", &cancellation)
            .await
            .expect("bounded claim")
            .is_none());

        assert_eq!(
            first
                .complete(DelegationTaskOutcome::Complete(
                    serde_json::json!({"ok": 0})
                ))
                .expect("complete first"),
            DelegationGroupState::Running
        );
        let third = coordinator
            .acquire_next("group-1", "provider/model", &cancellation)
            .await
            .expect("third claim")
            .expect("third permit");
        second
            .complete(DelegationTaskOutcome::Complete(
                serde_json::json!({"ok": 1}),
            ))
            .expect("complete second");
        assert_eq!(
            third
                .complete(DelegationTaskOutcome::Complete(
                    serde_json::json!({"ok": 2})
                ))
                .expect("complete third"),
            DelegationGroupState::ReadyForParent
        );
    }

    #[tokio::test]
    async fn independent_process_schedulers_cannot_bypass_durable_capacity() {
        let (coordinator, _temp_dir) = coordinator();
        let durable_policy = DelegationCapacityPolicy {
            initial_limit: 1,
            minimum_limit: 1,
            maximum_limit: 4,
            ramp_step: 1,
            healthy_completions_before_ramp: 4,
            default_cooldown_ms: 50,
        };
        let first_authority = coordinator
            .clone()
            .with_durable_capacity_policy(durable_policy);
        let independent_scheduler = AgentScheduler::new(AdaptiveConcurrencyPolicy {
            initial_limit: 4,
            minimum_limit: 1,
            maximum_limit: Some(4),
            ramp_step: 1,
            healthy_completions_before_ramp: 4,
            default_cooldown: Duration::from_millis(10),
        });
        let second_authority = DelegationCoordinator::with_scheduler(
            coordinator.db_path.clone(),
            independent_scheduler,
        )
        .with_durable_capacity_policy(durable_policy);
        first_authority
            .create_group(&group_input())
            .expect("create group");
        let cancellation = CancellationToken::new();
        let first = first_authority
            .acquire_task("task-0", "provider/model", &cancellation)
            .await
            .expect("first acquisition")
            .expect("first permit");

        let second_cancel = cancellation.clone();
        let mut waiting = tokio::spawn(async move {
            second_authority
                .acquire_task("task-1", "provider/model", &second_cancel)
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err(),
            "an independent scheduler bypassed the database capacity ceiling"
        );
        first
            .complete(DelegationTaskOutcome::Complete(
                serde_json::json!({"ok": 0}),
            ))
            .expect("complete first");
        let second = tokio::time::timeout(Duration::from_secs(2), waiting)
            .await
            .expect("durable slot released")
            .expect("join second")
            .expect("second acquisition")
            .expect("second permit");
        second
            .complete(DelegationTaskOutcome::Complete(
                serde_json::json!({"ok": 1}),
            ))
            .expect("complete second");
    }

    #[tokio::test]
    async fn durable_cooldown_waiter_does_not_occupy_local_scheduler_slot() {
        let (seed, _temp_dir) = coordinator();
        seed.create_group(&group_input()).expect("create group");
        let now_ms = Utc::now().timestamp_millis();
        let db = Database::new(&seed.db_path).expect("open capacity database");
        db.conn()
            .execute(
                "INSERT INTO delegation_capacity_hosts (
                    authority_key, target_limit, minimum_limit, maximum_limit,
                    ramp_step, healthy_threshold, default_cooldown_ms, updated_at_ms
                 ) VALUES (?1, 2, 1, 4, 1, 4, 1000, ?2)",
                params![DEFAULT_CAPACITY_AUTHORITY, now_ms],
            )
            .expect("seed host authority");
        db.conn()
            .execute(
                "INSERT INTO delegation_capacity_domains (
                    authority_key, domain_key, target_limit, cooldown_until_ms, updated_at_ms
                 ) VALUES (?1, 'model-cooled', 1, ?2, ?3)",
                params![
                    DEFAULT_CAPACITY_AUTHORITY,
                    now_ms.saturating_add(10_000),
                    now_ms
                ],
            )
            .expect("seed cooled domain");
        drop(db);
        let one_slot_scheduler = AgentScheduler::new(AdaptiveConcurrencyPolicy {
            initial_limit: 1,
            minimum_limit: 1,
            maximum_limit: Some(1),
            ramp_step: 1,
            healthy_completions_before_ramp: 4,
            default_cooldown: Duration::from_millis(10),
        });
        let authority =
            DelegationCoordinator::with_scheduler(seed.db_path.clone(), one_slot_scheduler);
        let cooled_cancel = CancellationToken::new();
        let cooled_authority = authority.clone();
        let cooled_child_cancel = cooled_cancel.clone();
        let cooled = tokio::spawn(async move {
            cooled_authority
                .acquire_task("task-0", "model-cooled", &cooled_child_cancel)
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;

        let healthy_cancel = CancellationToken::new();
        let healthy = tokio::time::timeout(
            Duration::from_secs(2),
            authority.acquire_task("task-1", "model-healthy", &healthy_cancel),
        )
        .await
        .expect("healthy domain was starved by cooled waiter")
        .expect("healthy acquisition")
        .expect("healthy permit");
        healthy
            .complete(DelegationTaskOutcome::Complete(
                serde_json::json!({"ok": true}),
            ))
            .expect("complete healthy task");
        cooled_cancel.cancel();
        assert!(cooled
            .await
            .expect("join cooled")
            .expect("cooled result")
            .is_none());
    }

    #[tokio::test]
    async fn expired_owner_is_recovered_with_the_same_logical_task() {
        let (coordinator, _temp_dir) = coordinator();
        // Keep acquisition independent from full-suite scheduler load, then
        // model the crashed owner deterministically below.
        let coordinator = coordinator.with_task_lease_ttl_ms(5_000);
        coordinator
            .create_group(&group_input())
            .expect("create group");
        let cancellation = CancellationToken::new();
        let first = coordinator
            .acquire_next("group-1", "provider/model", &cancellation)
            .await
            .expect("initial claim")
            .expect("initial permit");
        assert_eq!(first.task().attempt_count, 1);
        let first_task_id = first.task().specification.delegation_task_id.clone();
        drop(first);

        Database::new(&coordinator.db_path)
            .expect("crash simulation database")
            .conn()
            .execute(
                "UPDATE delegation_tasks
                    SET lease_owner_id = 'simulated-crashed-owner', lease_expires_at_ms = 0
                  WHERE delegation_task_id = ?1",
                params![first_task_id],
            )
            .expect("expire simulated crashed owner");
        let recovered = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(permit) = coordinator
                    .recover_next("group-1", "provider/model", &cancellation)
                    .await
                    .expect("recovery claim")
                {
                    break permit;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("expired owner was not reclaimed");
        assert_eq!(
            recovered.task().specification.delegation_task_id,
            first_task_id
        );
        assert_eq!(recovered.task().attempt_count, 2);
    }

    #[tokio::test]
    async fn specific_task_waits_for_durable_group_capacity() {
        let (coordinator, _temp_dir) = coordinator();
        let mut input = group_input();
        input.contract.governance.max_parallelism = 1;
        coordinator.create_group(&input).expect("create group");
        let cancellation = CancellationToken::new();
        let first = coordinator
            .acquire_task("task-0", "provider/model", &cancellation)
            .await
            .expect("first claim")
            .expect("first permit");

        let waiting_coordinator = coordinator.clone();
        let waiting_cancellation = cancellation.clone();
        let mut waiting = tokio::spawn(async move {
            waiting_coordinator
                .acquire_task("task-1", "provider/model", &waiting_cancellation)
                .await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut waiting)
                .await
                .is_err(),
            "second logical task bypassed the durable group ceiling"
        );
        first
            .complete(DelegationTaskOutcome::Complete(
                serde_json::json!({"task": 0}),
            ))
            .expect("complete first");
        let second = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("second task admitted")
            .expect("waiter join")
            .expect("second claim")
            .expect("second permit");
        assert_eq!(second.task().specification.delegation_task_id, "task-1");
    }

    #[tokio::test]
    async fn aggregate_finalization_is_first_writer_wins() {
        let (coordinator, _temp_dir) = coordinator();
        let mut input = group_input();
        input.tasks.truncate(1);
        input.contract.governance.max_parallelism = 1;
        coordinator.create_group(&input).expect("create group");
        let cancellation = CancellationToken::new();
        let task = coordinator
            .acquire_task("task-0", "provider/model", &cancellation)
            .await
            .expect("claim")
            .expect("permit");
        assert_eq!(
            task.complete(DelegationTaskOutcome::Complete(
                serde_json::json!({"task": 0}),
            ))
            .expect("complete task"),
            DelegationGroupState::ReadyForParent
        );
        let completed = coordinator
            .finalize_group("group-1", DelegationGroupState::Complete)
            .expect("finalize group");
        assert_eq!(completed.state, DelegationGroupState::Complete);
        let late_failure = coordinator
            .finalize_group("group-1", DelegationGroupState::Failed)
            .expect("late finalizer reads authority");
        assert_eq!(late_failure.state, DelegationGroupState::Complete);
    }

    #[tokio::test]
    async fn synthesis_owner_fence_renews_only_the_exact_owner() {
        let (coordinator, _temp_dir) = coordinator();
        let mut input = group_input();
        input.tasks.truncate(1);
        input.contract.governance.max_parallelism = 1;
        coordinator.create_group(&input).expect("create group");
        let cancellation = CancellationToken::new();
        coordinator
            .acquire_task("task-0", "provider/model", &cancellation)
            .await
            .expect("claim")
            .expect("permit")
            .complete(DelegationTaskOutcome::Complete(
                serde_json::json!({"task": 0}),
            ))
            .expect("complete task");
        let permit = coordinator
            .begin_synthesis("group-1")
            .expect("begin synthesis");
        let fence = permit.owner_fence();
        fence.renew_current().expect("renew exact synthesis owner");
        Database::new(&coordinator.db_path)
            .expect("owner mutation database")
            .conn()
            .execute(
                "UPDATE delegation_groups SET synthesis_owner_id = 'replacement-owner'
                  WHERE delegation_group_id = 'group-1'",
                [],
            )
            .expect("replace synthesis owner");
        fence
            .renew_current()
            .expect_err("stale synthesis fence must fail closed");
        drop(permit);
    }

    #[tokio::test]
    async fn runtime_policy_cannot_exceed_the_immutable_group_contract() {
        let (coordinator, _temp_dir) = coordinator();
        coordinator
            .create_group(&group_input())
            .expect("create group");
        let expected = crate::tools::registry::DelegationPolicy::for_subagent_explore(
            PermissionMode::Autonomous,
            Some(8),
        );
        coordinator
            .validate_task_runtime("task-0", Some(&expected), None, std::path::Path::new("."))
            .expect("exact policy");

        let broader = crate::tools::registry::DelegationPolicy::for_subagent_child(
            PermissionMode::Autonomous,
            Some(8),
            true,
            true,
            true,
        );
        coordinator
            .validate_task_runtime("task-0", Some(&broader), None, std::path::Path::new("."))
            .expect_err("broader runtime policy must fail closed");
    }

    #[tokio::test]
    async fn shared_actor_batches_many_task_capacity_and_synthesis_renewals_once() {
        let (coordinator, _temp_dir) = coordinator();
        let task_count = 24;
        let owner_id = "batch-owner";
        let leases = claim_test_leases(&coordinator, "batch-group", task_count, owner_id, 10_000);
        let store = coordinator.store().expect("batch store");
        let capacity_policy = DelegationCapacityPolicy {
            initial_limit: task_count,
            minimum_limit: 1,
            maximum_limit: task_count,
            ramp_step: 1,
            healthy_completions_before_ramp: task_count,
            default_cooldown_ms: 100,
        };
        for lease in &leases {
            let task_id = &lease.task.specification.delegation_task_id;
            assert!(store
                .try_admit_and_start_task(
                    task_id,
                    owner_id,
                    "batch-model",
                    &DelegationCapacityRequest {
                        authority_key: DEFAULT_CAPACITY_AUTHORITY.to_string(),
                        domain_key: "batch-model".to_string(),
                        partition_key: task_id.clone(),
                        scheduling_class: DelegationCapacityClass::ReadOnly,
                        isolation_group: Some("batch-group".to_string()),
                    },
                    capacity_policy,
                )
                .expect("start batch task"));
        }

        coordinator
            .create_group(&group_input_for("batch-synthesis", 1))
            .expect("create synthesis group");
        Database::new(&coordinator.db_path)
            .expect("synthesis state database")
            .conn()
            .execute(
                "UPDATE delegation_groups SET state = 'ready_for_parent'
                  WHERE delegation_group_id = 'batch-synthesis'",
                [],
            )
            .expect("make synthesis group ready");
        let synthesis = store
            .claim_synthesis("batch-synthesis", "batch-synth-owner", 10_000)
            .expect("claim synthesis")
            .expect("synthesis lease");

        let mut registrations = Vec::new();
        for lease in &leases {
            registrations.push(
                coordinator
                    .renewal_service
                    .register_task(
                        lease.task.specification.delegation_task_id.clone(),
                        owner_id.to_string(),
                        1_200,
                        lease.lease_expires_at_ms,
                        CancellationToken::new(),
                    )
                    .expect("register batch task"),
            );
        }
        registrations.push(
            coordinator
                .renewal_service
                .register_synthesis(
                    "batch-synthesis".to_string(),
                    synthesis.lease_owner_id,
                    1_200,
                    synthesis.lease_expires_at_ms,
                    CancellationToken::new(),
                )
                .expect("register batch synthesis"),
        );
        let service = registrations[0].service.clone();
        wait_until(|| service.stats.renewed_items.load(Ordering::Acquire) == task_count + 1).await;
        assert_eq!(service.stats.connections_opened.load(Ordering::Acquire), 1);
        assert_eq!(service.stats.batch_cycles.load(Ordering::Acquire), 1);

        let renewed_capacity: i64 = Database::new(&coordinator.db_path)
            .expect("capacity inspection database")
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM delegation_capacity_leases
                  WHERE lease_expires_at_ms > (CAST(strftime('%s', 'now') AS INTEGER) * 1000)",
                [],
                |row| row.get(0),
            )
            .expect("count renewed capacity leases");
        assert_eq!(renewed_capacity, task_count as i64);
        drop(registrations);
    }

    #[tokio::test]
    async fn renewal_actor_graces_transient_busy_until_a_later_success() {
        let (coordinator, _temp_dir) = coordinator();
        let lease = claim_test_leases(&coordinator, "busy-group", 1, "busy-owner", 5_000)
            .pop()
            .expect("busy lease");
        let lease_lost = CancellationToken::new();
        let registration = coordinator
            .renewal_service
            .register_task(
                lease.task.specification.delegation_task_id.clone(),
                "busy-owner".to_string(),
                3_000,
                lease.lease_expires_at_ms,
                lease_lost.clone(),
            )
            .expect("register busy lease");
        let service = registration.service.clone();
        wait_until(|| service.stats.renewed_items.load(Ordering::Acquire) >= 1).await;
        let renewed_before = service.stats.renewed_items.load(Ordering::Acquire);
        let errors_before = service.stats.batch_errors.load(Ordering::Acquire);
        let blocker = Database::new(&coordinator.db_path).expect("busy blocker database");
        blocker
            .conn()
            .execute_batch("BEGIN IMMEDIATE")
            .expect("hold database writer lock");
        wait_until(|| service.stats.batch_errors.load(Ordering::Acquire) > errors_before).await;
        assert!(!lease_lost.is_cancelled());
        blocker
            .conn()
            .execute_batch("ROLLBACK")
            .expect("release database writer lock");
        wait_until(|| service.stats.renewed_items.load(Ordering::Acquire) > renewed_before).await;
        assert!(!lease_lost.is_cancelled());
        drop(registration);
    }

    #[tokio::test]
    async fn renewal_actor_cancels_only_the_exact_lost_owner() {
        let (coordinator, _temp_dir) = coordinator();
        let leases = claim_test_leases(&coordinator, "owner-loss", 2, "shared-owner", 5_000);
        let first_lost = CancellationToken::new();
        let second_lost = CancellationToken::new();
        let first = coordinator
            .renewal_service
            .register_task(
                leases[0].task.specification.delegation_task_id.clone(),
                "shared-owner".to_string(),
                1_500,
                leases[0].lease_expires_at_ms,
                first_lost.clone(),
            )
            .expect("register first owner");
        let second = coordinator
            .renewal_service
            .register_task(
                leases[1].task.specification.delegation_task_id.clone(),
                "shared-owner".to_string(),
                1_500,
                leases[1].lease_expires_at_ms,
                second_lost.clone(),
            )
            .expect("register second owner");
        Database::new(&coordinator.db_path)
            .expect("owner mutation database")
            .conn()
            .execute(
                "UPDATE delegation_tasks SET lease_owner_id = 'replacement-owner'
                  WHERE delegation_task_id = ?1",
                params![leases[0].task.specification.delegation_task_id],
            )
            .expect("replace first owner");
        wait_until(|| first_lost.is_cancelled()).await;
        assert!(!second_lost.is_cancelled());
        drop((first, second));
    }

    #[tokio::test]
    async fn dropped_registration_is_not_renewed_or_cancelled() {
        let (coordinator, _temp_dir) = coordinator();
        let lease = claim_test_leases(&coordinator, "deregister", 1, "drop-owner", 360)
            .pop()
            .expect("deregister lease");
        let task_id = lease.task.specification.delegation_task_id.clone();
        let initial_expiry = lease.lease_expires_at_ms;
        let lease_lost = CancellationToken::new();
        let registration = coordinator
            .renewal_service
            .register_task(
                task_id.clone(),
                "drop-owner".to_string(),
                360,
                lease.lease_expires_at_ms,
                lease_lost.clone(),
            )
            .expect("register dropped lease");
        drop(registration);
        tokio::time::sleep(Duration::from_millis(180)).await;
        let persisted_expiry: i64 = Database::new(&coordinator.db_path)
            .expect("deregister inspection database")
            .conn()
            .query_row(
                "SELECT lease_expires_at_ms FROM delegation_tasks WHERE delegation_task_id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .expect("read deregistered expiry");
        assert_eq!(persisted_expiry, initial_expiry);
        assert!(!lease_lost.is_cancelled());
    }

    #[tokio::test]
    async fn stale_renewal_runtime_is_recreated_for_the_same_database() {
        let (coordinator, _temp_dir) = coordinator();
        let leases = claim_test_leases(&coordinator, "recreate", 2, "recreate-owner", 5_000);
        let first = coordinator
            .renewal_service
            .register_task(
                leases[0].task.specification.delegation_task_id.clone(),
                "recreate-owner".to_string(),
                900,
                leases[0].lease_expires_at_ms,
                CancellationToken::new(),
            )
            .expect("register first runtime");
        let first_generation = first.service.generation;
        let first_alive = first.service.alive.clone();
        drop(first);
        wait_until(|| !first_alive.load(Ordering::Acquire)).await;

        let second = coordinator
            .renewal_service
            .register_task(
                leases[1].task.specification.delegation_task_id.clone(),
                "recreate-owner".to_string(),
                900,
                leases[1].lease_expires_at_ms,
                CancellationToken::new(),
            )
            .expect("register recreated runtime");
        assert_ne!(second.service.generation, first_generation);
        let second_service = second.service.clone();
        wait_until(|| second_service.stats.renewed_items.load(Ordering::Acquire) >= 1).await;
        assert_eq!(
            second_service
                .stats
                .connections_opened
                .load(Ordering::Acquire),
            1
        );
        drop(second);
    }
}
