import { useCallback, useEffect, useRef, type MutableRefObject } from "react";
import { Alert } from "react-native";

import type { ModelInfo, SessionResponse, SessionType } from "@mitsuro/api";
import {
  beginMitsuroPerformanceSpan,
  type Attachment as SessionAttachment,
} from "@mitsuro/state";
import type { useConnection } from "../../../hooks/useConnection";
import type { useStores } from "../../../hooks/useStores";
import * as Haptics from "../../../platform/haptics";
import * as SecureStore from "../../../platform/secure-store";
import {
  getWorkspaceMode,
  sessionTypeForTab,
  tabForSessionType,
  type WorkspaceMode,
} from "./helpers";
import {
  IDENTITY_STORAGE_KEYS,
  writeCanonicalAsyncValue,
} from "../../../platform/identity-storage";
import {
  findCodeSessionForProject,
  resolveSendIntent,
  type ResolvedSendIntent,
} from "./sendIntent";
import { createSessionCreationCoordinator } from "./sessionCreationCoordinator";
import {
  createLatestIntentScheduler,
  type LatestIntentScheduler,
} from "../../../components/navigation/latestIntentScheduler";

type LoadedStores = NonNullable<ReturnType<typeof useStores>>;
type ConnectionClient = ReturnType<typeof useConnection>["client"];

type SessionStoreApi = LoadedStores["session"];
type SessionsStoreApi = LoadedStores["sessions"];
type WorkspaceStoreApi = LoadedStores["workspace"];
type ModeStores = LoadedStores["modes"];
type SessionSelectionIntent = {
  id: string;
  sessionType: SessionType;
  force?: boolean;
};

interface UseSessionActionsArgs {
  client: ConnectionClient;
  activeTab: number;
  activeToolCallId: string | null;
  setActiveToolCallId: (value: string | null) => void;
  setActiveTab: (value: number) => void;
  setDrawerOpen: (value: boolean) => void;
  ensureModelReady: (targetStore?: SessionStoreApi) => Promise<string | null>;
  sessionStore: SessionStoreApi;
  sessionsStore: SessionsStoreApi;
  workspace: WorkspaceStoreApi;
  modeStores: ModeStores;
  sessions: SessionResponse[];
  models: ModelInfo[];
  suppressCompletionRef: { current: boolean };
  lastSessionIdByTypeRef: MutableRefObject<Record<SessionType, string | null>>;
}

