//! Extension types
//!
//! Minimal types needed for extension host. Most types come directly from WIT bindings.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

/// Wrapper for language server names (uses Arc<str> for efficiency)
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LanguageServerName(pub Arc<str>);

impl From<String> for LanguageServerName {
    fn from(s: String) -> Self {
        Self(Arc::from(s))
    }
}

impl From<&str> for LanguageServerName {
    fn from(s: &str) -> Self {
        Self(Arc::from(s))
    }
}

/// Delegate trait for worktree operations (host provides to extensions)
#[async_trait]
pub trait WorktreeDelegate: Send + Sync {
    fn id(&self) -> u64;
    fn root_path(&self) -> String;
    async fn read_text_file(&self, path: &str) -> Result<String>;
    fn which(&self, binary_name: &str) -> Option<String>;
    fn shell_env(&self) -> Vec<(String, String)>;
}
