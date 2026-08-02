//! WorktreeDelegate implementation for App
//!
//! Provides the worktree interface that WASM extensions need to interact
//! with the filesystem and environment.

use anyhow::Result;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

use crate::extensions::types::WorktreeDelegate;

/// Implementation of WorktreeDelegate for the TUI app
pub struct AppWorktreeDelegate {
    working_dir: PathBuf,
    id: u64,
}

impl AppWorktreeDelegate {
    pub fn new(working_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            working_dir,
            id: 1, // Single worktree for CLI app
        })
    }
}

#[async_trait]
impl WorktreeDelegate for AppWorktreeDelegate {
    fn id(&self) -> u64 {
        self.id
    }

    fn root_path(&self) -> String {
        self.working_dir.to_string_lossy().into_owned()
    }

    async fn read_text_file(&self, path: &str) -> Result<String> {
        let full_path = if std::path::Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.working_dir.join(path)
        };

        tokio::fs::read_to_string(&full_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", full_path.display(), e))
    }

    fn which(&self, binary_name: &str) -> Option<String> {
        which::which(binary_name)
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }

    fn shell_env(&self) -> Vec<(String, String)> {
        std::env::vars().collect()
    }
}
