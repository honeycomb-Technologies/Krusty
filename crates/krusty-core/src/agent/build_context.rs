//! Shared build context for builder swarm coordination
//!
//! LEAN design - only what's actually used:
//! - Conventions (coding style rules)
//! - File locks (prevent concurrent edits)
//! - Line diffs (UI feedback)
//! - Modified files tracking (summary)
//! - Lock contention tracking (observability)
//! - Interface registry (inter-builder communication)

use dashmap::DashMap;
use parking_lot::RwLock;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::Duration;

/// Exported interface from a builder for inter-builder communication
#[derive(Clone, Debug)]
pub struct BuilderInterface {
    /// The builder that registered this interface
    pub builder_id: String,
    /// Path to the file containing the interface
    pub file_path: PathBuf,
    /// Exported function/class/type names
    pub exports: Vec<String>,
    /// Brief description of what this interface provides
    pub description: String,
}

/// Shared context for builder swarm coordination
pub struct SharedBuildContext {
    /// Coding conventions (all builders follow these)
    conventions: RwLock<Vec<String>>,

    /// File locks: path -> agent_id holding the lock
    file_locks: DashMap<PathBuf, String>,

    /// Files modified during this build: path -> agent_id
    modified_files: DashMap<PathBuf, String>,

    /// Line diff tracking for UI
    lines_added: AtomicUsize,
    lines_removed: AtomicUsize,

    /// Stats for debugging
    locks_acquired: AtomicUsize,
    lock_contentions: AtomicUsize,

    /// Track lock wait times per file for contention analysis
    lock_wait_times: DashMap<PathBuf, Vec<Duration>>,

    /// Total time spent waiting for locks (milliseconds)
    total_lock_wait_ms: AtomicU64,

    /// Interfaces registered by builders for inter-builder communication
    interfaces: DashMap<String, BuilderInterface>,
}

impl SharedBuildContext {
    pub fn new() -> Self {
        Self {
            conventions: RwLock::new(Vec::new()),
            file_locks: DashMap::new(),
            modified_files: DashMap::new(),
            lines_added: AtomicUsize::new(0),
            lines_removed: AtomicUsize::new(0),
            locks_acquired: AtomicUsize::new(0),
            lock_contentions: AtomicUsize::new(0),
            lock_wait_times: DashMap::new(),
            total_lock_wait_ms: AtomicU64::new(0),
            interfaces: DashMap::new(),
        }
    }

    // =========================================================================
    // Conventions
    // =========================================================================

    /// Set conventions at start of build
    pub fn set_conventions(&self, conventions: Vec<String>) {
        *self.conventions.write() = conventions;
    }

    /// Get all conventions
    pub fn get_conventions(&self) -> Vec<String> {
        self.conventions.read().clone()
    }

    // =========================================================================
    // File Locks
    // =========================================================================

