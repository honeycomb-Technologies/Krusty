use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use crate::ai::types::Content;

/// Terminal completion of a background child, used to wake the parent session.
#[derive(Debug, Clone)]
pub struct ChildCompletionEvent {
    pub session_id: Option<String>,
    pub user_id: Option<String>,
    pub workspace_root: Option<PathBuf>,
    pub pending_id: String,
    pub content: Vec<Content>,
    pub delegated_run_id: String,
    pub task_name: String,
    pub success: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeStatus {
    Running,
    Complete,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentRuntimeSnapshot {
    pub delegated_run_id: String,
    pub task_name: String,
    pub status: AgentRuntimeStatus,
}

#[derive(Debug, Clone)]
pub struct AgentMailbox {
    receiver: Arc<Mutex<broadcast::Receiver<String>>>,
}

impl AgentMailbox {
    fn new(receiver: broadcast::Receiver<String>) -> Self {
        Self {
            receiver: Arc::new(Mutex::new(receiver)),
        }
    }

    pub fn drain(&self) -> Vec<String> {
        let mut receiver = self.receiver.lock().expect("agent mailbox mutex");
        let mut messages = Vec::new();
        loop {
            match receiver.try_recv() {
                Ok(message) => messages.push(message),
                Err(broadcast::error::TryRecvError::Lagged(skipped)) => messages.push(format!(
                    "[mailbox lagged; {skipped} earlier parent message(s) were dropped]"
                )),
                Err(
                    broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed,
                ) => break,
            }
        }
        messages
    }
}

struct AgentRuntimeEntry {
    task_name: String,
    cancellation: CancellationToken,
    messages: broadcast::Sender<String>,
    status: AgentRuntimeStatus,
}

/// In-memory control plane for live children. Durable status and results remain
/// owned by DelegatedRunStore so restarts never invent a running agent.
#[derive(Clone, Default)]
pub struct AgentRuntimeManager {
    entries: Arc<Mutex<HashMap<String, AgentRuntimeEntry>>>,
    completion_tx: Arc<Mutex<Option<mpsc::UnboundedSender<ChildCompletionEvent>>>>,
}

impl AgentRuntimeManager {
    /// Register a completion listener for session wake (server wires this).
    pub fn set_completion_sender(&self, tx: mpsc::UnboundedSender<ChildCompletionEvent>) {
        *self.completion_tx.lock().expect("agent completion mutex") = Some(tx);
    }

    /// Notify parent session listeners that a background child finished.
    pub fn notify_completion(
        &self,
        event: ChildCompletionEvent,
    ) -> Result<(), ChildCompletionEvent> {
        let sender = self
            .completion_tx
            .lock()
            .expect("agent completion mutex")
            .as_ref()
            .cloned();
        let Some(sender) = sender else {
            return Err(event);
        };
        sender.send(event).map_err(|error| error.0)
    }

    pub fn register(
        &self,
        delegated_run_id: impl Into<String>,
        task_name: impl Into<String>,
        cancellation: CancellationToken,
    ) -> AgentMailbox {
        let delegated_run_id = delegated_run_id.into();
        let (messages, receiver) = broadcast::channel(64);
        self.entries.lock().expect("agent runtime mutex").insert(
            delegated_run_id,
            AgentRuntimeEntry {
                task_name: task_name.into(),
                cancellation,
                messages,
                status: AgentRuntimeStatus::Running,
            },
        );
        AgentMailbox::new(receiver)
    }

    pub fn subscribe(&self, delegated_run_id: &str) -> Result<AgentMailbox, String> {
        let entries = self.entries.lock().expect("agent runtime mutex");
        let entry = entries.get(delegated_run_id).ok_or_else(|| {
            format!("Agent '{delegated_run_id}' is not live in this server process")
        })?;
        if entry.status != AgentRuntimeStatus::Running {
            return Err(format!(
                "Agent '{delegated_run_id}' is {}; resume it as a new run instead",
                status_label(entry.status)
            ));
        }
        Ok(AgentMailbox::new(entry.messages.subscribe()))
    }

    pub fn send_message(&self, delegated_run_id: &str, message: String) -> Result<(), String> {
        let entries = self.entries.lock().expect("agent runtime mutex");
        let entry = entries.get(delegated_run_id).ok_or_else(|| {
            format!("Agent '{delegated_run_id}' is not live in this server process")
        })?;
        if entry.status != AgentRuntimeStatus::Running {
            return Err(format!(
                "Agent '{delegated_run_id}' is {}; resume it as a new run instead",
                status_label(entry.status)
            ));
        }
        entry
            .messages
            .send(message)
            .map_err(|_| format!("Agent '{delegated_run_id}' has no active mailbox"))?;
        Ok(())
    }

    pub fn cancel(&self, delegated_run_id: &str) -> Result<(), String> {
        let mut entries = self.entries.lock().expect("agent runtime mutex");
        let entry = entries.get_mut(delegated_run_id).ok_or_else(|| {
            format!("Agent '{delegated_run_id}' is not live in this server process")
        })?;
        entry.cancellation.cancel();
        entry.status = AgentRuntimeStatus::Cancelled;
        Ok(())
    }

    pub fn finish(&self, delegated_run_id: &str, _success: bool) {
        // Durable history lives in DelegatedRunStore. Keep this map limited to
        // live control targets so completed agents cannot accumulate forever.
        self.entries
            .lock()
            .expect("agent runtime mutex")
            .remove(delegated_run_id);
    }

    pub fn snapshots(&self) -> Vec<AgentRuntimeSnapshot> {
        let mut snapshots = self
            .entries
            .lock()
            .expect("agent runtime mutex")
            .iter()
            .map(|(run_id, entry)| AgentRuntimeSnapshot {
                delegated_run_id: run_id.clone(),
                task_name: entry.task_name.clone(),
                status: entry.status,
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.delegated_run_id.cmp(&right.delegated_run_id));
        snapshots
    }
}

fn status_label(status: AgentRuntimeStatus) -> &'static str {
    match status {
        AgentRuntimeStatus::Running => "running",
        AgentRuntimeStatus::Complete => "complete",
        AgentRuntimeStatus::Failed => "failed",
        AgentRuntimeStatus::Cancelled => "cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_manager_delivers_messages_and_individual_cancellation() {
        let manager = AgentRuntimeManager::default();
        let token = CancellationToken::new();
        let mailbox = manager.register("run-1", "audit", token.clone());
        let second_mailbox = manager.subscribe("run-1").unwrap();
        manager
            .send_message("run-1", "focus storage".into())
            .unwrap();
        assert_eq!(mailbox.drain(), vec!["focus storage"]);
        assert_eq!(second_mailbox.drain(), vec!["focus storage"]);
        manager.cancel("run-1").unwrap();
        assert!(token.is_cancelled());
        assert!(manager.send_message("run-1", "late".into()).is_err());
        manager.finish("run-1", false);
        assert!(manager.snapshots().is_empty());
    }
}
