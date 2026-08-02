import type {
	PlanItem,
	SessionContinuationEvent,
	StreamCallbacks,
} from "@krusty/api";
import {
	MAX_LIVE_MESSAGE_CONTENT_LENGTH,
	MAX_LIVE_THINKING_CONTENT_LENGTH,
	MAX_LIVE_TOOL_OUTPUT_LENGTH,
} from "./constants";
import type { createPlanStore } from "../plan";
import type { createSessionsStore } from "../sessions";
import { beginKrustyPerformanceSpan } from "../performance";
import {
	applyDelegatedProgress,
	createDelegatedArtifactState,
	formatToolOutputForDisplay,
	mergeDelegatedArtifactState,
	parseDelegatedArtifactState,
	resolveDelegatedKind,
} from "./delegated";
import {
	createChatMessageId,
	createStreamingAssistantMessage,
	finalizeTransientAssistantMessages,
	pruneEmptyAssistantMessages,
	upsertTransientAssistantMessage,
} from "./transient";
import type {
	AssistantMessageRef,
	ChatRenderPart,
	SessionMode,
	SessionStoreState,
	ToolCall,
} from "./types";

type SessionStateSetter = (
	partial:
		| Partial<SessionStoreState>
		| ((state: SessionStoreState) => Partial<SessionStoreState>),
) => void;

interface StreamCallbackDependencies {
	planStore: ReturnType<typeof createPlanStore>;
	sessionsStore: ReturnType<typeof createSessionsStore>;
	persistSessionMode: (
		getState: () => SessionStoreState,
		mode: SessionMode,
	) => Promise<void>;
	isActive?: () => boolean;
	onFirstEvent?: () => void;
}

function appendBounded(existing: string, delta: string, max: number): string {
	const next = existing + delta;
	if (next.length <= max) return next;
	return next.slice(next.length - max);
}

function appendRenderPart(ref: AssistantMessageRef, part: ChatRenderPart) {
	ref.current.renderParts = [...(ref.current.renderParts || []), part];
}

function appendTextRenderPart(ref: AssistantMessageRef, content: string) {
	if (!content) return;

	const parts = [...(ref.current.renderParts || [])];
	const lastPart = parts[parts.length - 1];
	if (lastPart?.type === "text") {
		parts[parts.length - 1] = {
			...lastPart,
			content: lastPart.content + content,
		};
	} else {
		const textCount = parts.filter((part) => part.type === "text").length;
		parts.push({
			type: "text",
			id: `text-${textCount}`,
			content,
		});
	}
	ref.current.renderParts = parts;
}

function appendThinkingRenderPart(ref: AssistantMessageRef, content: string) {
	if (!content) return;

	const parts = [...(ref.current.renderParts || [])];
	const lastPart = parts[parts.length - 1];
	if (lastPart?.type === "thinking") {
		parts[parts.length - 1] = {
			...lastPart,
			content: lastPart.content + content,
		};
	} else {
		parts.push({
			type: "thinking",
			id: `thinking-${parts.length}`,
			content,
		});
	}
	ref.current.renderParts = parts;
}

function appendToolRenderPart(ref: AssistantMessageRef, toolCallId: string) {
	if (!toolCallId) return;

	const parts = ref.current.renderParts || [];
	if (
		parts.some(
			(part) => part.type === "tool" && part.toolCallId === toolCallId,
		)
	) {
		return;
	}

	appendRenderPart(ref, {
		type: "tool",
		id: `tool-${toolCallId}`,
		toolCallId,
	});
}

