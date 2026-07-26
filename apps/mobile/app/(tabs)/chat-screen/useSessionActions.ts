import { useCallback } from "react";
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

interface UseSessionActionsArgs {
  client: ConnectionClient;
  activeTab: number;
  activeToolCallId: string | null;
  setActiveToolCallId: (value: string | null) => void;
  setActiveTab: (value: number) => void;
  setDrawerOpen: (value: boolean) => void;
  ensureModelReady: () => Promise<string | null>;
  sessionStore: SessionStoreApi;
  sessionsStore: SessionsStoreApi;
  workspace: WorkspaceStoreApi;
  sessions: SessionResponse[];
  models: ModelInfo[];
  suppressCompletionRef: { current: boolean };
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
  sessions,
  models,
  suppressCompletionRef,
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

  const bootstrapSession = useCallback(
    async (session: SessionResponse) => {
      const currentState = sessionStore.getState();
      const currentModel = currentState.model;
      const currentModelInfo = currentState.modelInfo;
      const currentThinkingLevel = sessionStore.getState().thinkingLevel;
      const directory = session.project_dir ?? session.working_dir ?? null;
      const workspaceMode = (session.workspace_mode ??
        getWorkspaceMode(directory)) as WorkspaceMode;

      sessionStore
        .getState()
        .initSession(
          session.id,
          session.title || "",
          session.permission_mode,
          session.session_type,
        );
      workspace
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
        sessionStore
          .getState()
          .setModel(currentModel, modelInfo?.provider ?? null, modelInfo);
      }
      if (sessionStore.getState().thinkingLevel !== currentThinkingLevel) {
        sessionStore.getState().setThinkingLevel(currentThinkingLevel);
      }

      await sessionsStore.getState().loadSessions();
    },
    [models, sessionStore, sessionsStore, workspace],
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

      stopCurrentStream();

      try {
        await ensureModelReady();
        const session = await client.createSession(
          undefined,
          directory,
          targetBranch ?? undefined,
          directory ? "selected" : "neutral",
          requestedType ?? sessionTypeForTab(activeTab),
          sessionStore.getState().permissionMode,
        );
        await bootstrapSession(session);
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
      sessionStore,
      setActiveToolCallId,
      setDrawerOpen,
      setActiveTab,
      stopCurrentStream,
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
      return { ...intent, sendOptions: undefined };
    } catch {
      return null;
    }
  }, [
    activeTab,
    bootstrapSession,
    client,
    ensureMakoCompanionSession,
    sessionStore,
    workspace,
  ]);

  const loadSession = useCallback(
    async (session: SessionResponse) => {
      stopCurrentStream();
      setDrawerOpen(false);
      setActiveTab(tabForSessionType(session.session_type));
      await sessionStore.getState().loadSession(session.id);
    },
    [sessionStore, setActiveTab, setDrawerOpen, stopCurrentStream],
  );

  const loadSessionById = useCallback(
    async (id: string) => {
      stopCurrentStream();
      setDrawerOpen(false);
      setActiveTab(2);
      await sessionStore.getState().loadSession(id);
    },
    [sessionStore, setActiveTab, setDrawerOpen, stopCurrentStream],
  );

  const openProjectInCode = useCallback(
    async (projectDir: string, targetBranch?: string | null) => {
      if (!client) {
        return;
      }

      stopCurrentStream();
      setDrawerOpen(false);
      setActiveTab(1);

      const existing = findCodeSessionForProject(
        sessions,
        projectDir,
        targetBranch ?? null,
      );

      if (existing) {
        await sessionStore.getState().loadSession(existing.id);
        return;
      }

      try {
        await ensureModelReady();
        const session = await client.createSession(
          undefined,
          projectDir,
          targetBranch ?? undefined,
          "selected",
          "code",
          sessionStore.getState().permissionMode,
        );
        await bootstrapSession(session);
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
      sessionStore,
      sessions,
      setActiveTab,
      setActiveToolCallId,
      setDrawerOpen,
      stopCurrentStream,
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
            const isActiveSession = sessionStore.getState().sessionId === id;

            if (isActiveSession) {
              stopCurrentStream();
            }

            const deleted = await sessionsStore.getState().deleteSession(id);
            if (!deleted) {
              return;
            }

            if (isActiveSession) {
              sessionStore.getState().clearSession();
              setActiveToolCallId(null);
            }

            void sessionsStore.getState().loadSessions();
          },
        },
      ]);
    },
    [sessionStore, sessionsStore, setActiveToolCallId, stopCurrentStream],
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
    (index: number) => {
      setActiveTab(index);

      // Mako tab: always bind to the durable main companion (not last Code/Chat).
      if (index === 2) {
        void ensureMakoCompanionSession();
        return;
      }

      const currentSessionId = sessionStore.getState().sessionId;
      if (!currentSessionId) {
        return;
      }

      const currentSession = sessions.find(
        (session) => session.id === currentSessionId,
      );
      if (
        !currentSession ||
        currentSession.session_type !== sessionTypeForTab(index)
      ) {
        stopCurrentStream();
        sessionStore.getState().clearSession();
      }
    },
    [
      ensureMakoCompanionSession,
      sessionStore,
      sessions,
      setActiveTab,
      stopCurrentStream,
    ],
  );

  return {
    stopCurrentStream,
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
