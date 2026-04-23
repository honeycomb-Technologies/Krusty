import { create } from 'zustand';
import type {
  KrustyClient,
  SessionStateResponse as ApiSessionStateResponse,
} from '@krusty/api';
import type { createPlanStore } from '../plan';
import type { createSessionsStore } from '../sessions';
import type { KrustyStorage } from '../storage';
import type { createWorkspaceStore } from '../workspace';

import {
  MAX_QUEUED_MESSAGES,
  PRESENCE_CLIENT_STORAGE_KEY,
  PRESENCE_HEARTBEAT_INTERVAL,
  STATE_POLL_INTERVAL,
} from './constants';
import {
  buildContentBlocks,
  getUnsupportedImageAttachment,
  processStoredMessages,
  unsupportedImageMimeTypeMessage,
} from './messages';
import {
  persistCurrentModel,
  persistSessionMode,
  persistSessionModel,
  syncSessionPresence,
} from './persistence';
import { applySessionSnapshot } from './serverState';
import { createStreamCallbacks } from './streaming';
import {
  applyLivePartialAssistant,
  applyRecoveryParity,
  createChatMessageId,
  finalizeTransientAssistantMessages,
  pruneEmptyAssistantMessages,
  toErrorMessage,
  upsertTransientAssistantMessage,
} from './transient';
import type {
  AssistantMessageRef,
  Attachment,
  PermissionMode,
  SessionMode,
  SessionStoreState,
  ThinkingLevel,
} from './types';
import {
  cycleThinkingLevel,
  isThinkingEnabled,
  thinkingLevelToApiValue,
} from './thinking';

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


  const persistMode = (getState: () => SessionStoreState, mode: SessionMode) =>
    persistSessionMode(client, sessionsStore, getState, mode);

  const persistModel = (
    getState: () => SessionStoreState,
    model: string | null,
  ) => persistSessionModel(client, sessionsStore, getState, model);

  const persistCurrentSelectedModel = (model: string | null) =>
    persistCurrentModel(client, model);

  const syncPresence = (
    sessionId: string,
    getState: () => SessionStoreState,
  ) => syncSessionPresence(client, sessionId, getPresenceClientId(), getState);

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
          createStreamCallbacks(ref, set, get, {
            planStore,
            sessionsStore,
            persistSessionMode: persistMode,
          }),
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

        applySessionSnapshot(sessionId, serverState, isRefresh, set, get, planStore);
        get().startPresenceHeartbeat(sessionId);
        if (sessionModel && sessionModel !== previousModel) {
          void persistCurrentSelectedModel(sessionModel);
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
      void persistMode(get, mode);
    },

    // -- setModel -----------------------------------------------------------

    setModel(model: string | null) {
      if (get().model === model) {
        return;
      }

      set({ model });
      void persistCurrentSelectedModel(model);
      void persistModel(get, model);
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
          createStreamCallbacks(ref, set, get, {
            planStore,
            sessionsStore,
            persistSessionMode: persistMode,
          }),
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
          applySessionSnapshot(sessionId, serverState, true, set, get, planStore);

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
      void syncPresence(sessionId, get);
      presenceHeartbeatInterval = setInterval(() => {
        void syncPresence(sessionId, get);
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
