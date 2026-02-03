//! Markdown Cache
//!
//! Caches rendered markdown lines to avoid re-rendering on every frame.

use std::collections::HashMap;
use std::sync::Arc;

use ratatui::text::Line;

use super::links::RenderedMarkdown;
use crate::tui::themes::Theme;

/// Cache key: (content_hash, wrap_width)
type CacheKey = (u64, usize);

/// Cached markdown with link tracking
pub struct MarkdownCache {
    /// The cache: (message_content_hash, width) -> rendered markdown with links
    cache: HashMap<CacheKey, Arc<RenderedMarkdown>>,
    /// Legacy cache for backward compatibility (no link tracking)
    legacy_cache: HashMap<CacheKey, Arc<Vec<Line<'static>>>>,
    /// Last render width to track changes
    last_width: usize,
    /// Max cache entries to prevent unbounded growth
    max_entries: usize,
}

impl Default for MarkdownCache {
    fn default() -> Self {
        Self::new()
    }
}

impl MarkdownCache {
    /// Create a new empty cache
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            legacy_cache: HashMap::new(),
            last_width: 0,
            max_entries: 1000, // Reasonable limit for typical usage
        }
    }

    /// Evict entries if cache exceeds max_entries
    fn evict_if_full(&mut self) {
        if self.cache.len() >= self.max_entries {
            let remove_count = self.max_entries / 5;
            let keys_to_remove: Vec<_> = self.cache.keys().take(remove_count).cloned().collect();
            for k in keys_to_remove {
                self.cache.remove(&k);
            }
        }
        if self.legacy_cache.len() >= self.max_entries {
            let remove_count = self.max_entries / 5;
            let keys_to_remove: Vec<_> = self
                .legacy_cache
                .keys()
                .take(remove_count)
                .cloned()
                .collect();
            for k in keys_to_remove {
                self.legacy_cache.remove(&k);
            }
        }
    }

    /// Check if width changed and update tracking
    /// NOTE: No longer clears cache since cache key includes width.
    /// Entries at old widths will naturally age out via cache size limits.
    pub fn check_width(&mut self, width: usize) -> bool {
        let changed = self.last_width != width;
        self.last_width = width;
        changed
    }

    /// Get cached lines for content hash (legacy, no link tracking)
    pub fn get(&self, content_hash: u64, width: usize) -> Option<Arc<Vec<Line<'static>>>> {
        self.legacy_cache.get(&(content_hash, width)).cloned()
    }

    /// Get or render markdown with link tracking, caching the result
    pub fn get_or_render_with_links(
        &mut self,
        content: &str,
        content_hash: u64,
        width: usize,
        theme: &Theme,
    ) -> Arc<RenderedMarkdown> {
        let key = (content_hash, width);

        if let Some(cached) = self.cache.get(&key) {
            Arc::clone(cached)
        } else {
            self.evict_if_full();
            let rendered = super::render_with_links(content, width, theme);
            let arc = Arc::new(rendered);
            self.cache.insert(key, Arc::clone(&arc));
            arc
        }
    }

    /// Get cached rendered markdown (from the links cache)
    pub fn get_rendered(&self, content_hash: u64, width: usize) -> Option<Arc<RenderedMarkdown>> {
        self.cache.get(&(content_hash, width)).cloned()
    }
}