export function createStreamCallbacks(
	ref: AssistantMessageRef,
	set: SessionStateSetter,
	get: () => SessionStoreState,
	{
		planStore,
		sessionsStore,
		persistSessionMode,
		isActive = () => true,
		onFirstEvent,
	}: StreamCallbackDependencies,
): StreamCallbacks {
	let pinchedSessionId: string | null = null;
	let compactedInPlace = false;
	let streamLagged = false;
	let pendingTextDelta = "";
	let pendingThinkingDelta = "";
	const pendingToolOutputDeltas = new Map<string, string>();
	let streamFlushScheduled = false;
	let firstEventPending = true;
	const finishFirstEventSpan = beginKrustyPerformanceSpan("stream.first_event");

	function noteFirstEvent() {
		if (!firstEventPending) return;
		firstEventPending = false;
		finishFirstEventSpan();
		onFirstEvent?.();
	}

	function updateLastAssistantMessage(
		updater?: (state: SessionStoreState) => Partial<SessionStoreState>,
	) {
		set((state) => {
			const messages = upsertTransientAssistantMessage(state.messages, {
				...ref.current,
			});
			return { messages, ...updater?.(state) };
		});
	}

	function flushPendingDeltas() {
		if (!isActive()) {
			pendingTextDelta = "";
			pendingThinkingDelta = "";
			pendingToolOutputDeltas.clear();
			return;
		}
		const finishFlushSpan = beginKrustyPerformanceSpan("stream.flush");
		let changed = false;
		let flushedText = false;
		let flushedThinking = false;

		if (pendingTextDelta) {
			ref.current.content = appendBounded(
				ref.current.content,
				pendingTextDelta,
				MAX_LIVE_MESSAGE_CONTENT_LENGTH,
			);
			appendTextRenderPart(ref, pendingTextDelta);
			pendingTextDelta = "";
			changed = true;
			flushedText = true;
		}

		if (pendingThinkingDelta) {
			ref.current.thinking = appendBounded(
				ref.current.thinking || "",
				pendingThinkingDelta,
				MAX_LIVE_THINKING_CONTENT_LENGTH,
			);
			appendThinkingRenderPart(ref, pendingThinkingDelta);
			pendingThinkingDelta = "";
			const delegatedIndex = (ref.current.toolCalls || []).findIndex(
				(toolCall) =>
					resolveDelegatedKind(
						toolCall.name,
						toolCall.arguments,
						toolCall.delegated?.kind,
					) !== undefined,
			);
			if (delegatedIndex >= 0) {
				const toolCalls = [...(ref.current.toolCalls || [])];
				const delegatedTool = toolCalls[delegatedIndex];
				const delegatedKind = resolveDelegatedKind(
					delegatedTool.name,
					delegatedTool.arguments,
					delegatedTool.delegated?.kind,
				);
				if (delegatedKind) {
					toolCalls[delegatedIndex] = {
						...delegatedTool,
						delegated: mergeDelegatedArtifactState(delegatedTool.delegated, {
							...(delegatedTool.delegated ||
								createDelegatedArtifactState(
									delegatedKind,
									delegatedTool.arguments,
								)),
							kind: delegatedKind,
							thinking: ref.current.thinking || "",
						}),
					};
					ref.current.toolCalls = toolCalls;
				}
			}
			changed = true;
			flushedThinking = true;
		}

		if (pendingToolOutputDeltas.size > 0) {
			let toolCallsChanged = false;
			if (ref.current.toolCalls?.length) {
				ref.current.toolCalls = ref.current.toolCalls.map((toolCall) => {
					const delta = pendingToolOutputDeltas.get(toolCall.id);
					if (!delta) return toolCall;
					toolCallsChanged = true;
					return {
						...toolCall,
						output: appendBounded(toolCall.output || "", delta, MAX_LIVE_TOOL_OUTPUT_LENGTH),
					};
				});
			}
			pendingToolOutputDeltas.clear();
			changed = changed || toolCallsChanged;
		}

		if (!changed) {
			finishFlushSpan();
			return;
		}
		updateLastAssistantMessage(() => ({
			...(flushedText
				? { isLoading: false, isThinking: false }
				: {}),
			...(flushedThinking
				? {
						isThinking: true,
						thinkingContent: ref.current.thinking || "",
					}
				: {}),
		}));
		finishFlushSpan();
	}

	function scheduleStreamFlush() {
		if (streamFlushScheduled) return;
		streamFlushScheduled = true;
		type FrameScheduler = (callback: (timestamp: number) => void) => unknown;
		const runtime = globalThis as typeof globalThis & {
			requestAnimationFrame?: FrameScheduler;
		};
		const schedule: FrameScheduler = runtime.requestAnimationFrame
			? runtime.requestAnimationFrame.bind(runtime)
			: (callback) => setTimeout(() => callback(Date.now()), 16);
		schedule(() => {
			streamFlushScheduled = false;
			if (!isActive()) {
				pendingTextDelta = "";
				pendingThinkingDelta = "";
				pendingToolOutputDeltas.clear();
				return;
			}
			flushPendingDeltas();
		});
	}

	function mapToolCalls(id: string, mapper: (toolCall: ToolCall) => ToolCall) {
		const toolCalls = ref.current.toolCalls;
		if (!toolCalls || toolCalls.length === 0) return;

		const index = toolCalls.findIndex((toolCall) => toolCall.id === id);
		if (index < 0) return;

		const nextToolCalls = [...toolCalls];
		nextToolCalls[index] = mapper(nextToolCalls[index]);
		ref.current.toolCalls = nextToolCalls;
		updateLastAssistantMessage();
	}

	return {
		onTextDelta: (delta) => {
			noteFirstEvent();
			if (pendingThinkingDelta || pendingToolOutputDeltas.size > 0) {
				flushPendingDeltas();
			}
			pendingTextDelta += delta;
			scheduleStreamFlush();
		},

		onThinkingDelta: (thinking) => {
			noteFirstEvent();
			if (pendingTextDelta || pendingToolOutputDeltas.size > 0) {
				flushPendingDeltas();
			}
			pendingThinkingDelta += thinking;
			scheduleStreamFlush();
		},

		onToolCallStart: (id, name) => {
			noteFirstEvent();
			flushPendingDeltas();
			if ((ref.current.toolCalls || []).some((toolCall) => toolCall.id === id)) {
				appendToolRenderPart(ref, id);
				return;
			}
			const delegatedKind = resolveDelegatedKind(name);
			ref.current.toolCalls = [
				...(ref.current.toolCalls || []),
				{
					id,
					name,
					delegated: delegatedKind
						? createDelegatedArtifactState(delegatedKind)
						: undefined,
					status: "running",
				},
			];
			appendToolRenderPart(ref, id);
			updateLastAssistantMessage();
		},

		onToolCallComplete: (id, _name, args) => {
			flushPendingDeltas();
			mapToolCalls(id, (toolCall) => {
				const delegatedKind = resolveDelegatedKind(
					toolCall.name,
					args,
					toolCall.delegated?.kind,
				);
				return {
					...toolCall,
					arguments: args,
					delegated: delegatedKind
						? mergeDelegatedArtifactState(
								toolCall.delegated,
								createDelegatedArtifactState(delegatedKind, args),
							)
						: toolCall.delegated,
				};
			});
		},

		onToolResult: (id, output, isError) => {
			flushPendingDeltas();
			mapToolCalls(id, (toolCall) => {
				const delegatedKind = resolveDelegatedKind(
					toolCall.name,
					toolCall.arguments,
					toolCall.delegated?.kind,
				);
				const delegated = delegatedKind
					? mergeDelegatedArtifactState(
							toolCall.delegated,
							parseDelegatedArtifactState(
								toolCall.name,
								output,
								toolCall.arguments,
								delegatedKind,
							) ||
								createDelegatedArtifactState(delegatedKind, toolCall.arguments),
						)
					: toolCall.delegated;
				const status: ToolCall["status"] =
					delegated?.outcome === "partial"
						? "partial"
						: delegated?.outcome === "failed" || delegated?.outcome === "cancelled"
							? "error"
							: isError
								? "error"
								: "success";
				return {
					...toolCall,
					output: formatToolOutputForDisplay(
						toolCall.name,
						output,
						toolCall.arguments,
					),
					delegated,
					status,
				};
			});
		},

		onToolOutputDelta: (id, delta) => {
			noteFirstEvent();
			if (pendingTextDelta || pendingThinkingDelta) {
				flushPendingDeltas();
			}
			pendingToolOutputDeltas.set(
				id,
				(pendingToolOutputDeltas.get(id) || "") + delta,
			);
			scheduleStreamFlush();
		},

		onDelegatedProgress: (event) => {
			flushPendingDeltas();
			mapToolCalls(event.tool_call_id, (toolCall) =>
				applyDelegatedProgress(toolCall, event),
			);
		},

		onPlanUpdate: (items: PlanItem[]) => {
			flushPendingDeltas();
			planStore.getState().setItems(items);
		},

		onWorkflowUpdated: (_goalId, aggregateRevision) => {
			flushPendingDeltas();
			planStore.getState().noteWorkflowRevision(aggregateRevision);
		},

		onModeChange: (mode) => {
			flushPendingDeltas();
			const nextMode: SessionMode = mode === "plan" ? "plan" : "build";
			set({ mode: nextMode });
			planStore.getState().setVisible(nextMode === "plan");
			void persistSessionMode(get, nextMode);
		},

		onPlanComplete: (toolCallId, title, taskCount) => {
			flushPendingDeltas();
			const planConfirmCall: ToolCall = {
				id: toolCallId,
				name: "PlanConfirm",
				arguments: { title, task_count: taskCount },
				status: "pending",
			};
			ref.current.toolCalls = [
				...(ref.current.toolCalls || []),
				planConfirmCall,
			];
			appendToolRenderPart(ref, toolCallId);
			updateLastAssistantMessage();
		},

		onTurnComplete: (_turn, hasMore) => {
			flushPendingDeltas();
			if (hasMore) {
				const completed = ref.current;
				const hasRenderableContent = Boolean(
					completed.content.trim()
						|| completed.thinking?.trim()
						|| (completed.toolCalls?.length ?? 0) > 0,
				);
				if (!hasRenderableContent) return;

				set((state) => ({
					messages: finalizeTransientAssistantMessages(state.messages),
				}));
				ref.current = createStreamingAssistantMessage();
			}
		},

		onToolApprovalRequired: (id, _name, args) => {
			flushPendingDeltas();
			mapToolCalls(id, (toolCall) => ({
				...toolCall,
				arguments: args,
				status: "awaiting_approval",
			}));
		},

		onToolApproved: (id) => {
			flushPendingDeltas();
			mapToolCalls(id, (toolCall) => ({ ...toolCall, status: "running" }));
		},

		onToolDenied: (id) => {
			flushPendingDeltas();
			mapToolCalls(id, (toolCall) => ({
				...toolCall,
				status: "error",
				output: "Denied by user",
			}));
		},

		onSteeringInjected: (pendingId, message) => {
			flushPendingDeltas();
			const id = pendingId
				? `user-steering-${pendingId}`
				: createChatMessageId("user-steering");
			set((state) => {
				if (state.messages.some((candidate) => candidate.id === id)) {
					return {
						messages: state.messages.map((candidate) =>
							candidate.id === id
								? {
									...candidate,
									isQueued: false,
									queuedUntilNextRun: false,
								}
								: candidate,
						),
					};
				}
				return {
					messages: [
						...pruneEmptyAssistantMessages(
							finalizeTransientAssistantMessages(state.messages),
						),
						{ id, role: "user" as const, content: message },
					],
				};
			});
		},

		onUsage: (promptTokens, completionTokens, metrics) => {
			flushPendingDeltas();
			set({
				tokenCount:
					metrics?.totalTokens ?? promptTokens + completionTokens,
				tokenUsage: metrics ?? null,
			});
		},

		onLagged: () => {
			streamLagged = true;
		},

		onSessionPinched: (event: SessionContinuationEvent) => {
			flushPendingDeltas();
			if (event.type === "session_pinched") {
				pinchedSessionId = event.new_session_id;
				return;
			}

			if (event.type === "context_compacted") {
				compactedInPlace = true;
				set({
					tokenCount: event.estimated_tokens_after,
					tokenUsage: null,
				});
			}
		},

		onTitleUpdate: (title) => {
			flushPendingDeltas();
			set({ title });
			sessionsStore.getState().loadSessions();
		},

		onFinish: (sessionId) => {
			flushPendingDeltas();
			const currentState = get();
			const queued = currentState.queuedMessages;
			const activeSessionId = pinchedSessionId ?? sessionId;
			const shouldLoadPinchedSession =
				pinchedSessionId !== null && pinchedSessionId !== sessionId;
			const shouldReloadCurrentSession =
				(compactedInPlace || streamLagged) && !shouldLoadPinchedSession;

			const messages = finalizeTransientAssistantMessages(
				currentState.messages.map((message) =>
					message.isQueued && message.id.startsWith("user-steering-")
						? { ...message, queuedUntilNextRun: true }
						: message.isQueued && !message.queuedUntilNextRun
							? { ...message, isQueued: false }
							: message,
				),
			);

			set({
				sessionId: activeSessionId,
				messages: pruneEmptyAssistantMessages(messages),
				queuedMessages: [],
				isStreaming: false,
				isThinking: false,
				thinkingContent: "",
			});
			sessionsStore.getState().loadSessions();

			if (shouldLoadPinchedSession) {
				const nextSessionId = pinchedSessionId;
				pinchedSessionId = null;
				if (nextSessionId) {
					void (async () => {
						try {
							await get().loadSession(nextSessionId, true);
						} catch {
							// loadSession already updates error state
						}

						if (queued.length > 0) {
							const combinedContent = queued
								.map((message) => message.content)
								.join("\n\n");
							const combinedAttachments = queued.flatMap(
								(message) => message.attachments,
							);
								void get().sendMessage(
									combinedContent,
									combinedAttachments,
								);
						}
					})();
				}
				return;
			}

			pinchedSessionId = null;

			if (shouldReloadCurrentSession) {
				compactedInPlace = false;
				streamLagged = false;
				void (async () => {
					try {
						await get().loadSession(sessionId, true);
					} catch {
						// loadSession already updates error state
					}

					if (queued.length > 0) {
						const combinedContent = queued
							.map((message) => message.content)
							.join("\n\n");
						const combinedAttachments = queued.flatMap(
							(message) => message.attachments,
						);
							void get().sendMessage(
								combinedContent,
								combinedAttachments,
							);
					}
				})();
				return;
			}

			compactedInPlace = false;
			streamLagged = false;

			if (queued.length > 0) {
				const combinedContent = queued
					.map((message) => message.content)
					.join("\n\n");
				const combinedAttachments = queued.flatMap(
					(message) => message.attachments,
				);
					setTimeout(
						() =>
							get().sendMessage(
								combinedContent,
								combinedAttachments,
							),
					50,
				);
			}
		},

		onError: (error) => {
			// Flush frame-batched stream content so the final update is not dropped.
			flushPendingDeltas();
			set((state) => ({
				isLoading: false,
				isStreaming: false,
				isThinking: false,
				thinkingContent: "",
				messages: pruneEmptyAssistantMessages(
					finalizeTransientAssistantMessages(state.messages),
				),
				error,
			}));
		},
	};
}
