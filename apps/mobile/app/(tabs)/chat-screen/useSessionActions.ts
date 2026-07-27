import { useCallback, type MutableRefObject } from "react";
import { Alert } from "react-native";

import type { ModelInfo, SessionResponse, SessionType } from "@krusty/api";
import type { Attachment as SessionAttachment } from "@krusty/state";
import type { useConnection } from "../../../hooks/useConnection";
import type { useStores } from "../../../hooks/useStores";
import * as Haptics from "../../../platform/haptics";
import * as SecureStore from "../../../platform/secure-store";
import {
  SELECTED_MODEL_KEY,
  getWorkspaceMode,
  sessionTypeForTab,
  tabForSessionType,
  type WorkspaceMode,
} from "./helpers";
import {
  findCodeSessionForProject,
  resolveSendIntent,
  type ResolvedSendIntent,
} from "./sendIntent";

type LoadedStores = NonNullable<ReturnType<typeof useStores>>;
type ConnectionClient = ReturnType<typeof useConnection>["client"];

type SessionStoreApi = LoadedStores["session"];
type SessionsStoreApi = LoadedStores["sessions"];
type WorkspaceStoreApi = LoadedStores["workspace"];
type ModeStores = LoadedStores["modes"];

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
        target.session
          .getState()
          .setModel(currentModel, modelInfo?.provider ?? null, modelInfo);
      }
      if (target.session.getState().thinkingLevel !== currentThinkingLevel) {
        target.session.getState().setThinkingLevel(currentThinkingLevel);
      }

      await sessionsStore.getState().loadSessions();
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
      if (targetStore.getState().sessionId) {
        targetStore.getState().detachSession();
      }

      try {
        await ensureModelReady(targetStore);
        const session = await client.createSession(
          undefined,
          directory,
          targetBranch ?? undefined,
          directory ? "selected" : "neutral",
          targetType,
          targetStore.getState().permissionMode,
        );
        await bootstrapSession(session);
        lastSessionIdByTypeRef.current[session.session_type] = session.id;
        setActiveTab(tabForSessionType(session.session_type));
        setActiveToolCallId(null);
        setDrawerOpen(false);
        void Haptics.notificationAsync(
          Haptics.NotificationFeedbackType.Success,
        );
        return session;
      } catch {
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
   * Ensure the durable per-user Mako companion is the active session.
   * Used when opening the Mako tab and before any Mako composer send.
   */
  const ensureMakoCompanionSession = useCallback(async (): Promise<string | null> => {
    if (!client) {
      return null;
    }
    return sessionStore.getState().ensureMakoMainSession();
  }, [client, sessionStore]);

  const ensureSessionForSend = useCallback(async (): Promise<ResolvedSendIntent | null> => {
    // Mako always sends on the durable main companion — never ad-hoc createSession.
    if (activeTab === 2 || sessionTypeForTab(activeTab) === "mako") {
      const mainId = await ensureMakoCompanionSession();
      if (!mainId) {
        return null;
      }
      return {
        shouldPrecreate: false,
        sendOptions: { sessionType: "mako" },
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
      const session = await client.createSession(
        undefined,
        precreate?.projectDir ?? undefined,
        precreate?.targetBranch ?? undefined,
        precreate?.workspaceMode,
        precreate?.sessionType ?? sessionTypeForTab(activeTab),
        sessionStore.getState().permissionMode,
      );
      await bootstrapSession(session);
      lastSessionIdByTypeRef.current[session.session_type] = session.id;
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
      const targetStore = modeStores[session.session_type].session;
      // Close drawer and switch mode immediately so the gesture feels instant.
      lastSessionIdByTypeRef.current[session.session_type] = session.id;
      setDrawerOpen(false);
      setActiveTab(tabForSessionType(session.session_type));
      // loadSession already detaches stream callbacks for the leaving session.
      // Avoid an extra detachSession() which can thrash presence/poll state.
      void targetStore.getState().loadSession(session.id);
    },
    [
      lastSessionIdByTypeRef,
      modeStores,
      setActiveTab,
      setDrawerOpen,
    ],
  );

  const loadSessionById = useCallback(
    async (id: string) => {
      setDrawerOpen(false);
      const target = sessions.find((session) => session.id === id);
      const targetType = target?.session_type ?? "mako";
      const targetStore = modeStores[targetType].session;
      lastSessionIdByTypeRef.current[targetType] = id;
      setActiveTab(tabForSessionType(targetType));
      void targetStore.getState().loadSession(id);
    },
    [
      lastSessionIdByTypeRef,
      modeStores,
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
        lastSessionIdByTypeRef.current.code = existing.id;
        // loadSession detaches the previous stream attachment; avoid thrashing
        // presence/poll state with an extra detachSession on thread open.
        void codeStore.getState().loadSession(existing.id);
        return;
      }

      try {
        await ensureModelReady(codeStore);
        const session = await client.createSession(
          undefined,
          projectDir,
          targetBranch ?? undefined,
          "selected",
          "code",
          codeStore.getState().permissionMode,
        );
        await bootstrapSession(session);
        lastSessionIdByTypeRef.current.code = session.id;
        setActiveToolCallId(null);
        void Haptics.notificationAsync(
          Haptics.NotificationFeedbackType.Success,
        );
      } catch {
        return;
      }
    },
    [
      bootstrapSession,
      client,
      ensureModelReady,
      modeStores,
      sessions,
      setActiveTab,
      setActiveToolCallId,
      setDrawerOpen,
      lastSessionIdByTypeRef,
    ],
  );

  const handleNewSession = useCallback(async (sessionType?: SessionType) => {
    // Mako has one durable companion — never create a fresh peer chat from "new".
    if (sessionType === "mako" || (sessionType == null && activeTab === 2)) {
      await ensureMakoCompanionSession();
      setActiveTab(2);
      setDrawerOpen(false);
      return;
    }
    await createSessionForCurrentTab(undefined, undefined, sessionType);
  }, [
    activeTab,
    createSessionForCurrentTab,
    ensureMakoCompanionSession,
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
            const activeEntry = (["chat", "code", "mako"] as const).find(
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
      void SecureStore.setItemAsync(SELECTED_MODEL_KEY, modelId);
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

  const handleTabChange = useCallback(
    async (index: number) => {
      setActiveTab(index);
      setDrawerOpen(false);
    },
    [
      setActiveTab,
      setDrawerOpen,
    ],
  );

  return {
    stopCurrentStream,
    detachCurrentSession,
    loadSession,
    loadSessionById,
    openProjectInCode,
    ensureMakoCompanionSession,
    handleNewSession,
    handleDirectorySelected,
    handleDeleteSession,
    handleInteractiveToolResult,
    handlePlanConfirm,
    handleSend,
    handleModelSelect,
    handleFastModeToggle,
    handleTabChange,
  };
}
