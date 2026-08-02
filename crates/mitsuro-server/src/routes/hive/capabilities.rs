use axum::Json;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub(super) struct HiveCapabilitiesResponse {
    product: &'static str,
    autonomous_system: &'static str,
    api_version: u32,
    read_only: bool,
}

/// Side-effect-free transport probe. Unlike `/main`, this never creates a
/// singleton session, opens a project, or mutates durable state.
pub(super) async fn capabilities() -> Json<HiveCapabilitiesResponse> {
    Json(HiveCapabilitiesResponse {
        product: "mitsuro",
        autonomous_system: "hive",
        api_version: 1,
        read_only: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn capability_probe_is_static_and_read_only() {
        let Json(response) = capabilities().await;
        assert_eq!(response.product, "mitsuro");
        assert_eq!(response.autonomous_system, "hive");
        assert_eq!(response.api_version, 1);
        assert!(response.read_only);
    }
}
