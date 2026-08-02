//! Live `LoopEvent` projection.

use mitsuro_core::{
    agent::{loop_events::LoopStopReason, LoopEvent},
    ai::types::Citation,
};

use crate::tui_v2::model::{
    artifact::{
        ArtifactContent, ArtifactModel, BoundedText, PartId, WebDocumentArtifact, WebResultArtifact,
    },
    conversation::{
        CitationModel, CompactionPart, ErrorPart, NoticeLevel, NoticePart, PendingInteraction,
        PendingPlanConfirmation, PendingQuestions, PendingToolApproval, PlanTaskModel,
        QuestionModel, QuestionOptionModel, RunBudgetSnapshot, TimelinePart, ToolStatus, TurnState,
        UsageSnapshot, WorkflowRevision,
    },
};

use super::{
    tool_output::{
        append_tool_delta, bound_text, finalize_tool_output, parse_tool_arguments,
        parse_tool_output,
    },
    ConversationProjection,
};

impl ConversationProjection {
    pub fn apply_event(&mut self, event: LoopEvent) {
        match event {
            LoopEvent::TextDelta { delta } => self.append_agent_text(&delta, Vec::new()),
            LoopEvent::TextDeltaWithCitations { delta, citations } => {
                self.append_agent_text(&delta, citations.into_iter().map(map_citation).collect());
            }
            LoopEvent::ThinkingDelta { thinking } => self.append_thinking(&thinking),
            LoopEvent::ThinkingComplete {
                thinking,
                signature,
            } => self.complete_thinking(&thinking, Some(signature)),
            LoopEvent::ToolCallStart { id, name } => {
                self.upsert_tool(&id, &name, ToolStatus::Receiving, false);
            }
            LoopEvent::ToolCallComplete {
                id,
                name,
                arguments,
            } => {
                let parsed = parse_tool_arguments(&arguments);
                self.upsert_tool(&id, &name, ToolStatus::Pending, false)
                    .arguments = parsed;
                self.prepare_special_interaction(&id, &name, &arguments);
            }
            LoopEvent::ToolExecuting { id, name } => {
                self.upsert_tool(&id, &name, ToolStatus::Running, false);
            }
            LoopEvent::ToolOutputDelta { id, delta } => {
                let tool = self.upsert_tool(&id, "", ToolStatus::Running, false);
                append_tool_delta(&mut tool.artifact, &delta);
            }
            LoopEvent::ToolResult {
                id,
                output,
                is_error,
            } => {
                let tool = self.upsert_tool(
                    &id,
                    "",
                    if is_error {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Succeeded
                    },
                    false,
                );
                finalize_tool_output(&mut tool.artifact, &tool.name, &output);
                self.clear_pending_interaction(&id);
            }
            LoopEvent::AwaitingInput {
                tool_call_id,
                tool_name,
            } => {
                self.current_turn_mut().state = TurnState::AwaitingInput;
                let status = if tool_name.eq_ignore_ascii_case("askuserquestion")
                    || tool_name.eq_ignore_ascii_case("planconfirm")
                {
                    ToolStatus::Pending
                } else {
                    ToolStatus::AwaitingApproval
                };
                self.upsert_tool(&tool_call_id, &tool_name, status, false);
            }
            LoopEvent::ToolApprovalRequired {
                id,
                name,
                arguments,
            } => {
                let arguments = parse_tool_arguments(&arguments);
                let session_id = self.session_id.clone();
                self.upsert_tool(&id, &name, ToolStatus::AwaitingApproval, false)
                    .arguments = arguments.clone();
                self.set_pending_interaction(PendingInteraction::ToolApproval(
                    PendingToolApproval {
                        session_id,
                        tool_call_id: id,
                        tool_name: name,
                        arguments,
                    },
                ));
                self.current_turn_mut().state = TurnState::AwaitingInput;
            }
            LoopEvent::ToolApproved { id } => {
                if let Some(tool) = self.tool_mut(&id) {
                    tool.status = ToolStatus::Approved;
                }
                self.clear_pending_interaction(&id);
                self.current_turn_mut().state = TurnState::Live;
            }
            LoopEvent::ToolDenied { id } => {
                if let Some(tool) = self.tool_mut(&id) {
                    tool.status = ToolStatus::Denied;
                }
                self.clear_pending_interaction(&id);
                self.current_turn_mut().state = TurnState::Live;
            }
            LoopEvent::SteeringInjected {
                pending_id,
                message,
            } => {
                let ordinal = self.presentation().turns.len();
                let id = pending_id.unwrap_or_else(|| format!("derived:{ordinal}"));
                self.push_user_prompt(&format!("steering:{id}"), message, Vec::new(), true);
            }
            LoopEvent::ServerToolStart { id, name } => {
                self.upsert_tool(&id, &name, ToolStatus::Running, true);
            }
            LoopEvent::ServerToolComplete { id, name } => {
                if let Some(tool) = self.tool_mut(&id) {
                    if !name.is_empty() {
                        tool.name = name;
                    }
                    tool.server_side = true;
                    if !matches!(tool.status, ToolStatus::Failed) {
                        tool.status = ToolStatus::Succeeded;
                    }
                } else {
                    self.upsert_tool(&id, &name, ToolStatus::Succeeded, true);
                }
            }
            LoopEvent::WebSearchResults {
                tool_use_id,
                results,
            } => {
                let tool = self.upsert_tool(&tool_use_id, "web_search", ToolStatus::Running, true);
                tool.artifact = ArtifactModel {
                    content: ArtifactContent::WebResults(
                        results
                            .into_iter()
                            .take(100)
                            .map(|result| WebResultArtifact {
                                title: result.title,
                                url: result.url,
                                age: result.page_age,
                            })
                            .collect(),
                    ),
                    ..ArtifactModel::default()
                };
            }
            LoopEvent::WebFetchResult {
                tool_use_id,
                content,
            } => {
                let tool = self.upsert_tool(&tool_use_id, "web_fetch", ToolStatus::Running, true);
                tool.artifact = ArtifactModel {
                    content: ArtifactContent::WebDocument(WebDocumentArtifact {
                        title: content.title,
                        url: content.url,
                        media_type: content.media_type,
                        content: bound_text(
                            &super::tool_output::sanitize_terminal_text(&content.content),
                            super::tool_output::LIVE_ARTIFACT_BYTES,
                        ),
                    }),
                    ..ArtifactModel::default()
                };
            }
            LoopEvent::ServerToolError {
                tool_use_id,
                error_code,
            } => {
                let tool = self.upsert_tool(&tool_use_id, "", ToolStatus::Failed, true);
                tool.artifact = ArtifactModel {
                    content: ArtifactContent::Text(BoundedText {
                        text: format!("Server tool failed: {error_code}"),
                        omitted_bytes: 0,
                    }),
                    ..ArtifactModel::default()
                };
            }
            LoopEvent::ModeChange { mode, .. } => self.presentation.metadata.mode = Some(mode),
            LoopEvent::PlanUpdate { tasks } => {
                self.presentation.metadata.plan_tasks = tasks
                    .into_iter()
                    .map(|task| PlanTaskModel {
                        description: task.description,
                        completed: task.completed,
                    })
                    .collect();
            }
            LoopEvent::WorkflowUpdated {
                goal_id,
                aggregate_revision,
                operation_id,
            } => {
                let should_update = self
                    .presentation
                    .metadata
                    .workflow
                    .as_ref()
                    .is_none_or(|current| aggregate_revision > current.aggregate_revision);
                if should_update {
                    self.presentation.metadata.workflow = Some(WorkflowRevision {
                        goal_id,
                        aggregate_revision,
                        operation_id,
                    });
                }
            }
            LoopEvent::PlanComplete {
                tool_call_id,
                title,
                task_count,
            } => {
                self.set_pending_interaction(PendingInteraction::PlanConfirm(
                    PendingPlanConfirmation {
                        session_id: self.session_id.clone(),
                        tool_call_id,
                        title,
                        task_count,
                        tasks: self.presentation.metadata.plan_tasks.clone(),
                    },
                ));
                self.current_turn_mut().state = TurnState::AwaitingInput;
            }
            LoopEvent::AgentSleeping {
                duration_secs,
                reason,
            } => {
                let ordinal = self.current_turn_mut().parts.len();
                self.push_part(TimelinePart::Notice(NoticePart {
                    id: PartId::from_semantic(format!("sleep:{ordinal}")),
                    message: format!("Waiting {duration_secs}s — {reason}"),
                    level: NoticeLevel::Neutral,
                    expandable: None,
                }));
            }
            LoopEvent::TurnComplete { has_more, .. } => {
                self.settle_streaming_parts(ToolStatus::Interrupted);
                if !has_more {
                    self.current_turn_mut().state = TurnState::Completed;
                    crate::tui_v2::presentation::retention::apply_historical_retention(
                        &mut self.presentation,
                    );
                }
            }
            LoopEvent::RunBudgetResolved { max_turns, source } => {
                self.presentation.metadata.run_budget = Some(RunBudgetSnapshot {
                    max_turns,
                    source: format!("{source:?}"),
                });
            }
            LoopEvent::ProviderRequestPrepared { .. }
            | LoopEvent::MicrocompactionApplied { .. }
            | LoopEvent::TickInjected { .. }
            | LoopEvent::ClassifierDecision { .. } => {}
            LoopEvent::ProgressGuard { telemetry } => {
                if telemetry.triggered {
                    self.presentation.metadata.last_error = telemetry.diagnostic();
                }
            }
            LoopEvent::Usage {
                prompt_tokens,
                input_tokens,
                completion_tokens,
                reasoning_tokens,
                cache_creation_input_tokens,
                cache_read_input_tokens,
                total_tokens,
            } => {
                let usage = UsageSnapshot {
                    prompt_tokens,
                    input_tokens,
                    completion_tokens,
                    reasoning_tokens,
                    cache_creation_input_tokens,
                    cache_read_input_tokens,
                    total_tokens,
                };
                self.presentation.metadata.usage = Some(usage.clone());
                self.current_turn_mut().usage = Some(usage);
            }
            LoopEvent::SessionPinched {
                reason,
                source_session_id,
                new_session_id,
                estimated_tokens_before,
            } => {
                self.push_part(TimelinePart::Notice(NoticePart {
                    id: PartId::from_semantic(format!(
                        "continuation:{source_session_id}:{new_session_id}"
                    )),
                    message: format!(
                        "Continued from {source_session_id} to {new_session_id}: {reason}"
                    ),
                    level: NoticeLevel::Neutral,
                    expandable: Some(ArtifactModel {
                        content: ArtifactContent::Fields(vec![
                            crate::tui_v2::model::artifact::ArtifactField {
                                key: "source_session".to_owned(),
                                value: source_session_id,
                            },
                            crate::tui_v2::model::artifact::ArtifactField {
                                key: "new_session".to_owned(),
                                value: new_session_id,
                            },
                            crate::tui_v2::model::artifact::ArtifactField {
                                key: "estimated_tokens_before".to_owned(),
                                value: estimated_tokens_before.to_string(),
                            },
                        ]),
                        ..ArtifactModel::default()
                    }),
                }));
            }
            LoopEvent::ContextCompactionStarted { .. } => {}
            LoopEvent::ContextCompacted {
                reason,
                estimated_tokens_before,
                estimated_tokens_after,
                replaced_messages,
                checkpoint_id,
                ..
            } => {
                self.push_part(TimelinePart::Compaction(CompactionPart {
                    id: PartId::from_semantic(format!("compaction:{checkpoint_id}")),
                    reason,
                    estimated_tokens_before,
                    estimated_tokens_after: Some(estimated_tokens_after),
                    replaced_messages: Some(replaced_messages),
                    checkpoint_id: Some(checkpoint_id),
                    in_place: true,
                }));
            }
            LoopEvent::TitleGenerated { title } => {
                self.presentation.metadata.title = Some(title);
            }
            LoopEvent::Finished {
                session_id: _,
                stop_reason,
            } => self.finish(stop_reason),
            LoopEvent::Error { error } => {
                self.presentation.metadata.last_error = Some(error.clone());
                let ordinal = self.current_turn_mut().parts.len();
                self.push_part(TimelinePart::Error(ErrorPart {
                    id: PartId::from_semantic(format!("error:{ordinal}")),
                    message: error,
                    provider_request_failure: true,
                }));
            }
            LoopEvent::AgentBackgroundStarted {
                delegated_run_id,
                agent_type,
                description,
            } => {
                let key = format!("delegated:{delegated_run_id}");
                let tool = self.upsert_tool(&key, "agent", ToolStatus::Running, false);
                tool.arguments.fields = vec![crate::tui_v2::model::artifact::ArtifactField {
                    key: "agent_type".to_owned(),
                    value: agent_type,
                }];
                tool.artifact = ArtifactModel {
                    content: ArtifactContent::Text(BoundedText {
                        text: description,
                        omitted_bytes: 0,
                    }),
                    ..ArtifactModel::default()
                };
            }
            LoopEvent::AgentBackgroundCompleted {
                delegated_run_id,
                agent_type,
                success,
                summary,
            } => {
                let key = format!("delegated:{delegated_run_id}");
                let tool = self.upsert_tool(
                    &key,
                    "agent",
                    if success {
                        ToolStatus::Succeeded
                    } else {
                        ToolStatus::Failed
                    },
                    false,
                );
                tool.arguments.fields = vec![crate::tui_v2::model::artifact::ArtifactField {
                    key: "agent_type".to_owned(),
                    value: agent_type,
                }];
                tool.artifact = parse_tool_output("agent", &summary, false);
            }
            LoopEvent::UserMessage {
                title,
                message,
                level,
            } => {
                let ordinal = self.current_turn_mut().parts.len();
                self.push_part(TimelinePart::Notice(NoticePart {
                    id: PartId::from_semantic(format!("agent-message:{ordinal}")),
                    message: title.map_or(message.clone(), |title| format!("{title}: {message}")),
                    level: match level.as_str() {
                        "error" | "warning" => NoticeLevel::Warning,
                        "success" => NoticeLevel::Success,
                        _ => NoticeLevel::Neutral,
                    },
                    expandable: None,
                }));
            }
            LoopEvent::TeammateSpawned { name, role } => {
                self.upsert_tool(
                    &format!("teammate:{name}"),
                    "agent",
                    ToolStatus::Running,
                    false,
                )
                .artifact = parse_tool_output("agent", &format!("{name} — {role}"), false);
            }
            LoopEvent::TeammateTaskCompleted {
                name,
                task_id,
                result,
            } => {
                self.upsert_tool(
                    &format!("teammate:{name}"),
                    "agent",
                    ToolStatus::Succeeded,
                    false,
                )
                .artifact = parse_tool_output("agent", &format!("{task_id}: {result}"), false);
            }
            LoopEvent::TeammateTaskFailed {
                name,
                task_id,
                error,
            } => {
                self.upsert_tool(
                    &format!("teammate:{name}"),
                    "agent",
                    ToolStatus::Failed,
                    false,
                )
                .artifact = parse_tool_output("agent", &format!("{task_id}: {error}"), false);
            }
            LoopEvent::TeammateCancelled { name } => {
                self.upsert_tool(
                    &format!("teammate:{name}"),
                    "agent",
                    ToolStatus::Interrupted,
                    false,
                );
            }
        }
    }

