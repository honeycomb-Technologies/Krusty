mod runner;
mod task_store;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::RwLock;
use tracing::info;

use crate::agent::subagent::{AgentIdentity, AgentProgress};
use crate::ai::client::AiClient;
use crate::process::ProcessRegistry;
use crate::tools::registry::{PermissionMode, ToolRegistry};

use super::teammate::{Teammate, TeammateConfig, TeammateStatus};

use self::runner::run_teammate_loop;

pub struct TeamManager {
    teammates: Arc<RwLock<HashMap<String, Teammate>>>,
    ai_client: Arc<AiClient>,
    tool_registry: Arc<ToolRegistry>,
    working_dir: PathBuf,
    project_dir: Option<PathBuf>,
    session_id: String,
    db_path: PathBuf,
    process_registry: Option<Arc<ProcessRegistry>>,
    process_owner_id: Option<String>,
    permission_mode: PermissionMode,
    subagent_max_turns: Option<usize>,
    next_ordinal: AtomicUsize,
}

impl TeamManager {
    pub fn new(
        ai_client: Arc<AiClient>,
        tool_registry: Arc<ToolRegistry>,
        working_dir: PathBuf,
        project_dir: Option<PathBuf>,
        session_id: String,
        db_path: PathBuf,
        permission_mode: PermissionMode,
        subagent_max_turns: Option<usize>,
    ) -> Self {
        Self {
            teammates: Arc::new(RwLock::new(HashMap::new())),
            ai_client,
            tool_registry,
            working_dir,
            project_dir,
            session_id,
            db_path,
            process_registry: None,
            process_owner_id: None,
            permission_mode,
            subagent_max_turns,
            next_ordinal: AtomicUsize::new(0),
        }
    }

    /// Attach the originating runtime's process registry and optional owner.
    /// Teammates otherwise keep process context absent rather than creating a
    /// detached or process-local registry that the parent cannot control.
    pub fn with_process_context(
        mut self,
        process_registry: Arc<ProcessRegistry>,
        process_owner_id: Option<String>,
    ) -> Self {
        self.process_registry = Some(process_registry);
        self.process_owner_id = process_owner_id.filter(|owner| !owner.trim().is_empty());
        self
    }

    pub async fn spawn_teammate(&self, config: TeammateConfig) -> Result<String> {
        let mut config = config;
        config.max_turns = restrictive_turn_budget(config.max_turns, self.subagent_max_turns);
        let name = config.name.clone();
        let ordinal = self.next_ordinal.fetch_add(1, Ordering::Relaxed);
        let identity = AgentIdentity::child(
            format!("{}:{}", self.session_id, name),
            "/root/team",
            name.clone(),
            config.role.label(),
            ordinal,
        );
        let mut teammate = Teammate::new(config.clone(), identity.clone());
        {
            let teammates = self.teammates.read().await;
            if teammates.contains_key(&name) {
                anyhow::bail!("teammate '{}' already exists", name);
            }
        }

        let cancel_token = teammate.cancel_token.clone();
        let status = teammate.status.clone();
        let progress = teammate.progress.clone();
        let ai_client = self.ai_client.clone();
        let tool_registry = self.tool_registry.clone();
        let working_dir = self.working_dir.clone();
        let _project_dir = self.project_dir.clone();
        let session_id = self.session_id.clone();
        let db_path = self.db_path.clone();
        let process_registry = self.process_registry.clone();
        let process_owner_id = self.process_owner_id.clone();
        let permission_mode = self.permission_mode;

        let handle = tokio::spawn(async move {
            run_teammate_loop(
                config,
                cancel_token,
                status,
                progress,
                ai_client,
                tool_registry,
                working_dir,
                session_id,
                db_path,
                process_registry,
                process_owner_id,
                permission_mode,
                identity,
            )
            .await;
        });

        teammate.set_handle(handle);

        self.teammates.write().await.insert(name.clone(), teammate);

        info!(teammate = %name, "spawned teammate");
        Ok(name)
    }

    pub async fn cancel_teammate(&self, name: &str) -> Result<()> {
        let teammate = self
            .teammates
            .write()
            .await
            .remove(name)
            .context("teammate not found")?;
        info!(teammate = %name, "cancelling teammate");
        teammate.shutdown().await;
        Ok(())
    }

    pub async fn list_teammates(&self) -> Vec<(String, TeammateStatus, AgentProgress)> {
        let teammates = self.teammates.read().await;
        let mut result = Vec::with_capacity(teammates.len());
        for (name, teammate) in teammates.iter() {
            let status = teammate.status.read().await.clone();
            let progress = teammate.progress.read().await.clone();
            result.push((name.clone(), status, progress));
        }
        result.sort_by(|a, b| a.0.cmp(&b.0));
        result
    }

    pub async fn get_teammate_status(&self, name: &str) -> Option<TeammateStatus> {
        let status = {
            let teammates = self.teammates.read().await;
            teammates
                .get(name)
                .map(|teammate| teammate.status.clone())?
        };
        let current = status.read().await.clone();
        Some(current)
    }

    pub async fn cancel_all(&self) {
        let drained: Vec<(String, Teammate)> = {
            let mut teammates = self.teammates.write().await;
            teammates.drain().collect()
        };

        for (name, teammate) in drained {
            info!(teammate = %name, "cancelling teammate (cancel_all)");
            teammate.shutdown().await;
        }
    }
}

fn restrictive_turn_budget(requested: Option<usize>, inherited: Option<usize>) -> Option<usize> {
    match (requested, inherited) {
        (Some(requested), Some(inherited)) => Some(requested.min(inherited)),
        (requested, inherited) => requested.or(inherited),
    }
}

impl Drop for TeamManager {
    fn drop(&mut self) {
        let teammates = self.teammates.clone();
        tokio::spawn(async move {
            let mut lock = teammates.write().await;
            for (_, teammate) in lock.drain() {
                teammate.cancel();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::restrictive_turn_budget;

    #[test]
    fn teammate_turn_budget_can_tighten_but_never_relax_parent() {
        assert_eq!(restrictive_turn_budget(Some(10), Some(20)), Some(10));
        assert_eq!(restrictive_turn_budget(Some(40), Some(20)), Some(20));
        assert_eq!(restrictive_turn_budget(None, Some(20)), Some(20));
    }
}
