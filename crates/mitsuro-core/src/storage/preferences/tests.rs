use tempfile::TempDir;

use super::Preferences;
use crate::ai::models::{
    dynamic_model_cache_ttl, model_catalog_fingerprint, ApiFormat, DynamicModelCacheMetadata,
    ModelAuthScope, ModelKey, ModelMetadata,
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

#[test]
fn clearing_model_cache_removes_snapshot_and_metadata() {
    let (prefs, _temp) = create_preferences();
    let models = vec![ModelMetadata::new(
        "gpt-5.6-sol",
        "GPT-5.6 Sol",
        ProviderId::OpenAI,
    )];
    prefs.cache_models(ProviderId::OpenAI, &models).unwrap();

    prefs.clear_model_cache(ProviderId::OpenAI).unwrap();

    assert!(prefs.get_cached_models(ProviderId::OpenAI).is_none());
    assert!(prefs.get_model_cache_metadata(ProviderId::OpenAI).is_none());
    assert!(prefs.is_model_cache_stale(ProviderId::OpenAI));
}

#[test]
fn provider_aware_model_preferences_dual_write_legacy_ids() {
    let (prefs, _temp) = create_preferences();
    let key = ModelKey::new(ProviderId::OpenAI, "gpt-shared", ApiFormat::OpenAIResponses)
        .with_auth_scope(ModelAuthScope::OAuth);

    prefs.set_current_model_key(&key).unwrap();
    prefs.add_recent_model_key(&key).unwrap();

    assert_eq!(prefs.get_current_model_key(), Some(key.clone()));
    assert_eq!(prefs.get_current_model().as_deref(), Some("gpt-shared"));
    assert_eq!(prefs.get_recent_model_keys(), vec![key]);
    assert_eq!(prefs.get_recent_models(), vec!["gpt-shared"]);
}

#[test]
fn legacy_model_write_clears_stale_exact_identity() {
    let (prefs, _temp) = create_preferences();
    let key = ModelKey::new(ProviderId::OpenAI, "gpt-shared", ApiFormat::OpenAIResponses);
    prefs.set_current_model_key(&key).unwrap();

    prefs.set_current_model("legacy-model").unwrap();

    assert!(prefs.get_current_model_key().is_none());
    assert_eq!(prefs.get_current_model().as_deref(), Some("legacy-model"));
}