    fn prepare_special_interaction(
        &mut self,
        tool_call_id: &str,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) {
        if tool_name.eq_ignore_ascii_case("askuserquestion") {
            self.set_pending_interaction(PendingInteraction::Questions(PendingQuestions {
                session_id: self.session_id.clone(),
                tool_call_id: tool_call_id.to_owned(),
                questions: parse_questions(arguments),
            }));
        }
    }

    fn clear_pending_interaction(&mut self, tool_call_id: &str) {
        self.presentation
            .pending_interactions
            .retain(|pending| pending.tool_call_id() != tool_call_id);
    }

    fn finish(&mut self, reason: LoopStopReason) {
        let (turn_state, tool_status) = match reason {
            LoopStopReason::Completed | LoopStopReason::Pinched => {
                (TurnState::Completed, ToolStatus::Interrupted)
            }
            LoopStopReason::AwaitingInput | LoopStopReason::Sleeping => {
                (TurnState::AwaitingInput, ToolStatus::AwaitingApproval)
            }
            LoopStopReason::UserAbort => (TurnState::Interrupted, ToolStatus::Interrupted),
            LoopStopReason::BudgetExhausted
            | LoopStopReason::ProviderError
            | LoopStopReason::LoopGuardTriggered
            | LoopStopReason::StreamIdleTimeout
            | LoopStopReason::PinchFailed => (TurnState::Failed, ToolStatus::Failed),
        };
        self.settle_streaming_parts(tool_status);
        self.current_turn_mut().state = turn_state;
        self.presentation.metadata.stop_reason = Some(format!("{reason:?}"));
        self.presentation.live_turn_id = None;
    }
}

