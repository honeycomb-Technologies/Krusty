//! Immutable deterministic-ID inputs from the first durable Hive protocol.
//!
//! These bytes are persisted idempotency identity, not product copy. Changing
//! them would duplicate controllers, runs, messages, and approvals on restart.

pub(crate) fn controller_id(session_id: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("krusty:mako:controller:{session_id}").as_bytes(),
    )
    .to_string()
}

pub(crate) fn tool_approval_id(controller_id: &str, run_id: &str, tool_call_id: &str) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!("krusty:mako:tool-approval:{controller_id}:{run_id}:{tool_call_id}").as_bytes(),
    )
    .to_string()
}

pub(crate) fn pending_message_id(
    user_id: &str,
    client_kind: &str,
    session_id: &str,
    idempotency_key: &str,
) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_URL,
        format!(
            "krusty:mako:pending-message:{user_id}:{client_kind}:{session_id}:{idempotency_key}"
        )
        .as_bytes(),
    )
    .to_string()
}

pub(crate) fn schedule_object_id(kind: &str, schedule_id: &str, timestamp_micros: i64) -> String {
    uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        format!("mako:{kind}:{schedule_id}:{timestamp_micros}").as_bytes(),
    )
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_deterministic_ids_are_stable() {
        assert_eq!(
            controller_id("session-1"),
            "18384afa-8e54-51d0-b900-5d77cea2d240"
        );
        assert_eq!(
            tool_approval_id("controller-1", "run-1", "tool-1"),
            "c469325f-04f5-5b74-b031-60ebcaa1c536"
        );
        assert_eq!(
            pending_message_id("user-1", "mobile", "session-1", "operation-1"),
            "a0151af1-581d-59ac-8bc5-f61fde1df5f7"
        );
        assert_eq!(
            schedule_object_id("run", "schedule-1", 1_700_000_000_000_000),
            "11203015-26af-5b9b-89cf-4c631d6c1c6a"
        );
    }
}
