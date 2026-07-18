use std::path::PathBuf;
use std::time::Duration;

const MAX_LIVE_EVENT_CAPACITY: usize = 128;
const MAX_SUBSCRIBER_CAPACITY: usize = 32;
const MAX_EXECUTION_EVENT_CAPACITY: usize = 64;
const MAX_EXECUTION_EVENT_BYTES: usize = 64 * 1024;
const MAX_SINGLE_PIPELINE_BUFFER_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct MakoRuntimeConfig {
    pub database_path: PathBuf,
    pub scheduler_poll_interval: Duration,
    pub daemon_lease_duration: Duration,
    pub worker_lease_duration: Duration,
    pub worker_heartbeat_interval: Duration,
    pub cancellation_grace_period: Duration,
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
            cancellation_grace_period: Duration::from_secs(2),
            global_concurrency_limit: 8,
            replay_limit: 1_000,
            live_event_capacity: 64,
            subscriber_capacity: 16,
            execution_event_capacity: 32,
            max_execution_event_bytes: 32 * 1024,
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
            self.scheduler_poll_interval < self.daemon_lease_duration,
            "scheduler poll interval must be shorter than the daemon lease"
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
            !self.cancellation_grace_period.is_zero(),
            "cancellation grace period is zero"
        );
        anyhow::ensure!(
            self.cancellation_grace_period < self.worker_lease_duration,
            "cancellation grace period must be shorter than the worker lease"
        );
        let cancellation_terminalization_budget = self
            .cancellation_grace_period
            .checked_mul(2)
            .ok_or_else(|| anyhow::anyhow!("cancellation terminalization budget overflow"))?;
        anyhow::ensure!(
            cancellation_terminalization_budget < self.worker_lease_duration,
            "cancellation grace plus abort delivery must fit inside the worker lease"
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
            self.live_event_capacity <= MAX_LIVE_EVENT_CAPACITY
                && self.subscriber_capacity <= MAX_SUBSCRIBER_CAPACITY
                && self.execution_event_capacity <= MAX_EXECUTION_EVENT_CAPACITY,
            "event channel capacity exceeds the bounded Mako runtime limits"
        );
        anyhow::ensure!(
            self.max_execution_event_bytes > 0
                && self.max_execution_event_bytes <= MAX_EXECUTION_EVENT_BYTES,
            "execution event byte limit is outside the bounded Mako runtime range"
        );
        let buffered_slots = self
            .live_event_capacity
            .checked_add(self.subscriber_capacity)
            .and_then(|value| value.checked_add(self.execution_event_capacity))
            .ok_or_else(|| anyhow::anyhow!("event channel capacity overflow"))?;
        let buffered_bytes = buffered_slots
            .checked_mul(self.max_execution_event_bytes)
            .ok_or_else(|| anyhow::anyhow!("event buffer byte budget overflow"))?;
        anyhow::ensure!(
            buffered_bytes <= MAX_SINGLE_PIPELINE_BUFFER_BYTES,
            "event buffer byte budget exceeds the bounded Mako runtime limit"
        );
        anyhow::ensure!(!self.idempotency_ttl.is_zero(), "idempotency TTL is zero");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        MakoRuntimeConfig, MAX_EXECUTION_EVENT_BYTES, MAX_EXECUTION_EVENT_CAPACITY,
        MAX_LIVE_EVENT_CAPACITY, MAX_SUBSCRIBER_CAPACITY,
    };

    #[test]
    fn scheduler_must_renew_before_its_daemon_lease_expires() {
        let mut config = MakoRuntimeConfig::for_database("runtime.db");
        config.scheduler_poll_interval = Duration::from_secs(15);
        config.daemon_lease_duration = Duration::from_secs(15);
        assert!(config.validate().is_err());
    }

    #[test]
    fn default_event_buffers_stay_within_a_small_bounded_budget() {
        let config = MakoRuntimeConfig::for_database("runtime.db");
        let buffered_slots = config.live_event_capacity
            + config.subscriber_capacity
            + config.execution_event_capacity;
        assert!(buffered_slots * config.max_execution_event_bytes <= 4 * 1024 * 1024);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn cancellation_grace_and_abort_delivery_fit_inside_the_worker_lease() {
        let mut config = MakoRuntimeConfig::for_database("runtime.db");
        config.worker_lease_duration = Duration::from_secs(4);
        config.cancellation_grace_period = Duration::from_secs(2);
        assert!(config.validate().is_err());

        config.cancellation_grace_period = Duration::from_millis(500);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn oversized_event_buffers_and_payload_limits_are_rejected() {
        let mut config = MakoRuntimeConfig::for_database("runtime.db");
        config.live_event_capacity = MAX_LIVE_EVENT_CAPACITY + 1;
        assert!(config.validate().is_err());

        let mut config = MakoRuntimeConfig::for_database("runtime.db");
        config.max_execution_event_bytes = MAX_EXECUTION_EVENT_BYTES + 1;
        assert!(config.validate().is_err());

        let mut config = MakoRuntimeConfig::for_database("runtime.db");
        config.live_event_capacity = MAX_LIVE_EVENT_CAPACITY;
        config.subscriber_capacity = MAX_SUBSCRIBER_CAPACITY;
        config.execution_event_capacity = MAX_EXECUTION_EVENT_CAPACITY;
        config.max_execution_event_bytes = MAX_EXECUTION_EVENT_BYTES;
        assert!(config.validate().is_err());
    }
}
