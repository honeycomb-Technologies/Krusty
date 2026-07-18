use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct MakoRuntimeConfig {
    pub database_path: PathBuf,
    pub scheduler_poll_interval: Duration,
    pub daemon_lease_duration: Duration,
    pub worker_lease_duration: Duration,
    pub worker_heartbeat_interval: Duration,
    pub global_concurrency_limit: u32,
    pub replay_limit: usize,
    pub live_event_capacity: usize,
    pub subscriber_capacity: usize,
    pub execution_event_capacity: usize,
    pub max_execution_event_bytes: usize,
    pub idempotency_ttl: Duration,
}

impl MakoRuntimeConfig {
    pub fn for_database(database_path: impl Into<PathBuf>) -> Self {
        Self {
            database_path: database_path.into(),
            scheduler_poll_interval: Duration::from_millis(250),
            daemon_lease_duration: Duration::from_secs(15),
            worker_lease_duration: Duration::from_secs(30),
            worker_heartbeat_interval: Duration::from_secs(5),
            global_concurrency_limit: 8,
            replay_limit: 1_000,
            live_event_capacity: 1_024,
            subscriber_capacity: 256,
            execution_event_capacity: 256,
            max_execution_event_bytes: 256 * 1024,
            idempotency_ttl: Duration::from_secs(24 * 60 * 60),
        }
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.scheduler_poll_interval.is_zero(),
            "scheduler poll interval is zero"
        );
        anyhow::ensure!(
            !self.daemon_lease_duration.is_zero(),
            "daemon lease duration is zero"
        );
        anyhow::ensure!(
            !self.worker_lease_duration.is_zero(),
            "worker lease duration is zero"
        );
        anyhow::ensure!(
            !self.worker_heartbeat_interval.is_zero(),
            "worker heartbeat interval is zero"
        );
        anyhow::ensure!(
            self.worker_heartbeat_interval < self.worker_lease_duration,
            "worker heartbeat interval must be shorter than its lease"
        );
        anyhow::ensure!(
            self.global_concurrency_limit > 0,
            "global concurrency limit is zero"
        );
        anyhow::ensure!(self.replay_limit > 0, "event replay limit is zero");
        anyhow::ensure!(
            self.live_event_capacity > 0
                && self.subscriber_capacity > 0
                && self.execution_event_capacity > 0,
            "event channel capacity is zero"
        );
        anyhow::ensure!(
            self.max_execution_event_bytes > 0,
            "execution event byte limit is zero"
        );
        anyhow::ensure!(!self.idempotency_ttl.is_zero(), "idempotency TTL is zero");
        Ok(())
    }
}
