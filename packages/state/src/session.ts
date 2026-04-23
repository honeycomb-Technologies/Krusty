import { create } from 'zustand';
import type {
  KrustyClient,
  PlanItem,
  SessionContinuationEvent,
  SessionStateResponse as ApiSessionStateResponse,
  StreamCallbacks,
} from '@krusty/api';
import type { KrustyStorage } from './storage';
import type { createWorkspaceStore } from './workspace';
import type { createSessionsStore } from './sessions';
import type { createPlanStore } from './plan';

import {
  MAX_QUEUED_MESSAGES,
  PRESENCE_CLIENT_STORAGE_KEY,
  PRESENCE_HEARTBEAT_INTERVAL,
  STATE_POLL_INTERVAL,
} from './session/constants';
import {
  applyDelegatedProgress,
  applyDelegatedSessionState,
  createDelegatedArtifactState,
  mergeDelegatedArtifactState,
  parseDelegatedArtifactState,
  resolveDelegatedKind,
} from './session/delegated';
import {
  buildContentBlocks,
  getUnsupportedImageAttachment,
  processStoredMessages,
  unsupportedImageMimeTypeMessage,
} from './session/messages';
import {
  applyLivePartialAssistant,
  applyRecoveryParity,
  createChatMessageId,
  finalizeTransientAssistantMessages,
  pruneEmptyAssistantMessages,
  toErrorMessage,
  upsertTransientAssistantMessage,
} from './session/transient';
import type {
  AssistantMessageRef,
  Attachment,
  PermissionMode,
  SessionMode,
  SessionStoreState,
  ThinkingLevel,
  ToolCall,
} from './session/types';
import {
  cycleThinkingLevel,
  isThinkingEnabled,
  thinkingLevelToApiValue,
} from './session/thinking';

export * from './session/types';
export {
  cycleThinkingLevel,
  isFastModeModel,
  isThinkingEnabled,
  supportsFastMode,
  thinkingLevelLabel,
  thinkingLevelToApiValue,
  toggleFastModeModel,
} from './session/thinking';

