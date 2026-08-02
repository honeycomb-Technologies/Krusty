//! Markdown Cache
//!
//! Caches rendered markdown lines to avoid re-rendering on every frame.
//! Bounded to a maximum number of entries with oldest-first eviction.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use super::links::RenderedMarkdown;
use crate::tui::themes::Theme;

/// Cache key: (content_hash, wrap_width)
type CacheKey = (u64, usize);

/// Maximum number of entries per cache map
const MAX_CACHE_ENTRIES: usize = 500;

/// Cached markdown with link tracking
pub struct MarkdownCache {
    /// The cache: (message_content_hash, width) -> rendered markdown with links
    cache: HashMap<CacheKey, Arc<RenderedMarkdown>>,
    /// Insertion order for the main cache (oldest at front)
    cache_order: VecDeque<CacheKey>,
    /// Last render width to track changes
    last_width: usize,
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
            cache_order: VecDeque::new(),
            last_width: 0,
        }
    }

    /// Evict oldest entries from the main cache if it exceeds the limit
    fn evict_cache_if_full(&mut self) {
        while self.cache.len() >= MAX_CACHE_ENTRIES {
            if let Some(oldest) = self.cache_order.pop_front() {
                self.cache.remove(&oldest);
            } else {
                break;
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
            self.evict_cache_if_full();
            let rendered = super::render_with_links(content, width, theme);
            let arc = Arc::new(rendered);
            self.cache.insert(key, Arc::clone(&arc));
            self.cache_order.push_back(key);
            arc
        }
    }

    /// Get cached rendered markdown (from the links cache)
    pub fn get_rendered(&self, content_hash: u64, width: usize) -> Option<Arc<RenderedMarkdown>> {
        self.cache.get(&(content_hash, width)).cloned()
    }
}
