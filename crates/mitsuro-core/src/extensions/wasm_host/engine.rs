use moka::sync::Cache;
use std::{
    borrow::Cow,
    sync::{Arc, LazyLock, OnceLock},
};
use wasmtime::CacheStore;

pub fn wasm_engine() -> wasmtime::Engine {
    static WASM_ENGINE: OnceLock<wasmtime::Engine> = OnceLock::new();
    WASM_ENGINE
        .get_or_init(|| {
            let mut config = wasmtime::Config::new();
            config.wasm_component_model(true);
            config.async_support(true);
            config
                .enable_incremental_compilation(cache_store())
                .expect("Failed to enable incremental compilation");
            config.epoch_interruption(true);
            wasmtime::Engine::new(&config).expect("Failed to create WASM engine")
        })
        .clone()
}

fn cache_store() -> Arc<IncrementalCompilationCache> {
    static CACHE_STORE: LazyLock<Arc<IncrementalCompilationCache>> =
        LazyLock::new(|| Arc::new(IncrementalCompilationCache::new()));
    CACHE_STORE.clone()
}

/// Cache for incremental compilation (matches Zed's implementation)
struct IncrementalCompilationCache {
    cache: Cache<Vec<u8>, Vec<u8>>,
}

impl std::fmt::Debug for IncrementalCompilationCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IncrementalCompilationCache").finish()
    }
}

impl IncrementalCompilationCache {
    fn new() -> Self {
        Self {
            cache: Cache::builder().max_capacity(64 * 1024 * 1024).build(),
        }
    }
}

impl CacheStore for IncrementalCompilationCache {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.cache.get(&key.to_vec()).map(Cow::Owned)
    }

    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool {
        self.cache.insert(key.to_vec(), value);
        true
    }
}
