import { create } from 'zustand';
import { KrustyApiError } from '@krusty/api';
import type {
  KrustyClient,
  ModelInfo,
  ModelKey,
  SessionStateResponse as ApiSessionStateResponse,
  StreamCallbacks,
} from '@krusty/api';
import type { createPlanStore } from '../plan';
import type { createSessionsStore } from '../sessions';
import type { KrustyStorage } from '../storage';
import type { createWorkspaceStore } from '../workspace';

import {
  MAX_QUEUED_MESSAGES,
  PRESENCE_CLIENT_STORAGE_KEY,
  STATE_POLL_DEGRADED_AFTER,
  STATE_POLL_DEGRADED_MESSAGE,
  PRESENCE_HEARTBEAT_INTERVAL,
  STATE_POLL_INTERVAL,
  STATE_POLL_MAX_BACKOFF,
  STATE_POLL_MAX_FAILURES,
} from './constants';
import {
  buildContentBlocks,
  getUnsupportedImageAttachment,
  processStoredMessages,
  unsupportedImageMimeTypeMessage,
} from './messages';
import { modelKeysEqual } from './modelSelection';
import {
  persistCurrentModel,
  persistSessionMode,
  persistSessionModel,
  persistSessionPermissionMode,
  syncSessionPresence,
} from './persistence';
import {
  applySessionSnapshot,
  isActionableSessionAgentState,
  isActiveSessionAgentState,
  isTerminalSessionAgentState,
  pendingInteractionsFromSnapshot,
  sessionAgentErrorMessage,
  shouldStopSessionStatePolling,
} from './serverState';
import { createStreamCallbacks } from './streaming';
import {
  applyLivePartialAssistant,
  applyRecoveryParity,
  createChatMessageId,
  createStreamingAssistantMessage,
  finalizeTransientAssistantMessages,
  pruneEmptyAssistantMessages,
  toErrorMessage,
  upsertTransientAssistantMessage,
} from './transient';
import type {
  AssistantMessageRef,
  Attachment,
  ChatMessageAttachment,
  PermissionMode,
  SendMessageOptions,
  SessionMode,
  SessionStoreState,
  ThinkingLevel,
} from './types';
import {
  cycleThinkingLevel,
  isThinkingEnabled,
  normalizeThinkingLevel,
  supportsFastMode,
  thinkingLevelToApiValue,
} from './thinking';

function hasOwnProperty<T extends object>(value: T, key: PropertyKey): boolean {
  return Object.hasOwn(value, key);
}

function normalizeTargetBranch(targetBranch: string | null | undefined): string | null {
  const trimmed = targetBranch?.trim();
  return trimmed ? trimmed : null;
}

function normalizeDisplayTitle(title: string | null | undefined): string {
  const trimmed = title?.trim() ?? "";
  const placeholder = trimmed.toLowerCase();
  return placeholder === "new chat" || placeholder === "new session"
    ? ""
    : trimmed;
}

function buildDisplayAttachments(
  attachments: Attachment[],
): ChatMessageAttachment[] {
  return attachments
    .map((attachment) => ({
      type: attachment.type,
      name: attachment.name,
      mimeType: attachment.mimeType,
      uri: attachment.uri,
      base64: attachment.type === "image" ? attachment.base64 : undefined,
    }))
    .filter((attachment) =>
      attachment.type !== "image" || Boolean(attachment.uri || attachment.base64),
    );
}

