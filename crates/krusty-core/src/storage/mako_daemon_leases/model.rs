use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonLease {
    pub lease_name: String,
    pub owner_id: String,
    pub fencing_token: u64,
    pub acquired_at: String,
    pub heartbeat_at: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DaemonLeaseAcquire {
    Acquired(DaemonLease),
    HeldByOther {
        owner_id: String,
        expires_at: String,
    },
}
