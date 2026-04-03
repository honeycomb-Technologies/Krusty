use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::RwLock;
use tokio::time::sleep;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::subagent::{
    execute_single_agent, AgentProgress, AgentProgressStatus, SingleExplorerConfig, SubAgentTask,
};
use crate::ai::client::AiClient;
use crate::tools::registry::ToolRegistry;

use super::teammate::{Teammate, TeammateConfig, TeammateStatus};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct TeamManager {
    teammates: Arc<RwLock<HashMap<String, Teammate>>>,
    ai_client: Arc<AiClient>,
    tool_registry: Arc<ToolRegistry>,
    working_dir: PathBuf,
    project_dir: Option<PathBuf>,
    session_id: String,
    db_path: PathBuf,
}

impl TeamManager {
    pub fn new(
        ai_client: Arc<AiClient>,
        tool_registry: Arc<ToolRegistry>,
        working_dir: PathBuf,
        project_dir: Option<PathBuf>,
        session_id: String,
        db_path: PathBuf,
    ) -> Self {
        Self {
            teammates: Arc::new(RwLock::new(HashMap::new())),
            ai_client,
            tool_registry,
            working_dir,
            project_dir,
            session_id,
            db_path,
        }
    }

    pub async fn spawn_teammate(&self, config: TeammateConfig) -> Result<String> {
        let name = config.name.clone();
        let mut teammate = Teammate::new(config.clone());
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

        let handle = tokio::spawn(async move {
            teammate_loop(
                config,
                cancel_token,
                status,
                progress,
                ai_client,
                tool_registry,
                working_dir,
                session_id,
                db_path,
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

/// Background loop for a single teammate.
///
/// Polls the autonomous_tasks table for unclaimed tasks, executes them via the
/// subagent execution loop, and records results. Exits after `IDLE_TIMEOUT` of
/// continuous idleness.
async fn teammate_loop(
    config: TeammateConfig,
    cancel_token: CancellationToken,
    status: Arc<RwLock<TeammateStatus>>,
    progress: Arc<RwLock<AgentProgress>>,
    ai_client: Arc<AiClient>,
    tool_registry: Arc<ToolRegistry>,
    working_dir: PathBuf,
    session_id: String,
    db_path: PathBuf,
) {
    let policy = config.role.delegation_policy(config.max_turns);
    let model = ai_client.config().model.clone();

    info!(
        teammate = %config.name,
        role = %config.role,
        model = %model,
        "teammate loop started"
    );

    let mut consecutive_idle_polls: u64 = 0;
    let max_idle_polls = IDLE_TIMEOUT.as_secs() / POLL_INTERVAL.as_secs();

    loop {
        if cancel_token.is_cancelled() {
            *status.write().await = TeammateStatus::Cancelled;
            info!(teammate = %config.name, "teammate cancelled");
            return;
        }

        let task = match poll_next_task(&db_path, &session_id, &config.name).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                consecutive_idle_polls += 1;
                if consecutive_idle_polls >= max_idle_polls {
                    info!(
                        teammate = %config.name,
                        idle_secs = consecutive_idle_polls * POLL_INTERVAL.as_secs(),
                        "teammate exiting after idle timeout"
                    );
                    *status.write().await = TeammateStatus::Idle;
                    return;
                }

                *status.write().await = TeammateStatus::Idle;
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        *status.write().await = TeammateStatus::Cancelled;
                        info!(teammate = %config.name, "teammate cancelled while idle");
                        return;
                    }
                    _ = sleep(POLL_INTERVAL) => continue,
                }
            }
            Err(e) => {
                warn!(
                    teammate = %config.name,
                    error = %e,
                    "failed to poll for tasks"
                );
                sleep(POLL_INTERVAL).await;
                continue;
            }
        };

        consecutive_idle_polls = 0;
        let task_id = task.task_id.clone();
        let task_description = task.description.clone();

        info!(
            teammate = %config.name,
            task_id = %task_id,
            "claimed task"
        );

        *status.write().await = TeammateStatus::Working {
            task_id: task_id.clone(),
        };

        {
            let mut p = progress.write().await;
            *p = AgentProgress {
                task_id: task_id.clone(),
                name: config.name.clone(),
                status: AgentProgressStatus::Running,
                ..Default::default()
            };
        }

        let subagent_task = SubAgentTask::new(&task_id, &task_description)
            .with_name(&config.name)
            .with_working_dir(working_dir.clone())
            .with_delegation_policy(policy.clone());

        let agent_config =
            SingleExplorerConfig::new(tool_registry.clone(), policy.clone(), String::new()).await;

        let result = execute_single_agent(
            &ai_client,
            subagent_task,
            agent_config,
            &model,
            cancel_token.clone(),
            None,
        )
        .await;

        if cancel_token.is_cancelled() {
            *status.write().await = TeammateStatus::Cancelled;
            return;
        }

        if result.success {
            info!(
                teammate = %config.name,
                task_id = %task_id,
                turns = result.turns_used,
                duration_ms = result.duration_ms,
                "task completed"
            );
            let summary = truncate(&result.output, 2000);
            record_task_complete(&db_path, &task_id, &summary).await;

            {
                let mut p = progress.write().await;
                p.status = AgentProgressStatus::Complete;
                p.current_action = None;
                p.completion_summary = Some(summary.clone());
            }

            *status.write().await = TeammateStatus::Completed {
                task_id: task_id.clone(),
                result: summary,
            };
        } else {
            let error_msg = result
                .error
                .clone()
                .unwrap_or_else(|| "unknown error".to_string());

            warn!(
                teammate = %config.name,
                task_id = %task_id,
                error = %error_msg,
                "task failed"
            );
            record_task_failed(&db_path, &task_id, &error_msg).await;

            {
                let mut p = progress.write().await;
                p.status = AgentProgressStatus::Failed;
            }

            *status.write().await = TeammateStatus::Failed {
                task_id: task_id.clone(),
                error: error_msg,
            };
        }
    }
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let mut boundary = max_chars.min(text.len());
    while boundary > 0 && !text.is_char_boundary(boundary) {
        boundary -= 1;
    }
    text[..boundary].to_string()
}

// ---------------------------------------------------------------------------
// Task store shim
//
// The canonical AutonomousTaskStore is being created in parallel. These helpers
// encapsulate the SQLite interactions via `spawn_blocking` (rusqlite is sync)
// so the teammate loop compiles and runs today. Swap to the real store once it
// lands.
// ---------------------------------------------------------------------------

struct PendingTask {
    task_id: String,
    description: String,
}

const CREATE_TABLE_SQL: &str = "\
    CREATE TABLE IF NOT EXISTS autonomous_tasks (\
        id TEXT PRIMARY KEY,\
        session_id TEXT NOT NULL,\
        description TEXT NOT NULL,\
        status TEXT NOT NULL DEFAULT 'pending',\
        assigned_to TEXT,\
        result TEXT,\
        error TEXT,\
        created_at INTEGER NOT NULL DEFAULT (unixepoch()),\
        updated_at INTEGER NOT NULL DEFAULT (unixepoch())\
    )";

async fn poll_next_task(
    db_path: &Path,
    session_id: &str,
    teammate_name: &str,
) -> Result<Option<PendingTask>> {
    let db_path = db_path.to_path_buf();
    let session_id = session_id.to_string();
    let teammate_name = teammate_name.to_string();

    tokio::task::spawn_blocking(move || {
        let db = crate::storage::Database::new(&db_path).context("open task store db")?;
        db.conn()
            .execute(CREATE_TABLE_SQL, [])
            .context("ensure autonomous_tasks table")?;

        let mut stmt = db
            .conn()
            .prepare(
                "UPDATE autonomous_tasks
                 SET status = 'running', assigned_to = ?1, updated_at = unixepoch()
                 WHERE id = (
                     SELECT id FROM autonomous_tasks
                     WHERE session_id = ?2 AND status = 'pending'
                     ORDER BY created_at ASC
                     LIMIT 1
                 )
                 RETURNING id, description",
            )
            .context("prepare claim query")?;

        let mut rows = stmt
            .query(rusqlite::params![teammate_name, session_id])
            .context("execute claim query")?;

        match rows.next().context("read claimed row")? {
            Some(row) => {
                let task_id: String = row.get(0).context("task id column")?;
                let description: String = row.get(1).context("description column")?;
                debug!(
                    teammate = %teammate_name,
                    task_id = %task_id,
                    "claimed task from store"
                );
                Ok(Some(PendingTask {
                    task_id,
                    description,
                }))
            }
            None => Ok(None),
        }
    })
    .await
    .context("spawn_blocking panicked")?
}

async fn record_task_complete(db_path: &Path, task_id: &str, result: &str) {
    if let Err(e) = record_task_complete_inner(db_path, task_id, result).await {
        warn!(task_id = %task_id, error = %e, "failed to mark task complete");
    }
}

async fn record_task_complete_inner(db_path: &Path, task_id: &str, result: &str) -> Result<()> {
    let db_path = db_path.to_path_buf();
    let task_id = task_id.to_string();
    let result = result.to_string();

    tokio::task::spawn_blocking(move || {
        let db = crate::storage::Database::new(&db_path).context("open task store db")?;
        db.conn()
            .execute(
                "UPDATE autonomous_tasks SET status = 'completed', result = ?1, updated_at = unixepoch() WHERE id = ?2",
                rusqlite::params![result, task_id],
            )
            .context("mark task completed")?;
        Ok(())
    })
    .await
    .context("spawn_blocking panicked")?
}

async fn record_task_failed(db_path: &Path, task_id: &str, error: &str) {
    if let Err(e) = record_task_failed_inner(db_path, task_id, error).await {
        warn!(task_id = %task_id, error = %e, "failed to mark task failed");
    }
}

async fn record_task_failed_inner(db_path: &Path, task_id: &str, error: &str) -> Result<()> {
    let db_path = db_path.to_path_buf();
    let task_id = task_id.to_string();
    let error = error.to_string();

    tokio::task::spawn_blocking(move || {
        let db = crate::storage::Database::new(&db_path).context("open task store db")?;
        db.conn()
            .execute(
                "UPDATE autonomous_tasks SET status = 'failed', error = ?1, updated_at = unixepoch() WHERE id = ?2",
                rusqlite::params![error, task_id],
            )
            .context("mark task failed")?;
        Ok(())
    })
    .await
    .context("spawn_blocking panicked")?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        let ascii = "hello world";
        assert_eq!(truncate(ascii, 5), "hello");

        let multibyte = "cafe\u{0301}";
        let result = truncate(multibyte, 5);
        assert!(result.len() <= 5);
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn truncate_returns_full_string_when_short() {
        assert_eq!(truncate("hi", 100), "hi");
    }
}