export function createSessionStore(
  client: KrustyClient,
  storage: KrustyStorage,
  workspace: ReturnType<typeof createWorkspaceStore>,
  sessionsStore: ReturnType<typeof createSessionsStore>,
  planStore: ReturnType<typeof createPlanStore>,
) {
  let statePollingTimer: ReturnType<typeof setTimeout> | null = null;
  let statePollingGeneration = 0;
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
    return "autonomous";
  }


  const persistMode = (getState: () => SessionStoreState, mode: SessionMode) =>
    persistSessionMode(client, sessionsStore, getState, mode);

  const persistModel = (
    getState: () => SessionStoreState,
    model: string | null,
    modelKey?: ModelKey | null,
  ) => persistSessionModel(client, sessionsStore, getState, model, modelKey);

  const persistPermissionMode = (
    getState: () => SessionStoreState,
    permissionMode: PermissionMode,
  ) => persistSessionPermissionMode(client, sessionsStore, getState, permissionMode);

  const persistCurrentSelectedModel = (
    model: string | null,
    modelKey?: ModelKey | null,
  ) => persistCurrentModel(client, model, modelKey);

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
    | "executeWorkflowCommand"
    | "setModel"
    | "setThinkingLevel"
    | "toggleThinking"
    | "setFastModeEnabled"
    | "toggleFastMode"
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
    sessionType: null,
    title: "",
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
    fastModeEnabled: false,
    tokenCount: 0,
    tokenUsage: null,
    lastEventSequence: null,
    error: null,
    model: null,
    modelKey: null,
    modelProvider: null,
    modelInfo: null,
  };




  // -------------------------------------------------------------------------
  // Create the Zustand store
  // -------------------------------------------------------------------------

  return create<SessionStoreState>((set, get) => {
    function applyStreamFailure(err: unknown) {
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
    }

    async function recoverAfterStreamInterruption(
      sessionId: string,
    ): Promise<boolean> {
      try {
        const serverState = await client.getSessionState(sessionId);
        applySessionSnapshot(sessionId, serverState, true, set, get, planStore);

        if (isActiveSessionAgentState(serverState.agent_state)) {
          set({ isLoading: false, error: null });
          get().startStatePolling(sessionId);
          return true;
        }

        if (isActionableSessionAgentState(serverState.agent_state)) {
          get().stopStatePolling();
          set({
            isLoading: false,
            isStreaming: false,
            isThinking: false,
            thinkingContent: "",
            error: null,
          });
          await get().loadSession(sessionId, true);
          return true;
        }

        if (isTerminalSessionAgentState(serverState.agent_state)) {
          const terminalError = sessionAgentErrorMessage(serverState);
          get().stopStatePolling();
          set({
            isLoading: false,
            isStreaming: false,
            isThinking: false,
            thinkingContent: "",
            error: terminalError,
          });
          await get().loadSession(sessionId, true);
          if (terminalError) {
            set({ error: terminalError });
          }
          // An idle snapshot with no canonical error does not prove that a
          // response completed. Let the original stream error surface after
          // refreshing the transcript instead of silently swallowing a clean
          // EOF that arrived before `finish`.
          return terminalError !== null;
        }
      } catch {
        // The stream and snapshot endpoints can fail independently during a
        // reconnect. Keep the session protected from duplicate sends and let
        // the bounded polling policy recover canonical state.
        set({
          isLoading: false,
          isStreaming: true,
          error: STATE_POLL_DEGRADED_MESSAGE,
        });
        get().startStatePolling(sessionId);
        return true;
      }

      return false;
    }

    function createRecoveringStreamCallbacks(
      callbacks: StreamCallbacks,
      sessionId: string | null,
      recoveryRef: { promise: Promise<boolean> | null },
    ): StreamCallbacks {
      return {
        ...callbacks,
        onError: (error) => {
          if (!sessionId) {
            callbacks.onError(error);
            return;
          }

          if (!recoveryRef.promise) {
            recoveryRef.promise = recoverAfterStreamInterruption(sessionId).then(
              (recovered) => {
                if (!recovered) {
                  callbacks.onError(error);
                }
                return recovered;
              },
            );
          }
        },
      };
    }

    return {
      ...initialState,

    // -- sendMessage --------------------------------------------------------

    async sendMessage(
      content: string,
      attachments: Attachment[] = [],
      sendOptions: SendMessageOptions = {},
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
      const displayAttachments = buildDisplayAttachments(attachments);

      if (state.isStreaming) {
        const queueLocally = (messageId = createChatMessageId("user-queued")) => {
          set((s) => {
            if (s.queuedMessages.length >= MAX_QUEUED_MESSAGES) {
              return {
                error:
                  "Message queue is full. Please wait for the current response to finish.",
              };
            }
            const alreadyDisplayed = s.messages.some(
              (message) => message.id === messageId,
            );
            return {
              queuedMessages: [
                ...s.queuedMessages,
                { content, attachments, sendOptions },
              ],
              messages: alreadyDisplayed
                ? s.messages
                : [
                    ...s.messages,
                    {
                      id: messageId,
                      role: "user" as const,
                      content: displayContent,
                      attachments:
                        displayAttachments.length > 0
                          ? displayAttachments
                          : undefined,
                      isQueued: true,
                    },
                  ],
            };
          });
        };

        // Rich follow-ups can change the model contract, so they remain a
        // separate turn. Plain text can steer the active core loop without
        // waiting for it to finish first.
        if (!state.sessionId || attachments.length > 0) {
          queueLocally();
          return;
        }

        const optimisticId = createChatMessageId("user-steering-pending");
        set((s) => ({
          messages: [
            ...s.messages,
            {
              id: optimisticId,
              role: "user" as const,
              content: displayContent,
              isQueued: true,
            },
          ],
          error: null,
        }));

        try {
          const response = await client.steerSession({
            session_id: state.sessionId,
            message: requestMessage,
          });
          const durableId = `user-steering-${response.pending_id}`;
          set((s) => {
            const eventAlreadyRendered = s.messages.some(
              (message) => message.id === durableId,
            );
            return {
              messages: eventAlreadyRendered
                ? s.messages.filter((message) => message.id !== optimisticId)
                : s.messages.map((message) =>
                    message.id === optimisticId
                      ? {
                          ...message,
                          id: durableId,
                          isQueued: true,
                          queuedUntilNextRun:
                            response.status === "queued" || !s.isStreaming,
                        }
                      : message,
                  ),
            };
          });
        } catch (error) {
          const recoverableRace =
            error instanceof KrustyApiError
            && (error.status === 404 || error.status === 409);
          if (recoverableRace && get().isStreaming) {
            queueLocally(optimisticId);
            return;
          }
          if (recoverableRace) {
            set((s) => ({
              messages: s.messages.filter(
                (message) => message.id !== optimisticId,
              ),
            }));
            await get().sendMessage(
              content,
              attachments,
              sendOptions,
            );
            return;
          }
          set((s) => ({
            messages: s.messages.filter(
              (message) => message.id !== optimisticId,
            ),
            error: toErrorMessage(error),
          }));
        }
        return;
      }

      const ref: AssistantMessageRef = {
        current: createStreamingAssistantMessage(),
      };

      set((s) => ({
        messages: [
          ...s.messages,
          {
            id: createChatMessageId("user"),
            role: "user",
            content: displayContent,
            attachments:
              displayAttachments.length > 0 ? displayAttachments : undefined,
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

      const streamRecovery: { promise: Promise<boolean> | null } = { promise: null };
      const callbacks = createRecoveringStreamCallbacks(
        createStreamCallbacks(ref, set, get, {
          planStore,
          sessionsStore,
          persistSessionMode: persistMode,
        }),
        pollingSessionId,
        streamRecovery,
      );
      let keepStatePolling = false;

      try {
        const contentBlocks =
          attachments.length > 0
            ? buildContentBlocks(requestMessage, attachments)
            : undefined;
        const isNewSessionRequest = !state.sessionId;
        const sendOptionHasProjectDir = sendOptions
          ? hasOwnProperty(sendOptions, "projectDir")
          : false;
        const sendOptionHasWorkingDir = sendOptions
          ? hasOwnProperty(sendOptions, "workingDir")
          : false;
        const sendOptionHasWorkspaceMode = sendOptions
          ? hasOwnProperty(sendOptions, "workspaceMode")
          : false;
        const sendOptionHasSessionType = sendOptions
          ? hasOwnProperty(sendOptions, "sessionType")
          : false;
        const sendOptionHasTargetBranch = sendOptions
          ? hasOwnProperty(sendOptions, "targetBranch")
          : false;
        const requestedProjectDir = isNewSessionRequest
          ? sendOptionHasProjectDir
            ? sendOptions?.projectDir ?? null
            : ws.directory
          : undefined;
        const requestedWorkingDir = isNewSessionRequest
          ? sendOptionHasWorkingDir
            ? sendOptions?.workingDir ?? null
            : sendOptionHasProjectDir
              ? requestedProjectDir
              : ws.directory
          : undefined;
        const requestedWorkspaceMode = isNewSessionRequest
          ? sendOptionHasWorkspaceMode
            ? sendOptions?.workspaceMode
            : ws.mode
          : undefined;
        const requestedSessionType = isNewSessionRequest
          ? sendOptionHasSessionType
            ? sendOptions?.sessionType
            : undefined
          : undefined;
        const effectiveSessionType = state.sessionType ?? requestedSessionType ?? "code";
        const requestedTargetBranch = isNewSessionRequest
          ? normalizeTargetBranch(
              sendOptionHasTargetBranch
                ? sendOptions?.targetBranch
                : ws.targetBranch,
            )
          : undefined;

        await client.streamChat(
          {
            session_id: state.sessionId ?? undefined,
            message: requestMessage,
            content: contentBlocks,
            project_dir: requestedProjectDir,
            working_dir: requestedWorkingDir,
            workspace_mode: requestedWorkspaceMode,
            session_type: requestedSessionType,
            target_branch:
              sendOptionHasTargetBranch || requestedTargetBranch
                ? requestedTargetBranch
                : undefined,
            model: effectiveSessionType === "mako" ? undefined : state.model ?? undefined,
            model_key:
              effectiveSessionType === "mako" ? undefined : state.modelKey ?? undefined,
            fast_mode: state.fastModeEnabled || undefined,
            thinking_enabled: thinkingLevelToApiValue(state.thinkingLevel),
            permission_mode:
              effectiveSessionType === "mako" ? undefined : state.permissionMode,
            mode: effectiveSessionType === "code" ? state.mode : undefined,
          },
          callbacks,
          abortController.signal,
        );

        const completedSessionId = get().sessionId;
        if (isNewSessionRequest && completedSessionId) {
          const nextDirectory = requestedProjectDir ?? requestedWorkingDir ?? null;
          workspace.getState().setWorkspace(
            nextDirectory,
            completedSessionId,
            requestedWorkspaceMode ?? (nextDirectory ? "selected" : "neutral"),
            requestedTargetBranch ?? null,
          );
        }
      } catch (err) {
        if (pollingSessionId) {
          streamRecovery.promise ??=
            recoverAfterStreamInterruption(pollingSessionId);
          keepStatePolling = await streamRecovery.promise;
        }
        if (!keepStatePolling) {
          applyStreamFailure(err);
        }
      } finally {
        if (streamRecovery.promise) {
          keepStatePolling = await streamRecovery.promise;
        }
        if (!keepStatePolling) {
          get().stopStatePolling();
        }
      }
    },

    // -- loadSession --------------------------------------------------------

    async loadSession(sessionId: string, isRefresh = false) {
      const previousSessionId = get().sessionId;
      if (previousSessionId !== sessionId) {
        planStore.getState().setWorkflow(null);
      }
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
        const permissionMode =
          serverState?.permission_mode ?? data.session.permission_mode ?? "autonomous";
        const previousModel = get().model;
        const previousModelKey = get().modelKey;
        const sessionModel = data.session.model?.trim() || null;
        const sessionModelKey = data.session.model_key ?? null;
        set((s) => {
          const sameExactSelection = Boolean(sessionModelKey)
            && modelKeysEqual(sessionModelKey, s.modelKey);
          const nextModelProvider = sessionModel
            ? sessionModelKey?.provider
              ?? (sessionModel === s.model
              ? s.modelProvider
              : null)
            : s.modelProvider;
          const nextModelInfo = sessionModel
            ? sameExactSelection || (!sessionModelKey && sessionModel === s.model)
              ? s.modelInfo
              : null
            : s.modelInfo;
          const capabilityInput = nextModelInfo ?? sessionModel ?? s.model;
          const nextThinkingLevel = normalizeThinkingLevel(
            s.thinkingLevel,
            capabilityInput,
          );
          return {
            ...s,
            sessionId: data.session.id,
            sessionType: data.session.session_type,
            title: normalizeDisplayTitle(data.session.title),
            mode,
            permissionMode,
            model: sessionModel ?? s.model,
            modelKey: sessionModel ? sessionModelKey : s.modelKey,
            modelProvider: nextModelProvider,
            modelInfo: nextModelInfo,
            thinkingLevel: nextThinkingLevel,
            thinkingEnabled: isThinkingEnabled(nextThinkingLevel),
            fastModeEnabled: sessionModel
              ? s.fastModeEnabled
                && supportsFastMode(capabilityInput, nextModelProvider)
              : s.fastModeEnabled,
            tokenCount: data.session.token_count ?? 0,
            tokenUsage: null,
            error:
              serverState !== null
                ? sessionAgentErrorMessage(serverState)
                : previousSessionId === sessionId
                  ? s.error
                  : null,
            messages: applyLivePartialAssistant(
              applyRecoveryParity(
                processedMessages,
                serverState?.recovery,
                serverState?.agent_state ?? "idle",
              ),
              serverState?.live_partial_assistant,
              serverState?.agent_state ?? "idle",
              pendingInteractionsFromSnapshot(serverState),
            ),
            isLoading: false,
          };
        });
        try {
          storage.set("krusty-permission-mode", permissionMode);
        } catch {
          /* ignore */
        }

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
            data.session.target_branch ?? null,
          );

        applySessionSnapshot(sessionId, serverState, isRefresh, set, get, planStore);
        get().startPresenceHeartbeat(sessionId);
        if (
          sessionModel
          && (
            sessionModel !== previousModel
            || !modelKeysEqual(sessionModelKey, previousModelKey)
          )
        ) {
          void persistCurrentSelectedModel(sessionModel, sessionModelKey);
        }
      } catch (err) {
        if (err instanceof KrustyApiError && err.status === 404) {
          const current = get();
          current.stopPresenceHeartbeat(previousSessionId);
          if (workspace.getState().sessionId === sessionId) {
            workspace.getState().setSession(null);
          }
          set({
            ...initialState,
            permissionMode: current.permissionMode,
            model: current.model,
            modelKey: current.modelKey,
            modelProvider: current.modelProvider,
            modelInfo: current.modelInfo,
            thinkingLevel: current.thinkingLevel,
            thinkingEnabled: current.thinkingEnabled,
            fastModeEnabled: current.fastModeEnabled,
            isLoading: false,
            error: null,
          });
          sessionsStore.getState().loadSessions();
          return;
        }
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
        modelKey: current.modelKey,
        modelProvider: current.modelProvider,
        modelInfo: current.modelInfo,
        thinkingLevel: current.thinkingLevel,
        thinkingEnabled: current.thinkingEnabled,
        fastModeEnabled: current.fastModeEnabled,
      });
      workspace.getState().clear();
      planStore.getState().setWorkflow(null);
    },

    // -- initSession --------------------------------------------------------

    initSession(
      sessionId: string,
      title: string,
      permissionMode?: PermissionMode,
      sessionType?: import("@krusty/api").SessionType,
    ) {
      const current = get();
      get().stopPresenceHeartbeat(current.sessionId);
      const nextPermissionMode = permissionMode ?? current.permissionMode;
      try {
        storage.set("krusty-permission-mode", nextPermissionMode);
      } catch {
        /* ignore */
      }
      set({
        ...initialState,
        permissionMode: nextPermissionMode,
        model: current.model,
        modelKey: current.modelKey,
        modelProvider: current.modelProvider,
        modelInfo: current.modelInfo,
        thinkingLevel: current.thinkingLevel,
        thinkingEnabled: current.thinkingEnabled,
        fastModeEnabled: current.fastModeEnabled,
        sessionId,
        sessionType: sessionType ?? null,
        title: normalizeDisplayTitle(title),
      });
      planStore.getState().setWorkflow(null);
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
      if (get().sessionType !== "code") return;
      set({ mode });
      planStore.getState().setVisible(mode === "plan");
      void persistMode(get, mode);
    },

    async executeWorkflowCommand(command) {
      const sessionId = get().sessionId;
      if (!sessionId) {
        throw new Error("No active session");
      }
      const mutation = await client.executeWorkflowCommand(sessionId, command);
      planStore.getState().setWorkflow(mutation.snapshot);
      if (mutation.snapshot.goal.status === "active") {
        set({ mode: "build" });
      }
      return mutation;
    },

    // -- setModel -----------------------------------------------------------

    setModel(
      model: string | null,
      provider?: string | null,
      modelInfo?: ModelInfo | null,
      modelKey?: ModelKey | null,
    ) {
      const current = get();
      const nextModelInfo = model
        ? modelInfo !== undefined
          ? modelInfo
          : model === current.model
            ? current.modelInfo
            : null
        : null;
      const nextModelKey = model
        ? modelKey !== undefined
          ? modelKey
          : nextModelInfo?.key
            ?? (model === current.model ? current.modelKey : null)
        : null;
      const nextProvider = nextModelInfo?.provider
        ?? nextModelKey?.provider
        ?? provider
        ?? (model === current.model ? current.modelProvider : null);
      if (
        current.model === model
        && modelKeysEqual(current.modelKey, nextModelKey)
        && current.modelProvider === nextProvider
        && current.modelInfo === nextModelInfo
      ) {
        return;
      }

      set((s) => {
        const capabilityInput = nextModelInfo ?? model;
        const thinkingLevel = normalizeThinkingLevel(s.thinkingLevel, capabilityInput);
        return {
          model,
          modelKey: nextModelKey,
          modelProvider: nextProvider,
          modelInfo: nextModelInfo,
          thinkingLevel,
          thinkingEnabled: isThinkingEnabled(thinkingLevel),
          fastModeEnabled: model
            ? s.fastModeEnabled && supportsFastMode(capabilityInput, nextProvider)
            : false,
        };
      });
      void persistCurrentSelectedModel(model, nextModelKey);
      void persistModel(get, model, nextModelKey);
    },

    // -- setThinkingLevel ---------------------------------------------------

    setThinkingLevel(level: ThinkingLevel) {
      set((state) => {
        const normalized = normalizeThinkingLevel(
          level,
          state.modelInfo ?? state.model,
        );
        return {
          thinkingLevel: normalized,
          thinkingEnabled: isThinkingEnabled(normalized),
        };
      });
    },

    // -- toggleThinking -----------------------------------------------------

    toggleThinking() {
      set((s) => {
        const newLevel = cycleThinkingLevel(
          s.thinkingLevel,
          s.modelInfo ?? s.model,
        );
        return {
          thinkingEnabled: isThinkingEnabled(newLevel),
          thinkingLevel: newLevel,
        };
      });
    },

    // -- setFastModeEnabled -------------------------------------------------

    setFastModeEnabled(enabled: boolean) {
      set((s) => ({
        fastModeEnabled:
          enabled && supportsFastMode(s.modelInfo ?? s.model, s.modelProvider),
      }));
    },

    // -- toggleFastMode -----------------------------------------------------

    toggleFastMode() {
      set((s) => {
        if (!supportsFastMode(s.modelInfo ?? s.model, s.modelProvider)) {
          return { fastModeEnabled: false };
        }
        return { fastModeEnabled: !s.fastModeEnabled };
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
        void persistPermissionMode(get, newMode);
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
        current: createStreamingAssistantMessage(),
      };

      set((s) => ({
        messages: upsertTransientAssistantMessage(s.messages, ref.current),
      }));

      const streamRecovery: { promise: Promise<boolean> | null } = { promise: null };
      const callbacks = createRecoveringStreamCallbacks(
        createStreamCallbacks(ref, set, get, {
          planStore,
          sessionsStore,
          persistSessionMode: persistMode,
        }),
        state.sessionId,
        streamRecovery,
      );
      let keepStatePolling = false;

      try {
        await client.streamToolResult(
          {
            session_id: state.sessionId,
            tool_call_id: toolCallId,
            result,
            fast_mode: state.fastModeEnabled,
            thinking_enabled: thinkingLevelToApiValue(state.thinkingLevel),
            permission_mode: state.permissionMode,
          },
          callbacks,
          abortController.signal,
        );
      } catch (err) {
        streamRecovery.promise ??=
          recoverAfterStreamInterruption(state.sessionId);
        keepStatePolling = await streamRecovery.promise;
        if (!keepStatePolling) {
          applyStreamFailure(err);
        }
      } finally {
        if (streamRecovery.promise) {
          keepStatePolling = await streamRecovery.promise;
        }
        if (!keepStatePolling) {
          get().stopStatePolling();
        }
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
      const activeSessionId = get().sessionId;
      if (activeSessionId && get().isStreaming) {
        void client.cancelSession(activeSessionId).catch(() => undefined);
      }
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
      const generation = statePollingGeneration;
      let consecutiveFailures = 0;

      const schedule = (delay: number) => {
        if (generation !== statePollingGeneration) return;
        statePollingTimer = setTimeout(poll, delay);
      };

      const poll = async () => {
        if (generation !== statePollingGeneration) return;
        try {
          const serverState = await client.getSessionState(sessionId);
          if (generation !== statePollingGeneration) return;
          consecutiveFailures = 0;
          applySessionSnapshot(sessionId, serverState, true, set, get, planStore);

          if (get().error === STATE_POLL_DEGRADED_MESSAGE) {
            set({ error: null });
          }

          if (shouldStopSessionStatePolling(serverState.agent_state)) {
            const terminalError = sessionAgentErrorMessage(serverState);
            get().stopStatePolling();
            set({
              isStreaming: false,
              isThinking: false,
              thinkingContent: "",
              error: terminalError,
            });
            await get().loadSession(sessionId, true);
            if (terminalError) {
              set({ error: terminalError });
            }
            return;
          }

          schedule(STATE_POLL_INTERVAL);
        } catch {
          if (generation !== statePollingGeneration) return;
          consecutiveFailures += 1;

          if (consecutiveFailures >= STATE_POLL_MAX_FAILURES) {
            get().stopStatePolling();
            set({
              isLoading: false,
              error:
                `Unable to refresh session status after ${STATE_POLL_MAX_FAILURES} attempts. `
                + 'The run may still be active; reconnect or refresh before sending another message.',
            });
            return;
          }

          if (consecutiveFailures >= STATE_POLL_DEGRADED_AFTER) {
            set({ error: STATE_POLL_DEGRADED_MESSAGE });
          }

          const backoff = Math.min(
            STATE_POLL_INTERVAL * 2 ** (consecutiveFailures - 1),
            STATE_POLL_MAX_BACKOFF,
          );
          schedule(backoff);
        }
      };

      schedule(STATE_POLL_INTERVAL);
    },

    stopStatePolling() {
      statePollingGeneration += 1;
      if (statePollingTimer) {
        clearTimeout(statePollingTimer);
        statePollingTimer = null;
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
    };
  });
}
