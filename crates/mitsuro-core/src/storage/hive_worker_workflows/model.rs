use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::WorkerGoalAttemptOutcome;

/// Immutable trusted result for one bounded Worker Workflow attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerGoalOutcomeRecord {
    pub run_id: String,
    pub worker_id: String,
    pub owner_user_id: Option<String>,
    pub session_id: String,
    pub workflow_goal_id: String,
    pub workflow_attempt_id: String,
    pub plan_revision_id: String,
    pub step_id: String,
    pub workspace_dir: String,
    pub provider_call_ids: Vec<String>,
    pub outcome: WorkerGoalAttemptOutcome,
    pub evidence: Value,
    pub effect: Value,
    pub counters: Value,
    pub no_progress_fingerprint: Option<String>,
    pub no_progress_streak: u32,
    pub committed_at: String,
}