export function useSessionActions({
  client,
  activeTab,
  activeToolCallId,
  setActiveToolCallId,
  setActiveTab,
  setDrawerOpen,
  ensureModelReady,
  sessionStore,
  sessionsStore,
  workspace,
  modeStores,
  sessions,
  models,
  suppressCompletionRef,
  lastSessionIdByTypeRef,
}: UseSessionActionsArgs) {
  const sessionCreationCoordinatorRef = useRef(
    createSessionCreationCoordinator<SessionResponse | null>(),
  );
  const admitSessionSelectionRef = useRef(
    ({ id, sessionType, force = false }: SessionSelectionIntent) => {
      const targetStore = modeStores[sessionType].session;
      if (!force && targetStore.getState().sessionId === id) return;
      void targetStore.getState().loadSession(id, force);
    },
  );
  admitSessionSelectionRef.current = ({ id, sessionType, force = false }) => {
    const targetStore = modeStores[sessionType].session;
    if (!force && targetStore.getState().sessionId === id) return;
    void targetStore.getState().loadSession(id, force);
  };
  const sessionSelectionSchedulerRef = useRef<
    LatestIntentScheduler<SessionSelectionIntent> | null
  >(null);
  if (!sessionSelectionSchedulerRef.current) {
    sessionSelectionSchedulerRef.current = createLatestIntentScheduler({
      // Loading a transcript is heavy work. Keep replacing the pending
      // selection while the user is still tapping and hydrate only the final
      // quiet destination.
      quietDelayMs: 72,
      onFlush: (intent) => admitSessionSelectionRef.current(intent),
    });
  }
  useEffect(() => {
    const scheduler = sessionSelectionSchedulerRef.current;
    return () => {
      scheduler?.cancel();
    };
  }, []);
  const scheduleSessionSelection = useCallback(
    (intent: SessionSelectionIntent) => {
      sessionSelectionSchedulerRef.current?.submit(intent);
    },
    [],
  );
  const cancelPendingSessionSelection = useCallback(() => {
    sessionSelectionSchedulerRef.current?.cancel();
  }, []);
  const stopCurrentStream = useCallback(
    (suppressCompletion = true) => {
      if (sessionStore.getState().isStreaming) {
        suppressCompletionRef.current = suppressCompletion;
        sessionStore.getState().stopStreaming();
      }
      setActiveToolCallId(null);
    },
    [sessionStore, setActiveToolCallId, suppressCompletionRef],
  );

  const detachCurrentSession = useCallback(() => {
    suppressCompletionRef.current = true;
    sessionStore.getState().detachSession();
    setActiveToolCallId(null);
  }, [
    sessionStore,
    setActiveToolCallId,
    suppressCompletionRef,
  ]);

  const bootstrapSession = useCallback(
    async (session: SessionResponse) => {
      const target = modeStores[session.session_type];
      const currentState = target.session.getState();
      const currentModel = currentState.model;
      const currentModelInfo = currentState.modelInfo;
      const currentThinkingLevel = currentState.thinkingLevel;
      const directory = session.project_dir ?? session.working_dir ?? null;
      const workspaceMode = (session.workspace_mode ??
        getWorkspaceMode(directory)) as WorkspaceMode;

      // Bind durable identity immediately so the empty shell is interactive.
      target.session
        .getState()
        .initSession(
          session.id,
          session.title || "",
          session.permission_mode,
          session.session_type,
        );
      target.workspace
        .getState()
        .initFromSession(
          session.id,
          directory,
          workspaceMode,
          session.target_branch ?? null,
        );

      if (currentModel) {
        const modelInfo = currentModelInfo
          ?? models.find((candidate) => candidate.id === currentModel)
          ?? null;
        // Keep create path local-first; setModel may persist, but we already
        // have a usable model for the empty composer.
        target.session
          .getState()
          .setModel(currentModel, modelInfo?.provider ?? null, modelInfo);
      }
      if (target.session.getState().thinkingLevel !== currentThinkingLevel) {
        target.session.getState().setThinkingLevel(currentThinkingLevel);
      }

      // Optimistic list patch + soft refresh. Never block New Chat on GET /sessions.
      sessionsStore.getState().upsertSession({
        id: session.id,
        title: session.title || "",
        updated_at: session.updated_at,
        token_count: session.token_count ?? null,
        parent_session_id: session.parent_session_id ?? null,
        working_dir: session.working_dir ?? null,
        project_dir: session.project_dir ?? null,
        workspace_mode: session.workspace_mode,
        session_type: session.session_type,
        target_branch: session.target_branch ?? null,
        permission_mode: session.permission_mode,
      });
      void sessionsStore.getState().loadSessions();
    },
    [modeStores, models, sessionsStore],
  );

  const createSessionForCurrentTab = useCallback(
    async (
      directory?: string,
      targetBranch?: string | null,
      requestedType?: SessionType,
    ) => {
      if (!client) {
        return null;
      }

      const targetType = requestedType ?? sessionTypeForTab(activeTab);
      const targetStore = modeStores[targetType].session;
      const creationCoordinator = sessionCreationCoordinatorRef.current;
      sessionSelectionSchedulerRef.current?.cancel();
      const finishShellSpan = beginMitsuroPerformanceSpan(
        "new_chat.shell",
        targetType,
      );

      // Instant local shell: close chrome and clear previous session without
      // waiting on network. Composer becomes empty/interactive immediately.
      setDrawerOpen(false);
      setActiveTab(tabForSessionType(targetType));
      setActiveToolCallId(null);
      if (!creationCoordinator.hasPending(targetType)) {
        if (targetStore.getState().sessionId) {
          targetStore.getState().detachSession();
        }
        // Clear to a blank local draft shell before durable id arrives.
        targetStore.setState({
          sessionId: null,
          title: "",
          messages: [],
          isLoading: true,
          isStreaming: false,
          isThinking: false,
          thinkingContent: "",
          error: null,
          tokenCount: 0,
          queuedMessages: [],
        } as never);
      }
      finishShellSpan();

      try {
        const session = await creationCoordinator.run(
          targetType,
          async (isCurrent) => {
            const finishBindSpan = beginMitsuroPerformanceSpan(
              "new_chat.session_bind",
              targetType,
            );
            try {
              // Only hard-wait for a model when none is already usable.
              if (!targetStore.getState().model) {
                await ensureModelReady(targetStore);
              } else {
                void ensureModelReady(targetStore);
              }
              const created = await client.createSession(
                undefined,
                directory,
                targetBranch ?? undefined,
                directory ? "selected" : "neutral",
                targetType,
                targetStore.getState().permissionMode,
              );
              if (!isCurrent()) return null;
              await bootstrapSession(created);
              if (!isCurrent()) return null;
              lastSessionIdByTypeRef.current[created.session_type] = created.id;
              void Haptics.notificationAsync(
                Haptics.NotificationFeedbackType.Success,
              );
              return created;
            } finally {
              finishBindSpan();
            }
          },
        );
        return session;
      } catch {
        targetStore.setState({
          isLoading: false,
          error: "Failed to create session",
        } as never);
        return null;
      }
    },
    [
      activeTab,
      bootstrapSession,
      client,
      ensureModelReady,
      lastSessionIdByTypeRef,
      modeStores,
      setActiveToolCallId,
      setDrawerOpen,
      setActiveTab,
    ],
  );

  /**
   * Ensure the durable per-user Hive companion is the active session.
   * Used when opening the Hive tab and before any Hive composer send.
   */
  const ensureHiveCompanionSession = useCallback(async (): Promise<string | null> => {
    if (!client) {
      return null;
    }
    return sessionStore.getState().ensureHiveMainSession();
  }, [client, sessionStore]);

  const ensureSessionForSend = useCallback(async (): Promise<ResolvedSendIntent | null> => {
    // Hive always sends on the durable main companion — never ad-hoc createSession.
    if (activeTab === 2 || sessionTypeForTab(activeTab) === "hive") {
      const mainId = await ensureHiveCompanionSession();
      if (!mainId) {
        return null;
      }
      return {
        shouldPrecreate: false,
        sendOptions: { sessionType: "hive" },
      };
    }

    const currentSessionId = sessionStore.getState().sessionId;
    const wsState = workspace.getState();
    const intent = resolveSendIntent({
      activeTab,
      currentSessionId,
      workspaceDirectory: wsState.directory,
      workspaceMode: wsState.mode,
      targetBranch: wsState.targetBranch,
    });

    if (!intent.shouldPrecreate) {
      return intent;
    }

    if (!client) {
      return null;
    }

    try {
      const precreate = intent.precreate;
      const targetType = precreate?.sessionType ?? sessionTypeForTab(activeTab);
      const session = await sessionCreationCoordinatorRef.current.run(
        targetType,
        async (isCurrent) => {
          const finishBindSpan = beginMitsuroPerformanceSpan(
            "new_chat.session_bind",
            targetType,
          );
          try {
            const created = await client.createSession(
              undefined,
              precreate?.projectDir ?? undefined,
              precreate?.targetBranch ?? undefined,
              precreate?.workspaceMode,
              targetType,
              sessionStore.getState().permissionMode,
            );
            if (!isCurrent()) return null;
            await bootstrapSession(created);
            if (!isCurrent()) return null;
            lastSessionIdByTypeRef.current[created.session_type] = created.id;
            return created;
          } finally {
            finishBindSpan();
          }
        },
      );
      if (!session) {
        return null;
      }
      return { ...intent, sendOptions: undefined };
    } catch {
      return null;
    }
  }, [
    activeTab,
    bootstrapSession,
    client,
    lastSessionIdByTypeRef,
    sessionStore,
    workspace,
  ]);

  const loadSession = useCallback(
    async (session: SessionResponse) => {
      sessionCreationCoordinatorRef.current.invalidate(session.session_type);
      // Close drawer and switch mode immediately so the gesture feels instant.
      lastSessionIdByTypeRef.current[session.session_type] = session.id;
      setDrawerOpen(false);
      setActiveTab(tabForSessionType(session.session_type));
      // loadSession already detaches stream callbacks for the leaving session.
      // Avoid an extra detachSession() which can thrash presence/poll state.
      scheduleSessionSelection({
        id: session.id,
        sessionType: session.session_type,
      });
    },
    [
      lastSessionIdByTypeRef,
      modeStores,
      scheduleSessionSelection,
      setActiveTab,
      setDrawerOpen,
    ],
  );

  const loadSessionById = useCallback(
    async (id: string) => {
      setDrawerOpen(false);
      const target = sessions.find((session) => session.id === id);
      const targetType = target?.session_type ?? "hive";
      sessionCreationCoordinatorRef.current.invalidate(targetType);
      lastSessionIdByTypeRef.current[targetType] = id;
      setActiveTab(tabForSessionType(targetType));
      scheduleSessionSelection({ id, sessionType: targetType });
    },
    [
      lastSessionIdByTypeRef,
      modeStores,
      scheduleSessionSelection,
      sessions,
      setActiveTab,
      setDrawerOpen,
    ],
  );

  const openProjectInCode = useCallback(
    async (projectDir: string, targetBranch?: string | null) => {
      if (!client) {
        return;
      }

      setDrawerOpen(false);
      setActiveTab(1);
      const codeStore = modeStores.code.session;

      const existing = findCodeSessionForProject(
        sessions,
        projectDir,
        targetBranch ?? null,
      );

      if (existing) {
        sessionCreationCoordinatorRef.current.invalidate("code");
        lastSessionIdByTypeRef.current.code = existing.id;
        // loadSession detaches the previous stream attachment; avoid thrashing
        // presence/poll state with an extra detachSession on thread open.
        scheduleSessionSelection({ id: existing.id, sessionType: "code" });
        return;
      }

      sessionSelectionSchedulerRef.current?.cancel();

      try {
        const session = await sessionCreationCoordinatorRef.current.run(
          "code",
          async (isCurrent) => {
            await ensureModelReady(codeStore);
            const created = await client.createSession(
              undefined,
              projectDir,
              targetBranch ?? undefined,
              "selected",
              "code",
              codeStore.getState().permissionMode,
            );
            if (!isCurrent()) {
              return null;
            }
            await bootstrapSession(created);
            if (!isCurrent()) {
              return null;
            }
            lastSessionIdByTypeRef.current.code = created.id;
            setActiveToolCallId(null);
            void Haptics.notificationAsync(
              Haptics.NotificationFeedbackType.Success,
            );
            return created;
          },
        );
        if (!session) return;
      } catch {
        return;
      }
    },
    [
      bootstrapSession,
      client,
      ensureModelReady,
      modeStores,
      scheduleSessionSelection,
      sessions,
      setActiveTab,
      setActiveToolCallId,
      setDrawerOpen,
      lastSessionIdByTypeRef,
    ],
  );

  const handleNewSession = useCallback(async (sessionType?: SessionType) => {
    // Hive has one durable companion — never create a fresh peer chat from "new".
    if (sessionType === "hive" || (sessionType == null && activeTab === 2)) {
      await ensureHiveCompanionSession();
      setActiveTab(2);
      setDrawerOpen(false);
      return;
    }
    await createSessionForCurrentTab(undefined, undefined, sessionType);
  }, [
    activeTab,
    createSessionForCurrentTab,
    ensureHiveCompanionSession,
    setActiveTab,
    setDrawerOpen,
  ]);

  const handleDirectorySelected = useCallback(
    async (path: string) => {
      await createSessionForCurrentTab(path, undefined, "code");
    },
    [createSessionForCurrentTab],
  );

  const handleDeleteSession = useCallback(
    (id: string) => {
      Alert.alert("Delete Session", "Delete this session?", [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: async () => {
            const activeEntry = (["chat", "code", "hive"] as const).find(
              (mode) => modeStores[mode].session.getState().sessionId === id,
            );
            const targetStore = activeEntry
              ? modeStores[activeEntry].session
              : null;

            if (targetStore?.getState().isStreaming) {
              targetStore.getState().stopStreaming();
            }

            const deleted = await sessionsStore.getState().deleteSession(id);
            if (!deleted) {
              return;
            }

            if (targetStore) {
              targetStore.getState().clearSession();
              setActiveToolCallId(null);
            }

            void sessionsStore.getState().loadSessions();
          },
        },
      ]);
    },
    [modeStores, sessionsStore, setActiveToolCallId],
  );

  const handleInteractiveToolResult = useCallback(
    async (toolCallId: string, result: string) => {
      const currentSessionId = sessionStore.getState().sessionId;
      if (!currentSessionId || activeToolCallId) {
        return;
      }

      setActiveToolCallId(toolCallId);
      try {
        await sessionStore.getState().submitToolResult(toolCallId, result);
      } catch {
        await sessionStore.getState().loadSession(currentSessionId, true);
      } finally {
        setActiveToolCallId(null);
      }
    },
    [activeToolCallId, sessionStore, setActiveToolCallId],
  );

  const handlePlanConfirm = useCallback(
    async (toolCallId: string, choice: "execute" | "abandon") => {
      if (choice === "execute") {
        sessionStore.getState().setMode("build");
      }
      await handleInteractiveToolResult(toolCallId, JSON.stringify({ choice }));
    },
    [handleInteractiveToolResult, sessionStore],
  );

  const handleSend = useCallback(
    async (content: string, attachments: SessionAttachment[] = []) => {
      const trimmed = content.trim();
      if (!client || (!trimmed && attachments.length === 0)) {
        return;
      }

      const resolvedModel = await ensureModelReady();
      if (!resolvedModel) {
        sessionStore.setState({
          error:
            "No model is available yet. Check your model settings and try again.",
        });
        return;
      }

      const sendIntent = await ensureSessionForSend();
      if (!sendIntent) {
        return;
      }

      try {
        await sessionStore
          .getState()
          .sendMessage(
            trimmed,
            attachments,
            sendIntent.sendOptions,
          );
      } catch (err) {
        sessionStore.setState({
          error:
            err instanceof Error
              ? err.message
              : "Failed to send message.",
        });
      }
    },
    [
      activeTab,
      client,
      ensureModelReady,
      ensureSessionForSend,
      sessionStore,
      workspace,
    ],
  );

  const handleModelSelect = useCallback(
    (modelId: string) => {
      const modelInfo = models.find((candidate) => candidate.id === modelId);
      sessionStore.setState({ error: null });
      sessionStore
        .getState()
        .setModel(modelId, modelInfo?.provider ?? null, modelInfo ?? null);
      void writeCanonicalAsyncValue(
        SecureStore,
        IDENTITY_STORAGE_KEYS.selectedModel,
        modelId,
      );
    },
    [models, sessionStore],
  );

  const handleFastModeToggle = useCallback(() => {
    const currentModel = sessionStore.getState().model;
    if (currentModel) {
      const modelInfo = sessionStore.getState().modelInfo
        ?? models.find((candidate) => candidate.id === currentModel)
        ?? null;
      sessionStore
        .getState()
        .setModel(currentModel, modelInfo?.provider ?? null, modelInfo);
    }
    sessionStore.getState().toggleFastMode();
  }, [models, sessionStore]);

  return {
    stopCurrentStream,
    detachCurrentSession,
    loadSession,
    loadSessionById,
    openProjectInCode,
    ensureHiveCompanionSession,
    handleNewSession,
    handleDirectorySelected,
    handleDeleteSession,
    handleInteractiveToolResult,
    handlePlanConfirm,
    handleSend,
    handleModelSelect,
    handleFastModeToggle,
    cancelPendingSessionSelection,
  };
}
