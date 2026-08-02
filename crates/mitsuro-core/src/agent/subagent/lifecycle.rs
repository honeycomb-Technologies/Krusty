use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{ensure, Context};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::agent::DelegatedRunStage;
use crate::ai::types::Content;
use crate::storage::DelegatedRunRecord;

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
    pub terminal_stage: DelegatedRunStage,
    pub outcome: String,
    pub usable_agents: usize,
    /// Compatibility projection for clients that have not adopted the richer
    /// terminal-stage contract yet. Only `Complete` maps to `true`.
    pub success: bool,
    pub summary: String,
}

impl ChildCompletionEvent {
    /// Reconstruct the complete wake payload from the authoritative durable
    /// row. Both live completion and crash recovery use this constructor so
    /// pending steering cannot drift from the stored terminal artifact.
    pub fn from_durable_run(
        delegated: &DelegatedRunRecord,
        user_id: Option<String>,
    ) -> anyhow::Result<Self> {
        ensure!(
            delegated.should_wake_parent(),
            "delegated run is not eligible to wake its parent"
        );
        ensure!(
            delegated.completed_at.is_some(),
            "delegated run has no durable completion timestamp"
        );
        ensure!(
            delegated.artifact.is_some(),
            "delegated run has no durable artifact"
        );

        let workspace_scopes = delegated
            .target_scope
            .iter()
            .filter(|scope| scope.kind == "workspace")
            .collect::<Vec<_>>();
        let [workspace_scope] = workspace_scopes.as_slice() else {
            anyhow::bail!("delegated run has no unique launch workspace");
        };
        let workspace_root = PathBuf::from(&workspace_scope.path)
            .canonicalize()
            .context("canonicalizing delegated launch workspace")?;
        ensure!(
            workspace_root.is_dir(),
            "delegated workspace is not a directory"
        );

        let terminal_stage = delegated.stage;
        let success = terminal_stage == DelegatedRunStage::Complete;
        let outcome = delegated
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.get("outcome"))
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string)
            .unwrap_or_else(|| child_terminal_stage_label(terminal_stage).to_string());
        let usable_agents = delegated
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.get("usable_agents"))
            .and_then(serde_json::Value::as_u64)
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(usize::from(success));
        let summary = delegated
            .human_review
            .clone()
            .context("delegated run has no durable review summary")?;
        let task_name = delegated
            .child_name
            .clone()
            .unwrap_or_else(|| "child".to_string());
        let pending_id = format!("child-wake-{}", delegated.delegated_run_id);
        let content = child_completion_content(
            &task_name,
            &delegated.delegated_run_id,
            terminal_stage,
            &outcome,
            usable_agents,
            success,
            &summary,
        );

        Ok(Self {
            session_id: Some(delegated.parent_session_id.clone()),
            user_id,
            workspace_root: Some(workspace_root),
            pending_id,
            content,
            delegated_run_id: delegated.delegated_run_id.clone(),
            task_name,
            terminal_stage,
            outcome,
            usable_agents,
            success,
            summary,
        })
    }
}

fn child_completion_content(
    name: &str,
    delegated_run_id: &str,
    terminal_stage: DelegatedRunStage,
    outcome: &str,
    usable_agents: usize,
    success: bool,
    summary: &str,
) -> Vec<Content> {
    let body = format!(
        "[CHILD AGENT COMPLETE]\nname: {name}\ndelegated_run_id: {delegated_run_id}\nterminal_stage: {}\noutcome: {outcome}\nusable_agents: {usable_agents}\nsuccess: {success}\nsummary:\n{summary}\n\nContinue from this result. Integrate usable evidence from degraded outcomes; report a total failure accurately. Do not re-poll status for this delegated_run_id unless you need more detail.\n",
        child_terminal_stage_label(terminal_stage),
    );
    vec![Content::Text { text: body }]
}

