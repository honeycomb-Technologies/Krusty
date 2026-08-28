use serde::{Deserialize, Serialize};

/// The canonical private session for one Worker inside one group room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiveGroupWorkerLane {
    pub group_id: String,
    pub worker_id: String,
    pub session_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Candidate binding for a group Worker lane.
///
/// If the pair is already bound, storage returns the existing canonical lane
/// and never replaces its `session_id` with this candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewHiveGroupWorkerLane {
    pub group_id: String,
    pub worker_id: String,
    pub session_id: String,
}

impl NewHiveGroupWorkerLane {
    pub fn new(
        group_id: impl Into<String>,
        worker_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            group_id: group_id.into(),
            worker_id: worker_id.into(),
            session_id: session_id.into(),
        }
    }
}
