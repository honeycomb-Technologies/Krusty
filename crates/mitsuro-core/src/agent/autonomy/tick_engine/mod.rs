mod conversation;
mod driver;
mod policy;

use std::time::Duration;

use tokio::sync::mpsc;

use crate::agent::loop_events::{LoopEvent, LoopInput};
use crate::agent::orchestrator::OrchestratorServices;
use crate::agent::{RunProvenance, RunSpec, RunSpecError};
use crate::ai::types::ModelMessage;
use crate::storage::ProjectSettings;

/// Maximum number of not-yet-consumed outer loop events retained by Hive.
/// Backpressure propagates into the tick driver once this queue is full.
pub const TICK_EVENT_CHANNEL_CAPACITY: usize = 64;

pub struct TickEngineConfig {
    pub tick_interval: Duration,
    /// Maximum total inner orchestrator runs, including the initial run.
    pub max_ticks: usize,
    pub enabled: bool,
}

impl Default for TickEngineConfig {
    fn default() -> Self {
        let settings = ProjectSettings::default().hive_settings();
        Self {
            tick_interval: Duration::from_secs(settings.tick_interval_secs),
            max_ticks: settings.max_ticks,
            enabled: false,
        }
    }
}

pub struct TickEngine;

impl TickEngine {
    pub fn run(
        services: OrchestratorServices,
        run_spec: RunSpec,
        tick_config: TickEngineConfig,
        initial_conversation: Vec<ModelMessage>,
    ) -> Result<(mpsc::Receiver<LoopEvent>, mpsc::UnboundedSender<LoopInput>), RunSpecError> {
        let (config, options) = run_spec.into_parts_for(RunProvenance::Hive)?;
        tracing::info!(
            surface = RunProvenance::Hive.as_str(),
            session_id = %config.session_id,
            "Starting resolved Hive tick run"
        );
        let (outer_tx, outer_rx) = mpsc::channel(TICK_EVENT_CHANNEL_CAPACITY);
        let (outer_input_tx, outer_input_rx) = mpsc::unbounded_channel();

        tokio::spawn(driver::drive(
            services,
            config,
            tick_config,
            initial_conversation,
            options,
            outer_tx,
            outer_input_rx,
        ));

        Ok((outer_rx, outer_input_tx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_engine_config_defaults() {
        let config = TickEngineConfig::default();
        assert_eq!(config.tick_interval, Duration::from_secs(30));
        assert_eq!(config.max_ticks, 1000);
        assert!(!config.enabled);
        const { assert!(TICK_EVENT_CHANNEL_CAPACITY > 0) };
    }
}
