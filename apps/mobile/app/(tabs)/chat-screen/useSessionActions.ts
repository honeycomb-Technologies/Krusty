import { useCallback } from "react";
import { Alert } from "react-native";

import type { ModelInfo, SessionResponse } from "@krusty/api";
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
  researchEnabled: boolean;
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
  researchEnabled,
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
      const currentModel = sessionStore.getState().model;
      const currentThinkingLevel = sessionStore.getState().thinkingLevel;
      const directory = session.project_dir ?? session.working_dir ?? null;
      const workspaceMode = (session.workspace_mode ??
        getWorkspaceMode(directory)) as WorkspaceMode;

      sessionStore.getState().initSession(session.id, session.title || "");
      workspace
        .getState()
        .initFromSession(session.id, directory, workspaceMode);

      if (currentModel) {
        const modelInfo = models.find((candidate) => candidate.id === currentModel);
        sessionStore.getState().setModel(currentModel, modelInfo?.provider ?? null);
      }
      if (sessionStore.getState().thinkingLevel !== currentThinkingLevel) {
        sessionStore.getState().setThinkingLevel(currentThinkingLevel);
      }

      await sessionsStore.getState().loadSessions();
    },
    [models, sessionStore, sessionsStore, workspace],
  );

  const createSessionForCurrentTab = useCallback(
    async (directory?: string) => {
      if (!client) {
        return null;
      }

      stopCurrentStream();

      try {
        await ensureModelReady();
        const session = await client.createSession(
          undefined,
          directory,
          undefined,
          directory ? "selected" : undefined,
          sessionTypeForTab(activeTab),
        );
        await bootstrapSession(session);
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
      setActiveToolCallId,
      setDrawerOpen,
      stopCurrentStream,
    ],
  );

  const ensureSessionForSend = useCallback(async () => {
    const currentSessionId = sessionStore.getState().sessionId;
    if (currentSessionId) {
      return currentSessionId;
    }

    if (!client) {
      return null;
    }

    try {
      const session = await client.createSession(
        undefined,
        undefined,
        undefined,
        undefined,
        sessionTypeForTab(activeTab),
      );
      await bootstrapSession(session);
      return session.id;
    } catch {
      return null;
    }
  }, [activeTab, bootstrapSession, client, sessionStore]);

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
    async (projectDir: string) => {
      if (!client) {
        return;
      }

      stopCurrentStream();
      setDrawerOpen(false);
      setActiveTab(1);

      const existing = sessions.find(
        (session) =>
          session.session_type === "code" &&
          (session.project_dir === projectDir || session.working_dir === projectDir),
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
          undefined,
          "selected",
          "code",
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

  const handleNewSession = useCallback(async () => {
    await createSessionForCurrentTab();
  }, [createSessionForCurrentTab]);

  const handleDirectorySelected = useCallback(
    async (path: string) => {
      await createSessionForCurrentTab(path);
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

      const ensuredSessionId = await ensureSessionForSend();
      if (!ensuredSessionId) {
        return;
      }

      try {
        await sessionStore
          .getState()
          .sendMessage(trimmed, attachments, researchEnabled);
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
      client,
      ensureModelReady,
      ensureSessionForSend,
      researchEnabled,
      sessionStore,
    ],
  );

  const handleModelSelect = useCallback(
    (modelId: string) => {
      const modelInfo = models.find((candidate) => candidate.id === modelId);
      sessionStore.getState().setModel(modelId, modelInfo?.provider ?? null);
      void SecureStore.setItemAsync(SELECTED_MODEL_KEY, modelId);
    },
    [models, sessionStore],
  );

  const handleFastModeToggle = useCallback(() => {
    const currentModel = sessionStore.getState().model;
    if (currentModel) {
      const modelInfo = models.find((candidate) => candidate.id === currentModel);
      sessionStore.getState().setModel(currentModel, modelInfo?.provider ?? null);
    }
    sessionStore.getState().toggleFastMode();
  }, [models, sessionStore]);

  const handleTabChange = useCallback(
    (index: number) => {
      setActiveTab(index);

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
    [sessionStore, sessions, setActiveTab, stopCurrentStream],
  );

  return {
    stopCurrentStream,
    loadSession,
    loadSessionById,
    openProjectInCode,
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