    /// Try to acquire a lock. Returns Ok(()) or Err(holder_id)
    pub fn acquire_lock(
        &self,
        path: PathBuf,
        agent_id: String,
        _reason: String,
    ) -> Result<(), String> {
        if let Some(holder) = self.file_locks.get(&path) {
            if *holder != agent_id {
                self.lock_contentions.fetch_add(1, Ordering::Relaxed);
                return Err(holder.clone());
            }
            return Ok(()); // Already held by this agent
        }

        self.file_locks.insert(path, agent_id);
        self.locks_acquired.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Release a file lock
    pub fn release_lock(&self, path: &PathBuf, agent_id: &str) {
        if let Some(holder) = self.file_locks.get(path) {
            if *holder == agent_id {
                drop(holder);
                self.file_locks.remove(path);
            }
        }
    }

    /// Release all locks held by an agent (cleanup)
    pub fn release_all_locks(&self, agent_id: &str) {
        // Use retain() for single-pass O(n) operation instead of collect + iterate
        self.file_locks.retain(|_path, owner| {
            // Keep the entry if it's NOT held by the agent being released
            owner != agent_id
        });
    }

    // =========================================================================
    // Lock Contention Tracking
    // =========================================================================

    /// Record a lock wait event for contention analysis
    pub fn record_lock_wait(&self, path: PathBuf, wait_time: Duration) {
        self.lock_wait_times
            .entry(path)
            .or_default()
            .push(wait_time);
        self.total_lock_wait_ms
            .fetch_add(wait_time.as_millis() as u64, Ordering::Relaxed);
    }

    /// Get files with high contention (waited > 1s total)
    pub fn high_contention_files(&self) -> Vec<(PathBuf, Duration)> {
        self.lock_wait_times
            .iter()
            .filter_map(|entry| {
                let total: Duration = entry.value().iter().sum();
                if total > Duration::from_secs(1) {
                    Some((entry.key().clone(), total))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get total lock wait time
    pub fn total_lock_wait(&self) -> Duration {
        Duration::from_millis(self.total_lock_wait_ms.load(Ordering::Relaxed))
    }

    // =========================================================================
    // Interface Registry (inter-builder communication)
    // =========================================================================

    /// Register an interface (builder publishes what it created)
    pub fn register_interface(&self, interface: BuilderInterface) {
        self.interfaces
            .insert(interface.builder_id.clone(), interface);
    }

    /// Get all registered interfaces
    pub fn get_interfaces(&self) -> Vec<BuilderInterface> {
        self.interfaces.iter().map(|e| e.value().clone()).collect()
    }

    /// Get interface by builder name
    pub fn get_interface(&self, builder_id: &str) -> Option<BuilderInterface> {
        self.interfaces.get(builder_id).map(|e| e.value().clone())
    }

    // =========================================================================
    // Modified Files
    // =========================================================================

    /// Record that a file was modified
    pub fn record_modification(&self, path: PathBuf, agent_id: String) {
        self.modified_files.insert(path, agent_id);
    }

    // =========================================================================
    // Line Diffs
    // =========================================================================

    /// Record line changes
    pub fn record_line_changes(&self, added: usize, removed: usize) {
        self.lines_added.fetch_add(added, Ordering::Relaxed);
        self.lines_removed.fetch_add(removed, Ordering::Relaxed);
    }

    /// Get current line diff totals
    pub fn get_line_diff(&self) -> (usize, usize) {
        (
            self.lines_added.load(Ordering::Relaxed),
            self.lines_removed.load(Ordering::Relaxed),
        )
    }

    // =========================================================================
    // Context Injection (for builder prompts)
    // =========================================================================

    /// Generate context to inject into builder prompts
    pub fn generate_context_injection(&self) -> String {
        let mut lines = Vec::new();

        // Conventions
        let conventions = self.get_conventions();
        if !conventions.is_empty() {
            lines.push("[CONVENTIONS]".to_string());
            for conv in conventions {
                lines.push(format!("- {}", conv));
            }
            lines.push(String::new());
        }

        // Current locks (so builders know what's being worked on)
        let locks: Vec<_> = self
            .file_locks
            .iter()
            .map(|r| (r.key().display().to_string(), r.value().clone()))
            .collect();
        if !locks.is_empty() {
            lines.push("[FILES IN PROGRESS]".to_string());
            for (path, agent) in locks {
                lines.push(format!("- {} (by {})", path, agent));
            }
            lines.push(String::new());
        }

        // Registered interfaces from other builders
        let interfaces = self.get_interfaces();
        if !interfaces.is_empty() {
            lines.push("[AVAILABLE INTERFACES]".to_string());
            for iface in interfaces {
                lines.push(format!(
                    "- {} ({}): {}",
                    iface.builder_id,
                    iface.file_path.display(),
                    iface.description
                ));
                if !iface.exports.is_empty() {
                    lines.push(format!("  Exports: {}", iface.exports.join(", ")));
                }
            }
            lines.push(String::new());
        }

        lines.join("\n")
    }

    // =========================================================================
    // Stats
    // =========================================================================

    pub fn stats(&self) -> BuildContextStats {
        BuildContextStats {
            files_modified: self.modified_files.len(),
            lines_added: self.lines_added.load(Ordering::Relaxed),
            lines_removed: self.lines_removed.load(Ordering::Relaxed),
            lock_contentions: self.lock_contentions.load(Ordering::Relaxed),
            high_contention_files: self.high_contention_files(),
            total_lock_wait_ms: self.total_lock_wait_ms.load(Ordering::Relaxed),
        }
    }
}

impl Default for SharedBuildContext {
    fn default() -> Self {
        Self::new()
    }
}

/// Stats for logging/display
#[derive(Debug, Clone)]
pub struct BuildContextStats {
    pub files_modified: usize,
    pub lines_added: usize,
    pub lines_removed: usize,
    pub lock_contentions: usize,
    pub high_contention_files: Vec<(PathBuf, Duration)>,
    pub total_lock_wait_ms: u64,
}

impl std::fmt::Display for BuildContextStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "+{} -{} lines, {} files, {} contentions",
            self.lines_added, self.lines_removed, self.files_modified, self.lock_contentions
        )?;
        if self.total_lock_wait_ms > 0 {
            write!(
                f,
                ", {:.1}s lock wait",
                self.total_lock_wait_ms as f64 / 1000.0
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_acquire_lock_success() {
        let ctx = SharedBuildContext::new();
        let path = PathBuf::from("/test/file.rs");
        let agent_id = "agent-1".to_string();

        let result = ctx.acquire_lock(path.clone(), agent_id.clone(), "testing".to_string());
        assert!(result.is_ok(), "Should acquire lock successfully");

        // Lock should be held
        let holder = ctx.file_locks.get(&path);
        assert!(holder.is_some());
        assert_eq!(*holder.unwrap().value(), agent_id);
    }

    #[test]
    fn test_acquire_lock_already_held_by_same_agent() {
        let ctx = SharedBuildContext::new();
        let path = PathBuf::from("/test/file.rs");
        let agent_id = "agent-1".to_string();

        // First acquisition
        let result1 = ctx.acquire_lock(path.clone(), agent_id.clone(), "first".to_string());
        assert!(result1.is_ok());

        // Second acquisition by same agent should succeed
        let result2 = ctx.acquire_lock(path.clone(), agent_id.clone(), "second".to_string());
        assert!(
            result2.is_ok(),
            "Re-acquisition by same agent should succeed"
        );
    }

    #[test]
    fn test_acquire_lock_contention() {
        let ctx = SharedBuildContext::new();
        let path = PathBuf::from("/test/file.rs");
        let agent1 = "agent-1".to_string();
        let agent2 = "agent-2".to_string();

        // Agent 1 acquires lock
        let result1 = ctx.acquire_lock(path.clone(), agent1.clone(), "first".to_string());
        assert!(result1.is_ok());

        // Agent 2 should fail to acquire
        let result2 = ctx.acquire_lock(path.clone(), agent2.clone(), "contention".to_string());
        assert!(
            result2.is_err(),
            "Agent 2 should fail to acquire lock held by agent 1"
        );

        let err = result2.unwrap_err();
        assert_eq!(err, agent1, "Error should return holder ID");

        // Check contention counter
        assert_eq!(ctx.lock_contentions.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_release_lock() {
        let ctx = SharedBuildContext::new();
        let path = PathBuf::from("/test/file.rs");
        let agent_id = "agent-1".to_string();

        // Acquire lock
        ctx.acquire_lock(path.clone(), agent_id.clone(), "testing".to_string())
            .unwrap();

        // Release lock
        ctx.release_lock(&path, &agent_id);

        // Lock should be gone
        let holder = ctx.file_locks.get(&path);
        assert!(holder.is_none(), "Lock should be released");
    }

    #[test]
    fn test_release_lock_by_non_holder() {
        let ctx = SharedBuildContext::new();
        let path = PathBuf::from("/test/file.rs");
        let agent1 = "agent-1".to_string();
        let agent2 = "agent-2".to_string();

        // Agent 1 acquires lock
        ctx.acquire_lock(path.clone(), agent1.clone(), "first".to_string())
            .unwrap();

        // Agent 2 tries to release (should be ignored)
        ctx.release_lock(&path, &agent2);

        // Lock should still be held by agent 1
        let holder = ctx.file_locks.get(&path);
        assert!(holder.is_some());
        assert_eq!(*holder.unwrap().value(), agent1);
    }

    #[test]
    fn test_release_all_locks_single_agent() {
        let ctx = SharedBuildContext::new();
        let agent1 = "agent-1".to_string();
        let agent2 = "agent-2".to_string();

        let file1 = PathBuf::from("/test/file1.rs");
        let file2 = PathBuf::from("/test/file2.rs");
        let file3 = PathBuf::from("/test/file3.rs");

        // Agent 1 holds 2 locks
        ctx.acquire_lock(file1.clone(), agent1.clone(), "1".to_string())
            .unwrap();
        ctx.acquire_lock(file2.clone(), agent1.clone(), "2".to_string())
            .unwrap();

        // Agent 2 holds 1 lock
        ctx.acquire_lock(file3.clone(), agent2.clone(), "3".to_string())
            .unwrap();

        // Release all agent 1 locks
        ctx.release_all_locks(&agent1);

        // Agent 1's locks should be released
        assert!(ctx.file_locks.get(&file1).is_none());
        assert!(ctx.file_locks.get(&file2).is_none());

        // Agent 2's lock should remain
        assert!(ctx.file_locks.get(&file3).is_some());
        assert_eq!(*ctx.file_locks.get(&file3).unwrap().value(), agent2);
    }

    #[test]
    fn test_release_all_locks_empty() {
        let ctx = SharedBuildContext::new();
        let agent = "agent-1".to_string();

        // Should not panic or error
        ctx.release_all_locks(&agent);

        assert_eq!(ctx.file_locks.len(), 0);
    }

    #[test]
    fn test_lock_stats_tracking() {
        let ctx = SharedBuildContext::new();
        let path = PathBuf::from("/test/file.rs");
        let agent = "agent-1".to_string();

        ctx.acquire_lock(path.clone(), agent.clone(), "test".to_string())
            .unwrap();

        assert_eq!(ctx.locks_acquired.load(Ordering::Relaxed), 1);
        assert_eq!(ctx.lock_contentions.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_modified_files_tracking() {
        let ctx = SharedBuildContext::new();
        let file1 = PathBuf::from("/test/file1.rs");
        let file2 = PathBuf::from("/test/file2.rs");
        let agent1 = "agent-1".to_string();
        let agent2 = "agent-2".to_string();

        ctx.record_modification(file1.clone(), agent1.clone());
        ctx.record_modification(file2.clone(), agent2.clone());

        assert_eq!(ctx.modified_files.len(), 2);

        // Check file1
        let entry = ctx.modified_files.get(&file1).unwrap();
        assert_eq!(*entry.value(), agent1);

        // Check file2
        let entry = ctx.modified_files.get(&file2).unwrap();
        assert_eq!(*entry.value(), agent2);
    }

    #[test]
    fn test_line_diff_tracking() {
        let ctx = SharedBuildContext::new();

        ctx.record_line_changes(10, 5);
        ctx.record_line_changes(3, 7);

        let (added, removed) = ctx.get_line_diff();
        assert_eq!(added, 13);
        assert_eq!(removed, 12);
    }

    #[test]
    fn test_conventions_management() {
        let ctx = SharedBuildContext::new();

        let conventions = vec![
            "Use anyhow for errors".to_string(),
            "Add tracing logs".to_string(),
        ];

        ctx.set_conventions(conventions.clone());
        let retrieved = ctx.get_conventions();

        assert_eq!(retrieved, conventions);
    }

    #[test]
    fn test_interface_registry() {
        let ctx = SharedBuildContext::new();

        let interface = BuilderInterface {
            builder_id: "builder-1".to_string(),
            file_path: PathBuf::from("/test/interface.rs"),
            exports: vec!["MyInterface".to_string(), "helper".to_string()],
            description: "Test interface".to_string(),
        };

        ctx.register_interface(interface.clone());

        // Get all interfaces
        let all = ctx.get_interfaces();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].builder_id, "builder-1");

        // Get by builder ID
        let retrieved = ctx.get_interface("builder-1");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().exports.len(), 2);
    }

    #[test]
    fn test_lock_contention_tracking() {
        let ctx = SharedBuildContext::new();
        let path = PathBuf::from("/test/file.rs");

        // Record some wait times
        ctx.record_lock_wait(path.clone(), Duration::from_millis(100));
        ctx.record_lock_wait(path.clone(), Duration::from_millis(200));
        ctx.record_lock_wait(path.clone(), Duration::from_millis(300));

        // Total wait time
        let total = ctx.total_lock_wait();
        assert_eq!(total, Duration::from_millis(600));

        // High contention files (total > 1s)
        let high = ctx.high_contention_files();
        assert_eq!(
            high.len(),
            0,
            "Total wait 600ms should not be high contention"
        );

        // Add more waits to exceed threshold
        ctx.record_lock_wait(path.clone(), Duration::from_millis(500));
        let high = ctx.high_contention_files();
        assert_eq!(high.len(), 1, "Total wait 1100ms should be high contention");
        assert_eq!(high[0].1, Duration::from_millis(1100));
    }

    #[test]
    fn test_context_injection_generation() {
        let ctx = SharedBuildContext::new();

        // Set conventions
        ctx.set_conventions(vec!["Use anyhow".to_string()]);

        // Acquire a lock
        let path = PathBuf::from("/test/file.rs");
        ctx.acquire_lock(path.clone(), "agent-1".to_string(), "test".to_string())
            .unwrap();

        // Register interface
        ctx.register_interface(BuilderInterface {
            builder_id: "builder-1".to_string(),
            file_path: PathBuf::from("/test/interface.rs"),
            exports: vec!["MyInterface".to_string()],
            description: "Test interface".to_string(),
        });

        let injection = ctx.generate_context_injection();

        assert!(injection.contains("[CONVENTIONS]"));
        assert!(injection.contains("Use anyhow"));
        assert!(injection.contains("[FILES IN PROGRESS]"));
        assert!(injection.contains("/test/file.rs"));
        assert!(injection.contains("[AVAILABLE INTERFACES]"));
    }

    #[test]
    fn test_concurrent_lock_access() {
        use std::sync::Arc;
        use std::thread;

        let ctx = Arc::new(SharedBuildContext::new());
        let path = Arc::new(PathBuf::from("/test/file.rs"));
        let mut handles = vec![];

        // Spawn multiple threads trying to acquire the same lock
        for i in 0..10 {
            let ctx_clone = Arc::clone(&ctx);
            let path_clone = Arc::clone(&path);
            let agent_id = format!("agent-{}", i);

            handles.push(thread::spawn(move || {
                let _ = ctx_clone.acquire_lock(
                    path_clone.as_path().to_path_buf(),
                    agent_id,
                    "test".to_string(),
                );
            }));
        }

        // Wait for all threads
        for handle in handles {
            handle.join().unwrap();
        }

        // Only one lock should exist
        assert_eq!(ctx.file_locks.len(), 1);
    }

    #[test]
    fn test_stats_display() {
        let ctx = SharedBuildContext::new();
        let file = PathBuf::from("/test/file.rs");

        ctx.record_line_changes(100, 50);
        ctx.record_modification(file, "agent-1".to_string());

        let stats = ctx.stats();
        assert_eq!(stats.files_modified, 1);
        assert_eq!(stats.lines_added, 100);
        assert_eq!(stats.lines_removed, 50);

        let display = format!("{}", stats);
        assert!(display.contains("+100"));
        assert!(display.contains("-50"));
        assert!(display.contains("1 files"));
    }
}
