//! Shared cache for sub-agent exploration
//!
//! Caches file reads, glob results, and grep results across agents
//! to avoid redundant disk I/O and tool calls. Read-only, cleared
//! automatically when the explore run finishes (Arc drops).

use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Cached file content
#[derive(Clone, Debug)]
pub struct CachedFile {
    pub content: String,
}

/// Shared cache for explore runs
///
/// All agents in a pool share one cache instance via Arc.
/// Cache is automatically cleared when the explore tool finishes
/// (when all Arc references are dropped).
pub struct SharedExploreCache {
    /// File content cache: absolute path -> content
    files: DashMap<PathBuf, CachedFile>,

    /// Glob results cache: (pattern, base_dir) -> matching paths
    globs: DashMap<(String, PathBuf), Vec<PathBuf>>,

    /// Stats for logging
    file_hits: AtomicUsize,
    file_misses: AtomicUsize,
    glob_hits: AtomicUsize,
    glob_misses: AtomicUsize,
}

impl SharedExploreCache {
    pub fn new() -> Self {
        Self {
            files: DashMap::new(),
            globs: DashMap::new(),
            file_hits: AtomicUsize::new(0),
            file_misses: AtomicUsize::new(0),
            glob_hits: AtomicUsize::new(0),
            glob_misses: AtomicUsize::new(0),
        }
    }

    // =========================================================================
    // File Cache
    // =========================================================================

    /// Get cached file content, if present
    pub fn get_file(&self, path: &PathBuf) -> Option<CachedFile> {
        if let Some(cached) = self.files.get(path) {
            self.file_hits.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(path = %path.display(), "Cache HIT");
            Some(cached.clone())
        } else {
            tracing::debug!(path = %path.display(), "Cache MISS");
            None
        }
    }

    /// Store file content in cache
    pub fn put_file(&self, path: PathBuf, content: String) {
        self.file_misses.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(path = %path.display(), size = content.len(), "Cache PUT");
        self.files.insert(path, CachedFile { content });
    }

    // =========================================================================
    // Glob Cache
    // =========================================================================

    /// Get cached glob results, if present
    pub fn get_glob(&self, pattern: &str, base_dir: &Path) -> Option<Vec<PathBuf>> {
        let key = (pattern.to_string(), base_dir.to_path_buf());
        if let Some(cached) = self.globs.get(&key) {
            self.glob_hits.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(pattern, base_dir = %base_dir.display(), count = cached.len(), "Glob cache HIT");
            Some(cached.clone())
        } else {
            tracing::debug!(pattern, base_dir = %base_dir.display(), "Glob cache MISS");
            None
        }
    }

    /// Store glob results in cache
    pub fn put_glob(&self, pattern: String, base_dir: PathBuf, results: Vec<PathBuf>) {
        self.glob_misses.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(pattern, base_dir = %base_dir.display(), count = results.len(), "Glob cache PUT");
        self.globs.insert((pattern, base_dir), results);
    }

    // =========================================================================
    // Stats
    // =========================================================================

    /// Get cache statistics for logging
    pub fn stats(&self) -> CacheStats {
        CacheStats {
            file_hits: self.file_hits.load(Ordering::Relaxed),
            file_misses: self.file_misses.load(Ordering::Relaxed),
            glob_hits: self.glob_hits.load(Ordering::Relaxed),
            glob_misses: self.glob_misses.load(Ordering::Relaxed),
        }
    }
}

impl Default for SharedExploreCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Cache statistics for logging
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub file_hits: usize,
    pub file_misses: usize,
    pub glob_hits: usize,
    pub glob_misses: usize,
}

impl CacheStats {
    pub fn total_hits(&self) -> usize {
        self.file_hits + self.glob_hits
    }

    pub fn total_misses(&self) -> usize {
        self.file_misses + self.glob_misses
    }

    pub fn hit_rate(&self) -> f64 {
        let total = self.total_hits() + self.total_misses();
        if total == 0 {
            0.0
        } else {
            self.total_hits() as f64 / total as f64 * 100.0
        }
    }
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cache: {:.1}% hit rate | files: {}/{} hits/misses | globs: {}/{}",
            self.hit_rate(),
            self.file_hits,
            self.file_misses,
            self.glob_hits,
            self.glob_misses,
        )
    }
}