export function createSessionStore(
  client: KrustyClient,
  storage: KrustyStorage,
  workspace: ReturnType<typeof createWorkspaceStore>,
  sessionsStore: ReturnType<typeof createSessionsStore>,
  planStore: ReturnType<typeof createPlanStore>,
) {
  let statePollingInterval: ReturnType<typeof setInterval> | null = null;
  let presenceHeartbeatInterval: ReturnType<typeof setInterval> | null = null;
  let abortController: AbortController | null = null;
  let presenceClientId: string | null = null;

  function getPresenceClientId(): string | null {
    if (presenceClientId) return presenceClientId;
    try {
      const existing = storage.get(PRESENCE_CLIENT_STORAGE_KEY);
      if (existing) {
        presenceClientId = existing;
        return existing;
      }
      const generated = createChatMessageId("presence");
      storage.set(PRESENCE_CLIENT_STORAGE_KEY, generated);
      presenceClientId = generated;
      return generated;
    } catch {
      return null;
    }
  }

  function loadPermissionMode(): PermissionMode {
    try {
      const stored = storage.get("krusty-permission-mode");
      if (stored === "supervised" || stored === "autonomous") return stored;
    } catch {
      /* ignore */
    }
    return "supervised";
  }

  const initialState: Omit<
    SessionStoreState,
    | "sendMessage"
    | "loadSession"
    | "clearSession"
    | "initSession"
    | "setTitle"
    | "updateTitle"
    | "setMode"
    | "setModel"
    | "setThinkingLevel"
    | "toggleThinking"
    | "togglePermissionMode"
    | "submitToolResult"
    | "submitToolApproval"
    | "stopStreaming"
    | "startStatePolling"
    | "stopStatePolling"
    | "startPresenceHeartbeat"
    | "stopPresenceHeartbeat"
    | "cleanup"
  > = {
    sessionId: null,
    title: "New Chat",
    mode: "build",
    permissionMode: loadPermissionMode(),
    messages: [],
    queuedMessages: [],
    isLoading: false,
    isStreaming: false,
    isThinking: false,
    thinkingContent: "",
    thinkingEnabled: true,
    thinkingLevel: "medium",
    tokenCount: 0,
    lastEventSequence: null,
    error: null,
    model: null,
  };

  // -------------------------------------------------------------------------
  // Streaming callbacks factory
  // -------------------------------------------------------------------------

  function createStreamCallbacks(
    ref: AssistantMessageRef,
    set: (
      partial:
        | Partial<SessionStoreState>
        | ((s: SessionStoreState) => Partial<SessionStoreState>),
    ) => void,
    get: () => SessionStoreState,
  ): StreamCallbacks {
    let pinchedSessionId: string | null = null;

    function updateLastAssistantMessage(
      updater?: (s: SessionStoreState) => Partial<SessionStoreState>,
    ) {
      set((s) => {
        const messages = upsertTransientAssistantMessage(
          s.messages,
          { ...ref.current },
        );
        return { messages, ...updater?.(s) };
      });
    }

    function mapToolCalls(id: string, mapper: (tc: ToolCall) => ToolCall) {
      const toolCalls = ref.current.toolCalls;
      if (!toolCalls || toolCalls.length === 0) return;

      const index = toolCalls.findIndex((tc) => tc.id === id);
      if (index < 0) return;

      const nextToolCalls = [...toolCalls];
      nextToolCalls[index] = mapper(nextToolCalls[index]);
      ref.current.toolCalls = nextToolCalls;
      updateLastAssistantMessage();
    }

    return {
      onTextDelta: (delta) => {
        ref.current.content += delta;
        updateLastAssistantMessage(() => ({
          isLoading: false,
          isThinking: false,
        }));
      },

      onThinkingDelta: (thinking) => {
        ref.current.thinking = (ref.current.thinking || "") + thinking;
        const delegatedIndex = (ref.current.toolCalls || []).findIndex((tc) =>
          resolveDelegatedKind(tc.name, tc.arguments, tc.delegated?.kind)
            !== undefined,
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
        updateLastAssistantMessage();
      },

      onToolCallComplete: (id, _name, args) => {
        mapToolCalls(id, (tc) => {
          const delegatedKind = resolveDelegatedKind(
            tc.name,
            args,
            tc.delegated?.kind,
          );
          return {
            ...tc,
            arguments: args,
            delegated: delegatedKind
              ? mergeDelegatedArtifactState(
                  tc.delegated,
                  createDelegatedArtifactState(delegatedKind, args),
                )
              : tc.delegated,
          };
        });
      },

      onToolResult: (id, output, isError) => {
        mapToolCalls(id, (tc) => {
          const delegatedKind = resolveDelegatedKind(
            tc.name,
            tc.arguments,
            tc.delegated?.kind,
          );
          const delegated = delegatedKind
            ? mergeDelegatedArtifactState(
                tc.delegated,
                parseDelegatedArtifactState(
                  tc.name,
                  output,
                  tc.arguments,
                  delegatedKind,
                ) || createDelegatedArtifactState(delegatedKind, tc.arguments),
              )
            : tc.delegated;
          const status: ToolCall["status"] =
            delegated?.outcome === "partial"
              ? "partial"
              : delegated?.outcome === "failed"
                ? "error"
                : isError
                  ? "error"
                  : "success";
          return { ...tc, output, delegated, status };
        });
      },

      onToolOutputDelta: (id, delta) => {
        mapToolCalls(id, (tc) => ({
          ...tc,
          output: (tc.output || "") + delta,
        }));
      },

      onDelegatedProgress: (event) => {
        mapToolCalls(event.tool_call_id, (tc) =>
          applyDelegatedProgress(tc, event),
        );
      },

      onPlanUpdate: (items: PlanItem[]) => {
        planStore.getState().setItems(items);
      },

      onModeChange: (mode) => {
        const nextMode: SessionMode = mode === "plan" ? "plan" : "build";
        set({ mode: nextMode });
        planStore.getState().setVisible(nextMode === "plan");
        void persistSessionMode(get, nextMode);
      },

      onPlanComplete: (toolCallId, title, taskCount) => {
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
        updateLastAssistantMessage();
      },

      onTurnComplete: (_turn, hasMore) => {
        if (hasMore) {
          updateLastAssistantMessage();
        }
      },

      onToolApprovalRequired: (id, _name, args) => {
        mapToolCalls(id, (tc) => ({
          ...tc,
          arguments: args,
          status: "awaiting_approval",
        }));
      },

      onToolApproved: (id) => {
        mapToolCalls(id, (tc) => ({ ...tc, status: "running" }));
      },

      onToolDenied: (id) => {
        mapToolCalls(id, (tc) => ({
          ...tc,
          status: "error",
          output: "Denied by user",
        }));
      },

      onUsage: (promptTokens, completionTokens) => {
        set({ tokenCount: promptTokens + completionTokens });
      },

      onSessionPinched: (event: SessionContinuationEvent) => {
        if (event.type === "session_pinched") {
          pinchedSessionId = event.new_session_id;
        }
      },

      onTitleUpdate: (title) => {
        set({ title });
        sessionsStore.getState().loadSessions();
      },

      onFinish: (sessionId) => {
        const currentState = get();
        const queued = currentState.queuedMessages;
        const activeSessionId = pinchedSessionId ?? sessionId;
        const shouldLoadPinchedSession =
          pinchedSessionId !== null && pinchedSessionId !== sessionId;

        const messages = finalizeTransientAssistantMessages(
          currentState.messages.map((m) =>
            m.isQueued ? { ...m, isQueued: false } : m,
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
                const combinedContent = queued.map((q) => q.content).join("\n\n");
                const combinedAttachments = queued.flatMap((q) => q.attachments);
                const queuedResearchEnabled = queued.some((q) => q.researchEnabled);
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

        if (queued.length > 0) {
          const combinedContent = queued.map((q) => q.content).join("\n\n");
          const combinedAttachments = queued.flatMap((q) => q.attachments);
          const queuedResearchEnabled = queued.some((q) => q.researchEnabled);
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
        set((s) => ({
          isLoading: false,
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
          messages: pruneEmptyAssistantMessages(
            finalizeTransientAssistantMessages(s.messages),
          ),
          error,
        }));
      },
    };
  }

  // -------------------------------------------------------------------------
  // Presence helpers
  // -------------------------------------------------------------------------

  async function syncSessionPresence(
    sessionId: string,
    getState: () => SessionStoreState,
  ) {
    const clientId = getPresenceClientId();
    if (!clientId) return;

    const state = getState();
    try {
      await client.heartbeatSessionPresence(sessionId, {
        client_id: clientId,
        surface: "mobile",
        capability: "controller",
        last_event_sequence: state.lastEventSequence,
      });
    } catch {
      // Presence heartbeat failed silently
    }
  }

  // -------------------------------------------------------------------------
  // Persist helpers
  // -------------------------------------------------------------------------

  async function persistSessionMode(
    getState: () => SessionStoreState,
    mode: SessionMode,
  ) {
    const state = getState();
    if (!state.sessionId) return;
    try {
      await client.updateSession(state.sessionId, { mode });
      sessionsStore.getState().loadSessions();
    } catch {
      // Failed to persist
    }
  }

  async function persistSessionModel(
    getState: () => SessionStoreState,
    model: string,
  ) {
    const state = getState();
    if (!state.sessionId) return;
    try {
      await client.updateSession(state.sessionId, { model });
      sessionsStore.getState().loadSessions();
    } catch {
      // Failed to persist
    }
  }

  async function persistCurrentModel(model: string | null) {
    try {
      await client.setCurrentModel(model);
    } catch {
      // Failed to persist
    }
  }

  // -------------------------------------------------------------------------
  // Apply session snapshot from server state
  // -------------------------------------------------------------------------

  function applySessionSnapshot(
    sessionId: string,
    serverState: ApiSessionStateResponse | null,
    isRefresh: boolean,
    set: (
      partial:
        | Partial<SessionStoreState>
        | ((s: SessionStoreState) => Partial<SessionStoreState>),
    ) => void,
    get: () => SessionStoreState,
  ) {
    if (!serverState) return;

    const nextMode: SessionMode = serverState.mode ?? "build";
    set((s) => ({
      mode: nextMode,
      isStreaming:
        serverState.agent_state === "streaming" ||
        serverState.agent_state === "tool_executing",
      isThinking:
        serverState.agent_state === "streaming"
          ? Boolean(serverState.live_partial_assistant?.thinking?.trim()) ||
            s.isThinking
          : false,
      thinkingContent: serverState.live_partial_assistant?.thinking || "",
      lastEventSequence: serverState.last_event_sequence ?? null,
      messages: applyDelegatedSessionState(
        applyLivePartialAssistant(
          applyRecoveryParity(
            s.messages,
            serverState.recovery,
            serverState.agent_state,
          ),
          serverState.live_partial_assistant,
          serverState.agent_state,
        ),
        serverState.delegated_tools,
        serverState.recent_delegated_runs,
      ),
    }));
    planStore.getState().setVisible(nextMode === "plan");

    if (
      (serverState.agent_state === "streaming" ||
        serverState.agent_state === "tool_executing") &&
      !isRefresh
    ) {
      get().startStatePolling(sessionId);
    }
  }

  // -------------------------------------------------------------------------
  // Create the Zustand store
  // -------------------------------------------------------------------------

  return create<SessionStoreState>((set, get) => ({
    ...initialState,

    // -- sendMessage --------------------------------------------------------

    async sendMessage(
      content: string,
      attachments: Attachment[] = [],
      researchEnabled = false,
    ) {
      const state = get();
      const ws = workspace.getState();
      const normalizedContent = content.trim();
      const requestMessage =
        normalizedContent.length > 0
          ? normalizedContent
          : attachments.length > 0
            ? "Please review the attached content."
            : content;

      const unsupportedImage = getUnsupportedImageAttachment(attachments);
      if (unsupportedImage) {
        set({
          error: unsupportedImageMimeTypeMessage(unsupportedImage.mimeType),
        });
        return;
      }

      const attachmentLabel =
        attachments.length > 0
          ? `[Attachments: ${attachments.map((a) => a.name).join(", ")}]`
          : "";
      const displayContent = attachmentLabel
        ? normalizedContent.length > 0
          ? `${normalizedContent}\n\n${attachmentLabel}`
          : attachmentLabel
        : requestMessage;

      if (state.isStreaming) {
        set((s) => {
          if (s.queuedMessages.length >= MAX_QUEUED_MESSAGES) {
            return {
              error:
                "Message queue is full. Please wait for the current response to finish.",
            };
          }
          return {
            queuedMessages: [
              ...s.queuedMessages,
              { content, attachments, researchEnabled },
            ],
            messages: [
              ...s.messages,
              {
                id: createChatMessageId("user-queued"),
                role: "user",
                content: displayContent,
                isQueued: true,
              },
            ],
          };
        });
        return;
      }

      const ref: AssistantMessageRef = {
        current: {
          id: createChatMessageId("assistant-stream"),
          role: "assistant",
          content: "",
          thinking: "",
          toolCalls: [],
          kind: "streaming",
        },
      };

      set((s) => ({
        messages: [
          ...s.messages,
          {
            id: createChatMessageId("user"),
            role: "user",
            content: displayContent,
          },
          ref.current,
        ],
        isLoading: true,
        isStreaming: true,
        error: null,
      }));

      abortController = new AbortController();

      const pollingSessionId = state.sessionId;
      if (pollingSessionId) {
        get().startStatePolling(pollingSessionId);
      }

      try {
        const contentBlocks =
          attachments.length > 0
            ? buildContentBlocks(requestMessage, attachments)
            : undefined;

        await client.streamChat(
          {
            session_id: state.sessionId ?? undefined,
            message: requestMessage,
            content: contentBlocks,
            project_dir: state.sessionId ? undefined : ws.directory,
            working_dir: state.sessionId ? undefined : ws.directory,
            workspace_mode: state.sessionId ? undefined : ws.mode,
            research_enabled: researchEnabled || undefined,
            model: state.model ?? undefined,
            thinking_enabled: thinkingLevelToApiValue(state.thinkingLevel),
            permission_mode: state.permissionMode,
            mode: state.mode,
          },
          createStreamCallbacks(ref, set, get),
          abortController.signal,
        );
      } catch (err) {
        set((s) => ({
          isLoading: false,
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
          messages: pruneEmptyAssistantMessages(
            finalizeTransientAssistantMessages(s.messages),
          ),
          error: toErrorMessage(err),
        }));
      } finally {
        get().stopStatePolling();
      }
    },

    // -- loadSession --------------------------------------------------------

    async loadSession(sessionId: string, isRefresh = false) {
      set({ isLoading: true });

      try {
        const data = await client.getSession(sessionId);
        const processedMessages = processStoredMessages(data.messages);

        let serverState: ApiSessionStateResponse | null = null;
        try {
          serverState = await client.getSessionState(sessionId);
        } catch {
          // State endpoint may not exist
        }

        const mode = serverState?.mode ?? data.session.mode ?? "build";
        const previousModel = get().model;
        const sessionModel = data.session.model?.trim() || null;
        set((s) => ({
          ...s,
          sessionId: data.session.id,
          title: data.session.title || "Untitled",
          mode,
          model: sessionModel ?? s.model,
          tokenCount: data.session.token_count ?? 0,
          messages: applyLivePartialAssistant(
            applyRecoveryParity(
              processedMessages,
              serverState?.recovery,
              serverState?.agent_state ?? "idle",
            ),
            serverState?.live_partial_assistant,
            serverState?.agent_state ?? "idle",
          ),
          isLoading: false,
        }));
        planStore.getState().setVisible(mode === "plan");

        workspace
          .getState()
          .initFromSession(
            data.session.id,
            data.session.project_dir ?? data.session.working_dir ?? null,
            (data.session.workspace_mode ??
              ((data.session.project_dir ?? data.session.working_dir)
                ? "selected"
                : "neutral")) as "neutral" | "selected" | "created",
          );

        applySessionSnapshot(sessionId, serverState, isRefresh, set, get);
        get().startPresenceHeartbeat(sessionId);
        if (sessionModel && sessionModel !== previousModel) {
          void persistCurrentModel(sessionModel);
        }
      } catch (err) {
        set({
          isLoading: false,
          error: toErrorMessage(err, "Failed to load session"),
        });
      }
    },

    // -- clearSession -------------------------------------------------------

    clearSession() {
      const current = get();
      get().stopPresenceHeartbeat(current.sessionId);
      set({
        ...initialState,
        permissionMode: current.permissionMode,
        model: current.model,
        thinkingLevel: current.thinkingLevel,
        thinkingEnabled: current.thinkingEnabled,
      });
      workspace.getState().clear();
    },

    // -- initSession --------------------------------------------------------

    initSession(sessionId: string, title: string) {
      const current = get();
      get().stopPresenceHeartbeat(current.sessionId);
      set({
        ...initialState,
        permissionMode: current.permissionMode,
        model: current.model,
        thinkingLevel: current.thinkingLevel,
        thinkingEnabled: current.thinkingEnabled,
        sessionId,
        title,
      });
      get().startPresenceHeartbeat(sessionId);
    },

    // -- setTitle ------------------------------------------------------------

    setTitle(title: string) {
      set({ title });
    },

    // -- updateTitle --------------------------------------------------------

    async updateTitle(sessionId: string, title: string) {
      try {
        await client.updateSession(sessionId, { title });
        set({ title });
        sessionsStore.getState().loadSessions();
      } catch {
        // Failed to update title
      }
    },

    // -- setMode ------------------------------------------------------------

    setMode(mode: SessionMode) {
      set({ mode });
      planStore.getState().setVisible(mode === "plan");
      void persistSessionMode(get, mode);
    },

    // -- setModel -----------------------------------------------------------

    setModel(model: string | null) {
      if (get().model === model) {
        return;
      }

      set({ model });
      void persistCurrentModel(model);
      if (model) {
        void persistSessionModel(get, model);
      }
    },

    // -- setThinkingLevel ---------------------------------------------------

    setThinkingLevel(level: ThinkingLevel) {
      set({
        thinkingLevel: level,
        thinkingEnabled: isThinkingEnabled(level),
      });
    },

    // -- toggleThinking -----------------------------------------------------

    toggleThinking() {
      set((s) => {
        const newLevel = cycleThinkingLevel(s.thinkingLevel, s.model);
        return {
          thinkingEnabled: isThinkingEnabled(newLevel),
          thinkingLevel: newLevel,
        };
      });
    },

    // -- togglePermissionMode -----------------------------------------------

    togglePermissionMode() {
      set((s) => {
        const newMode: PermissionMode =
          s.permissionMode === "supervised" ? "autonomous" : "supervised";
        try {
          storage.set("krusty-permission-mode", newMode);
        } catch {
          /* ignore */
        }
        return { permissionMode: newMode };
      });
    },

    // -- submitToolResult ---------------------------------------------------

    async submitToolResult(toolCallId: string, result: string) {
      const state = get();
      if (!state.sessionId) {
        throw new Error("No active session");
      }

      set((s) => ({
        messages: s.messages.map((m) => ({
          ...m,
          toolCalls: m.toolCalls?.map((tc) =>
            tc.id === toolCallId
              ? { ...tc, output: result, status: "success" as const }
              : tc,
          ),
        })),
        isStreaming: true,
        isLoading: true,
      }));

      abortController = new AbortController();
      get().startStatePolling(state.sessionId);

      const ref: AssistantMessageRef = {
        current: {
          id: createChatMessageId("assistant-stream"),
          role: "assistant",
          content: "",
          thinking: "",
          toolCalls: [],
          kind: "streaming",
        },
      };

      set((s) => ({
        messages: upsertTransientAssistantMessage(s.messages, ref.current),
      }));

      try {
        await client.streamToolResult(
          {
            session_id: state.sessionId,
            tool_call_id: toolCallId,
            result,
          },
          createStreamCallbacks(ref, set, get),
          abortController.signal,
        );
      } catch (err) {
        set((s) => ({
          isLoading: false,
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
          messages: pruneEmptyAssistantMessages(
            finalizeTransientAssistantMessages(s.messages),
          ),
          error: toErrorMessage(err),
        }));
      } finally {
        get().stopStatePolling();
      }
    },

    // -- submitToolApproval -------------------------------------------------

    async submitToolApproval(toolCallId: string, approved: boolean) {
      const state = get();
      if (!state.sessionId) return;

      set((s) => ({
        messages: s.messages.map((m) => ({
          ...m,
          toolCalls: m.toolCalls?.map((tc) =>
            tc.id === toolCallId
              ? approved
                ? { ...tc, status: "running" as const, output: undefined }
                : {
                    ...tc,
                    status: "error" as const,
                    output: tc.output ?? "Denied by user",
                  }
              : tc,
          ),
        })),
      }));

      await client.submitToolApproval(state.sessionId, toolCallId, approved);
      set({ isStreaming: true, isLoading: true });
      get().startStatePolling(state.sessionId);
    },

    // -- stopStreaming ------------------------------------------------------

    stopStreaming() {
      abortController?.abort();
      get().stopStatePolling();
      set((s) => ({
        isLoading: false,
        isStreaming: false,
        isThinking: false,
        thinkingContent: "",
        messages: pruneEmptyAssistantMessages(
          finalizeTransientAssistantMessages(s.messages),
        ),
      }));
    },

    // -- state polling ------------------------------------------------------

    startStatePolling(sessionId: string) {
      get().stopStatePolling();

      statePollingInterval = setInterval(async () => {
        try {
          const serverState = await client.getSessionState(sessionId);
          applySessionSnapshot(sessionId, serverState, true, set, get);

          if (
            serverState.agent_state === "idle" ||
            serverState.agent_state === "awaiting_input"
          ) {
            get().stopStatePolling();
            set({ isStreaming: false, isThinking: false });
            await get().loadSession(sessionId, true);
          }
        } catch {
          get().stopStatePolling();
        }
      }, STATE_POLL_INTERVAL);
    },

    stopStatePolling() {
      if (statePollingInterval) {
        clearInterval(statePollingInterval);
        statePollingInterval = null;
      }
    },

    // -- presence heartbeat -------------------------------------------------

    startPresenceHeartbeat(sessionId: string) {
      get().stopPresenceHeartbeat();
      void syncSessionPresence(sessionId, get);
      presenceHeartbeatInterval = setInterval(() => {
        void syncSessionPresence(sessionId, get);
      }, PRESENCE_HEARTBEAT_INTERVAL);
    },

    stopPresenceHeartbeat(sessionId?: string | null) {
      if (presenceHeartbeatInterval) {
        clearInterval(presenceHeartbeatInterval);
        presenceHeartbeatInterval = null;
      }

      if (!sessionId) return;

      const clientId = getPresenceClientId();
      if (!clientId) return;

      void client.removeSessionPresence(sessionId, clientId).catch(() => {});
    },

    // -- cleanup ------------------------------------------------------------

    cleanup() {
      get().stopStatePolling();
      const state = get();
      get().stopPresenceHeartbeat(state.sessionId);
    },
  }));
}
