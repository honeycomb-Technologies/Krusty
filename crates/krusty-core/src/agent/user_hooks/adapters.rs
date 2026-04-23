use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::agent::hooks::{HookResult, PostToolHook, PreToolHook};
use crate::tools::registry::{ToolContext, ToolResult};

use super::executor::UserHookExecutor;
use super::manager::UserHookManager;
use super::model::{UserHookResult, UserHookType};

/// Wrapper that implements `PreToolHook` for user-defined hooks.
pub struct UserPreToolHook {
    manager: Arc<RwLock<UserHookManager>>,
}

impl UserPreToolHook {
    pub fn new(manager: Arc<RwLock<UserHookManager>>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl PreToolHook for UserPreToolHook {
    async fn before_execute(
        &self,
        name: &str,
        params: &serde_json::Value,
        _ctx: &ToolContext,
    ) -> HookResult {
        let mut manager = self.manager.write().await;
        let result = UserHookExecutor::execute_matching(
            &mut manager,
            UserHookType::PreToolUse,
            name,
            params,
        )
        .await;

        match result {
            UserHookResult::Block { reason } => HookResult::Block { reason },
            UserHookResult::Warn { message } => {
                tracing::warn!(tool = name, "User pre-hook warning: {}", message);
                HookResult::Continue
            }
            UserHookResult::Continue => HookResult::Continue,
        }
    }
}

/// Wrapper that implements `PostToolHook` for user-defined hooks.
pub struct UserPostToolHook {
    manager: Arc<RwLock<UserHookManager>>,
}

impl UserPostToolHook {
    pub fn new(manager: Arc<RwLock<UserHookManager>>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl PostToolHook for UserPostToolHook {
    async fn after_execute(
        &self,
        name: &str,
        params: &serde_json::Value,
        _result: &ToolResult,
        _duration: Duration,
    ) -> HookResult {
        let mut manager = self.manager.write().await;
        let _ = UserHookExecutor::execute_matching(
            &mut manager,
            UserHookType::PostToolUse,
            name,
            params,
        )
        .await;

        HookResult::Continue
    }
}