fn map_citation(citation: Citation) -> CitationModel {
    CitationModel {
        url: citation.url,
        title: citation.title,
        cited_text: citation.cited_text,
    }
}

fn parse_questions(arguments: &serde_json::Value) -> Vec<QuestionModel> {
    arguments
        .get("questions")
        .and_then(serde_json::Value::as_array)
        .map(|questions| {
            questions
                .iter()
                .filter_map(|question| {
                    Some(QuestionModel {
                        header: question.get("header")?.as_str()?.to_owned(),
                        question: question.get("question")?.as_str()?.to_owned(),
                        options: question
                            .get("options")
                            .and_then(serde_json::Value::as_array)
                            .map(|options| {
                                options
                                    .iter()
                                    .filter_map(|option| {
                                        Some(QuestionOptionModel {
                                            label: option.get("label")?.as_str()?.to_owned(),
                                            description: option
                                                .get("description")
                                                .and_then(serde_json::Value::as_str)
                                                .map(ToOwned::to_owned),
                                        })
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        multi_select: question
                            .get("multi_select")
                            .or_else(|| question.get("multiSelect"))
                            .and_then(serde_json::Value::as_bool)
                            .unwrap_or(false),
                    })
                })
                .collect()
        })
        .filter(|questions: &Vec<_>| !questions.is_empty())
        .unwrap_or_else(|| {
            vec![QuestionModel {
                header: "Question".to_owned(),
                question: "Mitsuro is waiting for your input.".to_owned(),
                options: Vec::new(),
                multi_select: false,
            }]
        })
}
