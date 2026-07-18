use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration as ChronoDuration, Utc};
use krusty_core::agent::LoopInput;
use krusty_core::ai::types::Content;
use krusty_server::mako_execution_host::MakoExecutionHost;
use krusty_server::types::AgenticEvent;
use serde_json::{json, Value};

use crate::{ExecutionBackend, ExecutionControl, ExecutionOutcome, ExecutionRequest};

pub struct KrustyExecutionBackend {
    host: Arc<MakoExecutionHost>,
}

impl KrustyExecutionBackend {
    pub fn new(host: Arc<MakoExecutionHost>) -> Self {
        Self { host }
    }
}

#[derive(Default)]
struct TerminalState {
    finish_reason: Option<String>,
    error: Option<String>,
    sleeping: Option<(chrono::DateTime<Utc>, String)>,
    awaiting_input: Option<Value>,
}

impl TerminalState {
    fn observe(&mut self, event: &AgenticEvent) {
        match event {
            AgenticEvent::AgentSleeping {
                duration_secs,
                reason,
            } => {
                let seconds = i64::try_from(*duration_secs).unwrap_or(i64::MAX);
                let wake_at = Utc::now()
                    .checked_add_signed(ChronoDuration::seconds(seconds))
                    .unwrap_or_else(Utc::now);
                self.sleeping = Some((wake_at, reason.clone()));
            }
            AgenticEvent::AwaitingInput {
                tool_call_id,
                tool_name,
            } => {
                self.awaiting_input = Some(json!({
                    "tool_call_id": tool_call_id,
                    "tool_name": tool_name,
                }));
            }
            AgenticEvent::Error { error } => self.error = Some(error.clone()),
            AgenticEvent::Finish { stop_reason, .. } => {
                self.finish_reason = Some(stop_reason.clone());
            }
            _ => {}
        }
    }

    fn into_outcome(self, completion: std::result::Result<(), String>) -> ExecutionOutcome {
        if self.finish_reason.as_deref() == Some("user_abort") {
            return ExecutionOutcome::Cancelled {
                reason: "agent execution was cancelled".to_string(),
            };
        }
        if let Err(error) = completion {
            let error = self.error.unwrap_or(error);
            return ExecutionOutcome::Failed {
                retryable: transient_execution_error(&error),
                error,
                retry_after: None,
            };
        }

        match self.finish_reason.as_deref() {
            Some("completed" | "pinched") => ExecutionOutcome::Succeeded {
                output: json!({"stop_reason": self.finish_reason}),
            },
            Some("sleeping") => match self.sleeping {
                Some((wake_at, reason)) => ExecutionOutcome::Sleeping {
                    wake_at,
                    reason: Some(reason),
                },
                None => ExecutionOutcome::RecoveryRequired {
                    reason: "agent entered sleeping state without durable wake metadata".into(),
                },
            },
            Some("awaiting_input") => ExecutionOutcome::AwaitingInput {
                details: self
                    .awaiting_input
                    .unwrap_or_else(|| json!({"reason": "agent requested user input"})),
            },
            Some("provider_error" | "stream_idle_timeout") => ExecutionOutcome::Failed {
                error: self
                    .error
                    .unwrap_or_else(|| format!("agent stopped: {:?}", self.finish_reason)),
                retryable: true,
                retry_after: None,
            },
            Some(reason) => ExecutionOutcome::Failed {
                error: self
                    .error
                    .unwrap_or_else(|| format!("agent stopped: {reason}")),
                retryable: false,
                retry_after: None,
            },
            None => ExecutionOutcome::Failed {
                error: self
                    .error
                    .unwrap_or_else(|| "agent event stream ended without a terminal event".into()),
                retryable: true,
                retry_after: None,
            },
        }
    }
}

fn transient_execution_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    !normalized.contains("no ai credentials configured")
        && !normalized.contains("session not found")
        && !normalized.contains("not a mako session")
        && !normalized.contains("event payload is")
}

