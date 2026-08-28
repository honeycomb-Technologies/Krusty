//! Private conversation lanes used when one Hive Worker participates in a group.
//!
//! A lane belongs to exactly one `(group, worker)` pair and points at a
//! dedicated Hive session. The first persisted binding wins: concurrent
//! callers adopt the same canonical session instead of rebinding the pair and
//! mixing private DM history into a group run.

mod model;
mod store;

#[cfg(test)]
mod tests;

pub use model::{HiveGroupWorkerLane, NewHiveGroupWorkerLane};
pub use store::{
    load_group_worker_lane_with_conn, upsert_group_worker_lane_with_conn, HiveGroupWorkerLaneStore,
};
