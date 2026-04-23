use tempfile::TempDir;

use super::Preferences;
use crate::ai::models::{
    dynamic_model_cache_ttl, model_catalog_fingerprint, DynamicModelCacheMetadata, ModelMetadata,
};
use crate::ai::providers::ProviderId;
use crate::storage::{database::Database, unix_timestamp};

fn create_preferences() -> (Preferences, TempDir) {
    let temp_dir = TempDir::new().expect("temp dir");
    let db_path = temp_dir.path().join("prefs.db");
    let db = Database::new(&db_path).expect("database");
    (Preferences::new(db), temp_dir)
}

#[test]
fn caches_dynamic_model_metadata_with_provider_ttl() {
    let (prefs, _temp) = create_preferences();
    let models = vec![ModelMetadata::new(
        "gpt-5.3-codex",
        "GPT-5.3 Codex",
        ProviderId::OpenAI,
    )];

    prefs.cache_models(ProviderId::OpenAI, &models).unwrap();

    let metadata = prefs.get_model_cache_metadata(ProviderId::OpenAI).unwrap();
    assert_eq!(
        metadata.ttl_seconds,
        dynamic_model_cache_ttl(ProviderId::OpenAI)
    );
    assert_eq!(metadata.model_count, 1);
    assert_eq!(metadata.fingerprint, model_catalog_fingerprint(&models));
}

#[test]
fn invalid_cache_metadata_marks_model_cache_stale() {
    let (prefs, _temp) = create_preferences();
    let models = vec![ModelMetadata::new(
        "gpt-5.3-codex",
        "GPT-5.3 Codex",
        ProviderId::OpenAI,
    )];

    prefs.cache_models(ProviderId::OpenAI, &models).unwrap();

    let broken = DynamicModelCacheMetadata {
        fetched_at: unix_timestamp(),
        ttl_seconds: dynamic_model_cache_ttl(ProviderId::OpenAI),
        model_count: 99,
        fingerprint: 0,
    };
    prefs
        .set(
            "openai_models_cache_meta",
            &serde_json::to_string(&broken).unwrap(),
        )
        .unwrap();

    assert!(prefs.is_model_cache_stale(ProviderId::OpenAI));
}