#[async_trait]
impl ExecutionBackend for KrustyExecutionBackend {
    async fn execute(&self, request: ExecutionRequest) -> ExecutionOutcome {
        let Some(session_id) = request.claim.run.session_id.clone() else {
            return ExecutionOutcome::Failed {
                error: "claimed Mako run has no session".into(),
                retryable: false,
                retry_after: None,
            };
        };
        let run_id = request.claim.run.id.clone();
        let wake_reason = request.claim.run.kind.clone();
        let run = match self
            .host
            .start(session_id.clone(), run_id, wake_reason)
            .await
        {
            Ok(run) => run,
            Err(error) => {
                let error = error.to_string();
                return ExecutionOutcome::Failed {
                    retryable: transient_execution_error(&error),
                    error,
                    retry_after: None,
                };
            }
        };

        let (mut events, mut completion, _execution_guard) = run.into_parts();
        let mut terminal = TerminalState::default();
        let mut completion_result = None;
        let mut events_open = true;
        while completion_result.is_none() || events_open {
            tokio::select! {
                event = events.recv(), if events_open => {
                    match event {
                        Some(event) => {
                            terminal.observe(&event);
                            let payload = match serde_json::to_value(event) {
                                Ok(payload) => payload,
                                Err(error) => {
                                    self.host.abort(&session_id).await;
                                    return ExecutionOutcome::Failed {
                                        error: format!("could not encode agent event: {error}"),
                                        retryable: false,
                                        retry_after: None,
                                    };
                                }
                            };
                            if let Err(error) = request.events.agentic(payload).await {
                                self.host.abort(&session_id).await;
                                return ExecutionOutcome::Failed {
                                    error: format!("could not durably emit agent event: {error}"),
                                    retryable: false,
                                    retry_after: None,
                                };
                            }
                        }
                        None => events_open = false,
                    }
                }
                result = &mut completion, if completion_result.is_none() => {
                    completion_result = Some(result.unwrap_or_else(|_| {
                        Err("Mako execution completion channel closed".to_string())
                    }));
                }
            }
        }
        terminal.into_outcome(
            completion_result
                .unwrap_or_else(|| Err("Mako execution completed without a result".to_string())),
        )
    }

    async fn control(&self, session_id: &str, control: ExecutionControl) -> anyhow::Result<()> {
        match control {
            // Start is a scheduler wake-up hint. The durable pump owns actual
            // execution and will call `execute` after successfully claiming.
            ExecutionControl::Start => Ok(()),
            ExecutionControl::Message { message, persisted } => {
                let content = vec![Content::Text { text: message }];
                let input = if persisted {
                    LoopInput::PersistedUserMessage { content }
                } else {
                    LoopInput::Steer {
                        pending_id: None,
                        content,
                    }
                };
                self.host.send_input(session_id, input).await
            }
            ExecutionControl::Steer {
                pending_id,
                content,
            } => {
                let content: Vec<Content> = serde_json::from_value(content)
                    .map_err(|error| anyhow::anyhow!("invalid steering content: {error}"))?;
                self.host
                    .send_input(
                        session_id,
                        LoopInput::Steer {
                            pending_id,
                            content,
                        },
                    )
                    .await
            }
            ExecutionControl::ToolApproval {
                tool_call_id,
                approved,
            } => {
                self.host
                    .send_input(
                        session_id,
                        LoopInput::ToolApproval {
                            tool_call_id,
                            approved,
                        },
                    )
                    .await
            }
            ExecutionControl::UserResponse {
                tool_call_id,
                response,
            } => {
                self.host
                    .send_input(
                        session_id,
                        LoopInput::UserResponse {
                            tool_call_id,
                            response,
                        },
                    )
                    .await
            }
            ExecutionControl::Cancel { reason } => {
                tracing::info!(session_id, reason, "Cancelling hosted Mako execution");
                self.host.cancel(session_id).await
            }
        }
    }
}
