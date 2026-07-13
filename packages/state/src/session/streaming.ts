import type {
	PlanItem,
	SessionContinuationEvent,
	StreamCallbacks,
} from "@krusty/api";
import type { createPlanStore } from "../plan";
import type { createSessionsStore } from "../sessions";
import {
	applyDelegatedProgress,
	createDelegatedArtifactState,
	formatToolOutputForDisplay,
	mergeDelegatedArtifactState,
	parseDelegatedArtifactState,
	resolveDelegatedKind,
} from "./delegated";
import {
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
		parts.push({
			type: "text",
			id: `text-${parts.length}`,
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
	{ planStore, sessionsStore, persistSessionMode }: StreamCallbackDependencies,
): StreamCallbacks {
	let pinchedSessionId: string | null = null;
	let compactedInPlace = false;
	let pendingTextDelta = "";
	let textFlushScheduled = false;

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

	function flushPendingTextDelta() {
		if (!pendingTextDelta) return;
		ref.current.content += pendingTextDelta;
		appendTextRenderPart(ref, pendingTextDelta);
		pendingTextDelta = "";
		updateLastAssistantMessage(() => ({
			isLoading: false,
			isThinking: false,
		}));
	}

	function scheduleTextFlush() {
		if (textFlushScheduled) return;
		textFlushScheduled = true;
		const schedule =
			typeof globalThis.requestAnimationFrame === "function"
				? globalThis.requestAnimationFrame
				: (callback: FrameRequestCallback) => setTimeout(callback, 16);
		schedule(() => {
			textFlushScheduled = false;
			flushPendingTextDelta();
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
			pendingTextDelta += delta;
			scheduleTextFlush();
		},

		onThinkingDelta: (thinking) => {
			flushPendingTextDelta();
			ref.current.thinking = (ref.current.thinking || "") + thinking;
			appendThinkingRenderPart(ref, thinking);
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
				if (!delegatedKind) {
					updateLastAssistantMessage(() => ({
						isThinking: true,
						thinkingContent: ref.current.thinking || "",
					}));
					return;
				}
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
			updateLastAssistantMessage(() => ({
				isThinking: true,
				thinkingContent: ref.current.thinking || "",
			}));
		},

		onToolCallStart: (id, name) => {
			flushPendingTextDelta();
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
			flushPendingTextDelta();
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
			flushPendingTextDelta();
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
						: delegated?.outcome === "failed"
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
			flushPendingTextDelta();
			mapToolCalls(id, (toolCall) => ({
				...toolCall,
				output: (toolCall.output || "") + delta,
			}));
		},

		onDelegatedProgress: (event) => {
			flushPendingTextDelta();
			mapToolCalls(event.tool_call_id, (toolCall) =>
				applyDelegatedProgress(toolCall, event),
			);
		},

		onPlanUpdate: (items: PlanItem[]) => {
			flushPendingTextDelta();
			planStore.getState().setItems(items);
		},

		onModeChange: (mode) => {
			flushPendingTextDelta();
			const nextMode: SessionMode = mode === "plan" ? "plan" : "build";
			set({ mode: nextMode });
			planStore.getState().setVisible(nextMode === "plan");
			void persistSessionMode(get, nextMode);
		},

		onPlanComplete: (toolCallId, title, taskCount) => {
			flushPendingTextDelta();
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
			flushPendingTextDelta();
			if (hasMore) {
				updateLastAssistantMessage();
			}
		},

		onToolApprovalRequired: (id, _name, args) => {
			flushPendingTextDelta();
			mapToolCalls(id, (toolCall) => ({
				...toolCall,
				arguments: args,
				status: "awaiting_approval",
			}));
		},

		onToolApproved: (id) => {
			flushPendingTextDelta();
			mapToolCalls(id, (toolCall) => ({ ...toolCall, status: "running" }));
		},

		onToolDenied: (id) => {
			flushPendingTextDelta();
			mapToolCalls(id, (toolCall) => ({
				...toolCall,
				status: "error",
				output: "Denied by user",
			}));
		},

		onUsage: (promptTokens, completionTokens, metrics) => {
			flushPendingTextDelta();
			set({
				tokenCount:
					metrics?.totalTokens ?? promptTokens + completionTokens,
			});
		},

		onSessionPinched: (event: SessionContinuationEvent) => {
			flushPendingTextDelta();
			if (event.type === "session_pinched") {
				pinchedSessionId = event.new_session_id;
				return;
			}

			if (event.type === "context_compacted") {
				compactedInPlace = true;
				set({
					tokenCount: event.estimated_tokens_after,
				});
			}
		},

		onTitleUpdate: (title) => {
			flushPendingTextDelta();
			set({ title });
			sessionsStore.getState().loadSessions();
		},

		onFinish: (sessionId) => {
			flushPendingTextDelta();
			const currentState = get();
			const queued = currentState.queuedMessages;
			const activeSessionId = pinchedSessionId ?? sessionId;
			const shouldLoadPinchedSession =
				pinchedSessionId !== null && pinchedSessionId !== sessionId;
			const shouldReloadCompactedSession =
				compactedInPlace && !shouldLoadPinchedSession;

			const messages = finalizeTransientAssistantMessages(
				currentState.messages.map((message) =>
					message.isQueued ? { ...message, isQueued: false } : message,
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
							const queuedResearchEnabled = queued.some(
								(message) => message.researchEnabled,
							);
							void get().sendMessage(
								combinedContent,
								combinedAttachments,
								queuedResearchEnabled,
							);
						}
					})();
				}
				return;
			}

			pinchedSessionId = null;

			if (shouldReloadCompactedSession) {
				compactedInPlace = false;
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
						const queuedResearchEnabled = queued.some(
							(message) => message.researchEnabled,
						);
						void get().sendMessage(
							combinedContent,
							combinedAttachments,
							queuedResearchEnabled,
						);
					}
				})();
				return;
			}

			compactedInPlace = false;

			if (queued.length > 0) {
				const combinedContent = queued
					.map((message) => message.content)
					.join("\n\n");
				const combinedAttachments = queued.flatMap(
					(message) => message.attachments,
				);
				const queuedResearchEnabled = queued.some(
					(message) => message.researchEnabled,
				);
				setTimeout(
					() =>
						get().sendMessage(
							combinedContent,
							combinedAttachments,
							queuedResearchEnabled,
						),
					50,
				);
			}
		},

		onError: (error) => {
			// Flush rAF-batched text so the last frame is not dropped on failure.
			flushPendingTextDelta();
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
