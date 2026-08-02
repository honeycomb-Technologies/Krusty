mod model;
mod store;

pub use model::{
    MobileDiagnosticCategoryCount, MobileDiagnosticEvent, MobileDiagnosticEventInput,
    MobileDiagnosticNativePayload, MobileDiagnosticNativePayloadInput, MobileDiagnosticReport,
    MobileDiagnosticRun, MobileDiagnosticRunInput,
};
pub use store::MobileDiagnosticStore;

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use crate::storage::Database;

    use super::*;

    #[test]
    fn batches_are_idempotent_bounded_and_reportable() {
        let temp = TempDir::new().expect("temp dir");
        let mut store = MobileDiagnosticStore::new(
            Database::new(&temp.path().join("diagnostics.db")).expect("database"),
        );
        let run = MobileDiagnosticRunInput {
            id: "run-1",
            user_id: None,
            installation_id: "install-1",
            app_version: "0.9.20",
            build_number: "261",
            platform: "ios",
            os_version: "26.6",
            device_class: "iPhone",
            capture_level: "stress",
            started_at_ms: 1,
            ended_at_ms: None,
            completed: false,
            dropped_event_count: 0,
        };
        let event = MobileDiagnosticEventInput {
            sequence: 1,
            occurred_at_ms: 2,
            monotonic_ms: 1.0,
            category: "runtime",
            name: "long_task",
            duration_ms: Some(125.0),
            severity: "warning",
            attributes_json: "{\"mode\":\"code\"}",
        };
        let native = MobileDiagnosticNativePayloadInput {
            payload_id: "metric-1",
            kind: "diagnostic",
            received_at_ms: 3,
            payload_json: "{\"callStackTree\":{\"binaryName\":\"Mitsuro\"}}",
        };

        assert_eq!(
            store
                .ingest_batch(
                    run.clone(),
                    std::slice::from_ref(&event),
                    std::slice::from_ref(&native),
                )
                .unwrap(),
            (1, 1)
        );
        assert_eq!(
            store.ingest_batch(run, &[event], &[native]).unwrap(),
            (0, 0)
        );
        let report = store
            .report_for_user("run-1", None)
            .unwrap()
            .expect("report");
        assert_eq!(report.run.event_count, 1);
        assert_eq!(report.long_task_count, 1);
        assert_eq!(report.max_long_task_ms, Some(125.0));
        assert_eq!(report.native_payload_count, 1);
        assert_eq!(report.recent_events.len(), 1);
        let native_payloads = store
            .native_payloads_for_user("run-1", None)
            .unwrap()
            .expect("native payloads");
        assert_eq!(native_payloads.len(), 1);
        assert_eq!(native_payloads[0].kind, "diagnostic");
    }

    #[test]
    fn run_ids_cannot_cross_owners_or_installations() {
        let temp = TempDir::new().expect("temp dir");
        let mut store = MobileDiagnosticStore::new(
            Database::new(&temp.path().join("diagnostics.db")).expect("database"),
        );
        let base = MobileDiagnosticRunInput {
            id: "run-1",
            user_id: Some("alice"),
            installation_id: "install-1",
            app_version: "0.9.20",
            build_number: "261",
            platform: "ios",
            os_version: "26.6",
            device_class: "iPhone",
            capture_level: "stress",
            started_at_ms: 1,
            ended_at_ms: None,
            completed: false,
            dropped_event_count: 0,
        };
        store.ingest_batch(base.clone(), &[], &[]).unwrap();

        let conflicting = MobileDiagnosticRunInput {
            user_id: Some("bob"),
            ..base
        };
        assert!(store.ingest_batch(conflicting, &[], &[]).is_err());
    }
}