fn child_terminal_stage_label(stage: DelegatedRunStage) -> &'static str {
    match stage {
        DelegatedRunStage::Created => "created",
        DelegatedRunStage::Running => "running",
        DelegatedRunStage::Synthesizing => "synthesizing",
        DelegatedRunStage::Complete => "complete",
        DelegatedRunStage::Degraded => "degraded",
        DelegatedRunStage::Failed => "failed",
        DelegatedRunStage::Cancelled => "cancelled",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRuntimeStatus {
    Running,
    Cancelling,
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
    worker: Arc<AgentMailboxWorker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentMailboxFinish {
    Continue(Vec<String>),
    WorkerFinished,
    LastWorkerSealed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentMailboxPhase {
    Accepting,
    Finishing,
    Cancelled,
}

#[derive(Debug)]
struct AgentMailboxGate {
    phase: AgentMailboxPhase,
    active_workers: usize,
}

#[derive(Debug)]
struct AgentMailboxWorker {
    receiver: Mutex<broadcast::Receiver<String>>,
    gate: Arc<Mutex<AgentMailboxGate>>,
    active: AtomicBool,
}

impl Drop for AgentMailboxWorker {
    fn drop(&mut self) {
        if !self.active.swap(false, Ordering::AcqRel) {
            return;
        }

        let mut gate = self.gate.lock().expect("agent mailbox gate mutex");
        gate.active_workers = gate.active_workers.saturating_sub(1);
        if gate.active_workers == 0 && gate.phase == AgentMailboxPhase::Accepting {
            gate.phase = AgentMailboxPhase::Finishing;
        }
    }
}

impl AgentMailbox {
    fn new(receiver: broadcast::Receiver<String>, gate: Arc<Mutex<AgentMailboxGate>>) -> Self {
        Self {
            worker: Arc::new(AgentMailboxWorker {
                receiver: Mutex::new(receiver),
                gate,
                active: AtomicBool::new(true),
            }),
        }
    }

    pub fn drain(&self) -> Vec<String> {
        let _gate = self.worker.gate.lock().expect("agent mailbox gate mutex");
        self.drain_locked()
    }

    /// Atomically consume every message accepted for this worker or retire it
    /// from the run. The last live worker seals delivery before returning, so
    /// `send_message` can never report success after the loop's final drain.
    pub(crate) fn drain_or_seal_for_finish(&self) -> AgentMailboxFinish {
        let mut gate = self.worker.gate.lock().expect("agent mailbox gate mutex");
        if gate.phase == AgentMailboxPhase::Cancelled {
            return AgentMailboxFinish::Cancelled;
        }

        let messages = self.drain_locked();
        if !messages.is_empty() {
            return AgentMailboxFinish::Continue(messages);
        }

        if !self.retire_locked(&mut gate) {
            return if gate.phase == AgentMailboxPhase::Cancelled {
                AgentMailboxFinish::Cancelled
            } else {
                AgentMailboxFinish::WorkerFinished
            };
        }

        if gate.active_workers == 0 {
            AgentMailboxFinish::LastWorkerSealed
        } else {
            AgentMailboxFinish::WorkerFinished
        }
    }

    /// Atomically take any already-accepted messages and retire this worker
    /// before a non-continuable terminal return. The caller may persist those
    /// messages as continuation evidence instead of silently discarding a
    /// parent steering request that was accepted just before failure. Other
    /// subscribers remain live; the final worker seals the shared gate.
    pub(crate) fn seal_for_terminal(&self) -> Vec<String> {
        let mut gate = self.worker.gate.lock().expect("agent mailbox gate mutex");
        let messages = self.drain_locked();
        self.retire_locked(&mut gate);
        messages
    }

    fn retire_locked(&self, gate: &mut AgentMailboxGate) -> bool {
        if !self.worker.active.swap(false, Ordering::AcqRel) {
            return false;
        }

        gate.active_workers = gate.active_workers.saturating_sub(1);
        if gate.active_workers == 0 && gate.phase == AgentMailboxPhase::Accepting {
            gate.phase = AgentMailboxPhase::Finishing;
        }
        true
    }

    fn drain_locked(&self) -> Vec<String> {
        let mut receiver = self.worker.receiver.lock().expect("agent mailbox mutex");
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
    parent_session_id: Option<String>,
    cancellation: CancellationToken,
    messages: broadcast::Sender<String>,
    mailbox_gate: Arc<Mutex<AgentMailboxGate>>,
    registration_token: Arc<()>,
    status: AgentRuntimeStatus,
}

/// RAII ownership for one live background runtime registration.
///
/// Background tasks move this guard into their spawned future. Normal
/// completion may call `finish`; panic, abort, or early future drop falls back
/// to `Drop`, ensuring stale in-memory ownership never blocks durable resume.
/// The opaque token prevents an old guard from removing a newer registration
/// that happens to reuse the same durable run ID.
pub(crate) struct AgentRuntimeRegistration {
    runtime: AgentRuntimeManager,
    delegated_run_id: Option<String>,
    registration_token: Arc<()>,
}

impl AgentRuntimeRegistration {
    pub(crate) fn finish(&mut self, success: bool) {
        let Some(delegated_run_id) = self.delegated_run_id.take() else {
            return;
        };
        self.runtime
            .finish_registration(&delegated_run_id, &self.registration_token, success);
    }
}

impl Drop for AgentRuntimeRegistration {
    fn drop(&mut self) {
        let Some(delegated_run_id) = self.delegated_run_id.take() else {
            return;
        };
        if self
            .runtime
            .finish_registration(&delegated_run_id, &self.registration_token, false)
        {
            if let Err(delegated_run_id) = self
                .runtime
                .request_completion_reconciliation(delegated_run_id)
            {
                warn!(
                    delegated_run_id,
                    "Background Agent ownership ended abnormally without a live reconciliation listener; startup recovery must publish its durable outcome"
                );
            }
        }
    }
}

/// In-memory control plane for live children. Durable status and results remain
/// owned by DelegatedRunStore so restarts never invent a running agent.
#[derive(Clone, Default)]
pub struct AgentRuntimeManager {
    entries: Arc<Mutex<HashMap<String, AgentRuntimeEntry>>>,
    completion_tx: Arc<Mutex<Option<mpsc::UnboundedSender<ChildCompletionEvent>>>>,
    completion_reconciliation_tx: Arc<Mutex<Option<mpsc::UnboundedSender<String>>>>,
}

impl AgentRuntimeManager {
    /// Register a completion listener for session wake (server wires this).
    pub fn set_completion_sender(&self, tx: mpsc::UnboundedSender<ChildCompletionEvent>) {
        *self.completion_tx.lock().expect("agent completion mutex") = Some(tx);
    }

    /// Register the server's durable reconciliation listener. A guarded
    /// background registration sends only its run ID here when panic/abort or
    /// future drop bypasses normal completion publication.
    pub fn set_completion_reconciliation_sender(&self, tx: mpsc::UnboundedSender<String>) {
        *self
            .completion_reconciliation_tx
            .lock()
            .expect("agent completion reconciliation mutex") = Some(tx);
    }

    /// Whether this process currently has a live host capable of turning a
    /// durable child completion into parent-loop input. CLI/TUI and ACP do not
    /// yet provide that lifecycle host, so background execution must fail
    /// closed there instead of promising a wake that can never occur.
    pub fn has_completion_listener(&self) -> bool {
        let completion_live = self
            .completion_tx
            .lock()
            .expect("agent completion mutex")
            .as_ref()
            .is_some_and(|sender| !sender.is_closed());
        let reconciliation_live = self
            .completion_reconciliation_tx
            .lock()
            .expect("agent completion reconciliation mutex")
            .as_ref()
            .is_some_and(|sender| !sender.is_closed());
        completion_live && reconciliation_live
    }

    /// Notify parent session listeners that a background child finished.
    pub fn notify_completion(
        &self,
        event: ChildCompletionEvent,
    ) -> Result<(), Box<ChildCompletionEvent>> {
        let sender = self
            .completion_tx
            .lock()
            .expect("agent completion mutex")
            .as_ref()
            .cloned();
        let Some(sender) = sender else {
            return Err(Box::new(event));
        };
        sender.send(event).map_err(|error| Box::new(error.0))
    }

    pub fn request_completion_reconciliation(
        &self,
        delegated_run_id: String,
    ) -> Result<(), String> {
        let sender = self
            .completion_reconciliation_tx
            .lock()
            .expect("agent completion reconciliation mutex")
            .as_ref()
            .cloned();
        let Some(sender) = sender else {
            return Err(delegated_run_id);
        };
        sender.send(delegated_run_id).map_err(|error| error.0)
    }

    pub fn register(
        &self,
        delegated_run_id: impl Into<String>,
        task_name: impl Into<String>,
        parent_session_id: Option<String>,
        cancellation: CancellationToken,
    ) -> AgentMailbox {
        self.register_entry(
            delegated_run_id.into(),
            task_name.into(),
            parent_session_id,
            cancellation,
        )
        .0
    }

    pub(crate) fn register_guarded(
        &self,
        delegated_run_id: impl Into<String>,
        task_name: impl Into<String>,
        parent_session_id: Option<String>,
        cancellation: CancellationToken,
    ) -> (AgentMailbox, AgentRuntimeRegistration) {
        let delegated_run_id = delegated_run_id.into();
        let (mailbox, registration_token) = self.register_entry(
            delegated_run_id.clone(),
            task_name.into(),
            parent_session_id,
            cancellation,
        );
        let registration = AgentRuntimeRegistration {
            runtime: self.clone(),
            delegated_run_id: Some(delegated_run_id),
            registration_token,
        };
        (mailbox, registration)
    }

    fn register_entry(
        &self,
        delegated_run_id: String,
        task_name: String,
        parent_session_id: Option<String>,
        cancellation: CancellationToken,
    ) -> (AgentMailbox, Arc<()>) {
        let (messages, receiver) = broadcast::channel(64);
        let mailbox_gate = Arc::new(Mutex::new(AgentMailboxGate {
            phase: AgentMailboxPhase::Accepting,
            active_workers: 1,
        }));
        let registration_token = Arc::new(());
        self.entries.lock().expect("agent runtime mutex").insert(
            delegated_run_id,
            AgentRuntimeEntry {
                task_name,
                parent_session_id,
                cancellation,
                messages,
                mailbox_gate: Arc::clone(&mailbox_gate),
                registration_token: Arc::clone(&registration_token),
                status: AgentRuntimeStatus::Running,
            },
        );
        (
            AgentMailbox::new(receiver, mailbox_gate),
            registration_token,
        )
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
        let mut gate = entry.mailbox_gate.lock().expect("agent mailbox gate mutex");
        if gate.phase != AgentMailboxPhase::Accepting {
            return Err(format!(
                "Agent '{delegated_run_id}' is finishing; resume it as a new run instead"
            ));
        }
        gate.active_workers = gate.active_workers.saturating_add(1);
        Ok(AgentMailbox::new(
            entry.messages.subscribe(),
            Arc::clone(&entry.mailbox_gate),
        ))
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
        let gate = entry.mailbox_gate.lock().expect("agent mailbox gate mutex");
        if gate.phase != AgentMailboxPhase::Accepting {
            return Err(format!(
                "Agent '{delegated_run_id}' is finishing; resume it as a new run instead"
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
        entry
            .mailbox_gate
            .lock()
            .expect("agent mailbox gate mutex")
            .phase = AgentMailboxPhase::Cancelled;
        entry.cancellation.cancel();
        entry.status = AgentRuntimeStatus::Cancelling;
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

    fn finish_registration(
        &self,
        delegated_run_id: &str,
        registration_token: &Arc<()>,
        _success: bool,
    ) -> bool {
        let mut entries = self.entries.lock().expect("agent runtime mutex");
        let owns_current_registration = entries
            .get(delegated_run_id)
            .is_some_and(|entry| Arc::ptr_eq(&entry.registration_token, registration_token));
        if owns_current_registration {
            entries.remove(delegated_run_id);
        }
        owns_current_registration
    }

    /// Whether this process still owns a child entry, including one that is
    /// cancelling but has not finished unwinding. Durable resume may proceed
    /// for stale running rows only when no live process entry exists.
    pub fn contains(&self, delegated_run_id: &str) -> bool {
        self.entries
            .lock()
            .expect("agent runtime mutex")
            .contains_key(delegated_run_id)
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

    pub fn snapshots_for_session(&self, parent_session_id: &str) -> Vec<AgentRuntimeSnapshot> {
        let mut snapshots = self
            .entries
            .lock()
            .expect("agent runtime mutex")
            .iter()
            .filter(|(_, entry)| entry.parent_session_id.as_deref() == Some(parent_session_id))
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
        AgentRuntimeStatus::Cancelling => "cancelling",
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
        let mailbox = manager.register(
            "run-1",
            "audit",
            Some("session-a".to_string()),
            token.clone(),
        );
        let second_mailbox = manager.subscribe("run-1").unwrap();
        manager
            .send_message("run-1", "focus storage".into())
            .unwrap();
        assert_eq!(mailbox.drain(), vec!["focus storage"]);
        assert_eq!(second_mailbox.drain(), vec!["focus storage"]);
        manager.cancel("run-1").unwrap();
        assert!(token.is_cancelled());
        assert!(manager.contains("run-1"));
        assert_eq!(
            manager.snapshots()[0].status,
            AgentRuntimeStatus::Cancelling
        );
        assert!(manager.send_message("run-1", "late".into()).is_err());
        manager.finish("run-1", false);
        assert!(!manager.contains("run-1"));
        assert!(manager.snapshots().is_empty());
    }

    #[test]
    fn terminal_seal_rejects_messages_after_the_final_drain() {
        let manager = AgentRuntimeManager::default();
        let mailbox = manager.register(
            "run-seal",
            "sealing child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );

        assert_eq!(
            mailbox.drain_or_seal_for_finish(),
            AgentMailboxFinish::LastWorkerSealed
        );
        assert!(manager
            .send_message("run-seal", "too late".to_string())
            .is_err());
    }

    #[test]
    fn failure_seal_returns_accepted_messages_and_rejects_late_delivery() {
        let manager = AgentRuntimeManager::default();
        let mailbox = manager.register(
            "run-failure-seal",
            "failing child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );
        manager
            .send_message("run-failure-seal", "accepted before failure".to_string())
            .expect("message should be accepted before the terminal decision");

        let pending = mailbox.seal_for_terminal();

        assert_eq!(pending, vec!["accepted before failure"]);
        assert!(mailbox.drain().is_empty());
        assert!(manager
            .send_message("run-failure-seal", "too late".to_string())
            .is_err());
    }

    #[test]
    fn accepted_message_is_drained_before_a_worker_can_finish() {
        let manager = AgentRuntimeManager::default();
        let mailbox = manager.register(
            "run-message",
            "steered child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );
        manager
            .send_message("run-message", "inspect storage".to_string())
            .expect("message should be accepted");

        assert_eq!(
            mailbox.drain_or_seal_for_finish(),
            AgentMailboxFinish::Continue(vec!["inspect storage".to_string()])
        );
        assert!(manager
            .send_message("run-message", "then inspect routes".to_string())
            .is_ok());
    }

    #[test]
    fn one_parallel_worker_cannot_seal_surviving_subscribers() {
        let manager = AgentRuntimeManager::default();
        let first = manager.register(
            "run-parallel",
            "parallel build",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );
        let second = manager
            .subscribe("run-parallel")
            .expect("second worker should subscribe");

        assert_eq!(
            first.drain_or_seal_for_finish(),
            AgentMailboxFinish::WorkerFinished
        );
        manager
            .send_message("run-parallel", "focus the surviving worker".to_string())
            .expect("surviving worker should keep the gate open");
        assert_eq!(
            second.drain_or_seal_for_finish(),
            AgentMailboxFinish::Continue(vec!["focus the surviving worker".to_string()])
        );
        assert_eq!(
            second.drain_or_seal_for_finish(),
            AgentMailboxFinish::LastWorkerSealed
        );
        assert!(manager
            .send_message("run-parallel", "late".to_string())
            .is_err());
    }

    #[test]
    fn cancellation_closes_the_shared_mailbox_gate() {
        let manager = AgentRuntimeManager::default();
        let mailbox = manager.register(
            "run-cancel",
            "cancelled child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );

        manager.cancel("run-cancel").expect("cancel live child");
        assert_eq!(
            mailbox.drain_or_seal_for_finish(),
            AgentMailboxFinish::Cancelled
        );
        assert!(manager
            .send_message("run-cancel", "late".to_string())
            .is_err());
    }

    #[test]
    fn runtime_snapshots_are_isolated_by_parent_session() {
        let manager = AgentRuntimeManager::default();
        manager.register(
            "run-a",
            "alpha audit",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );
        manager.register(
            "run-b",
            "beta repair",
            Some("session-b".to_string()),
            CancellationToken::new(),
        );
        manager.register("run-local", "unbound", None, CancellationToken::new());

        let session_a = manager.snapshots_for_session("session-a");
        assert_eq!(session_a.len(), 1);
        assert_eq!(session_a[0].delegated_run_id, "run-a");
        assert_eq!(session_a[0].task_name, "alpha audit");
        assert!(manager.snapshots_for_session("missing").is_empty());
    }

    #[test]
    fn completion_listener_health_tracks_receiver_lifetime() {
        let manager = AgentRuntimeManager::default();
        assert!(!manager.has_completion_listener());
        let (sender, receiver) = mpsc::unbounded_channel();
        manager.set_completion_sender(sender);
        assert!(!manager.has_completion_listener());
        let (reconciliation_sender, reconciliation_receiver) = mpsc::unbounded_channel();
        manager.set_completion_reconciliation_sender(reconciliation_sender);
        assert!(manager.has_completion_listener());
        drop(receiver);
        assert!(!manager.has_completion_listener());
        drop(reconciliation_receiver);
    }

    #[test]
    fn guarded_registration_finish_is_idempotent() {
        let manager = AgentRuntimeManager::default();
        let (reconciliation_tx, mut reconciliation_rx) = mpsc::unbounded_channel();
        manager.set_completion_reconciliation_sender(reconciliation_tx);
        let (_mailbox, mut registration) = manager.register_guarded(
            "run-guarded",
            "guarded child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );

        assert!(manager.contains("run-guarded"));
        registration.finish(true);
        registration.finish(false);
        assert!(!manager.contains("run-guarded"));
        assert!(reconciliation_rx.try_recv().is_err());
    }

    #[test]
    fn stale_guard_cannot_remove_a_newer_registration() {
        let manager = AgentRuntimeManager::default();
        let (_old_mailbox, old_registration) = manager.register_guarded(
            "run-reused",
            "old child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );
        let _new_mailbox = manager.register(
            "run-reused",
            "new child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );

        drop(old_registration);

        assert!(manager.contains("run-reused"));
        assert_eq!(manager.snapshots()[0].task_name, "new child");
    }

    #[tokio::test]
    async fn guarded_registration_is_released_when_task_panics() {
        let manager = AgentRuntimeManager::default();
        let (reconciliation_tx, mut reconciliation_rx) = mpsc::unbounded_channel();
        manager.set_completion_reconciliation_sender(reconciliation_tx);
        let (_mailbox, registration) = manager.register_guarded(
            "run-panic",
            "panicking child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );

        let handle = tokio::spawn(async move {
            let _registration = registration;
            panic!("simulated background task panic");
        });
        let error = handle.await.expect_err("task should panic");

        assert!(error.is_panic());
        assert!(!manager.contains("run-panic"));
        assert_eq!(reconciliation_rx.recv().await.as_deref(), Some("run-panic"));
    }

    #[tokio::test]
    async fn guarded_registration_is_released_when_task_is_aborted() {
        let manager = AgentRuntimeManager::default();
        let (reconciliation_tx, mut reconciliation_rx) = mpsc::unbounded_channel();
        manager.set_completion_reconciliation_sender(reconciliation_tx);
        let (_mailbox, registration) = manager.register_guarded(
            "run-abort",
            "aborted child",
            Some("session-a".to_string()),
            CancellationToken::new(),
        );
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let handle = tokio::spawn(async move {
            let _registration = registration;
            let _ = started_tx.send(());
            std::future::pending::<()>().await;
        });
        started_rx.await.expect("task should start");
        assert!(manager.contains("run-abort"));

        handle.abort();
        let error = handle.await.expect_err("task should be aborted");

        assert!(error.is_cancelled());
        assert!(!manager.contains("run-abort"));
        assert_eq!(reconciliation_rx.recv().await.as_deref(), Some("run-abort"));
    }
}
