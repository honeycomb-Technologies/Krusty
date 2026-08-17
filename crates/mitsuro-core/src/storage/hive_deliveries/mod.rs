//! Durable Worker-to-Worker delivery ledger.
//!
//! `hive_deliveries` generalizes the proven `hive_control_outbox` pattern
//! (dedupe key, status, attempts, `available_at`) into one row per
//! message-per-recipient. The daemon pump claims due rows each fenced tick
//! and delivers by enqueueing a run on the recipient Worker's DM lane or by
//! steering its active run; crash replay is idempotent redelivery. This is
//! the structural fix for volatile in-memory wake queues: no peer message
//! survives only in process memory.

mod model;
mod store;

#[cfg(test)]
mod tests;

pub use model::{
    HiveDelivery, HiveDeliveryEnqueue, HiveDeliveryKind, HiveDeliveryPriority, HiveDeliveryStatus,
    NewHiveDelivery, DEFAULT_HIVE_DELIVERY_MAX_ATTEMPTS, MAX_HIVE_DELIVERY_BODY_BYTES,
};
pub use store::{
    ack_for_terminal_runs_with_conn, claim_due_with_conn, enqueue_with_conn,
    fail_attempt_with_conn, hive_delivery_retry_backoff, load_delivery, mark_delivered_with_conn,
    revert_wait_with_conn, HiveDeliveryStore,
};
