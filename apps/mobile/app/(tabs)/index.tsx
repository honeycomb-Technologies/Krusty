import { useState, useRef, useCallback, useEffect, useMemo } from "react";
import {
  AppState,
  View,
  StyleSheet,
  Text,
  Pressable,
  Alert,
  ActivityIndicator,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { router } from "expo-router";
import { Menu, FileSearch } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import * as SecureStore from "../../platform/secure-store";
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import {
  useSessionStore,
  useSessionsStore,
  useStores,
  useWorkspaceStore,
} from "../../hooks/useStores";
import { ChatTranscript } from "../../components/chat/ChatTranscript";
import { KrustyLogo } from "../../components/ui/KrustyLogo";
import {
  ChatBar,
  type Attachment as ChatBarAttachment,
} from "../../components/chat/ChatBar";
import { SessionDrawer } from "../../components/chat/SessionDrawer";
import { DesktopShell } from "../../components/layout/DesktopShell";
import { ReportsViewer } from "../../components/ReportsViewer";
import { MakoScreen } from "../../components/mako/MakoScreen";
import { useSplashState } from "../../hooks/useSplashState";
import { useEntranceAnimation } from "../../hooks/useEntranceAnimation";
import { useLiveActivity } from "../../hooks/useLiveActivity";
import { useWidgetSync } from "../../hooks/useWidgetSync";
import { useNotifications } from "../../hooks/useNotifications";
import Animated from "react-native-reanimated";
import type {
  ChatMessage,
  ModelInfo,
  SessionResponse,
  SessionType,
} from "@krusty/api";
import type {
  Attachment as SessionAttachment,
  PermissionMode,
  ThinkingLevel,
  ToolCall,
} from "@krusty/state";
import {
  isFastModeModel,
  supportsFastMode,
  toggleFastModeModel,
} from "@krusty/state";

const TAB_TYPES: SessionType[] = ["chat", "code", "mako"];
const CHAT_BAR_ZONE = 130;
const SELECTED_MODEL_KEY = "krusty_selected_model";

type WorkspaceMode = "neutral" | "selected" | "created";
type LoadedStores = NonNullable<ReturnType<typeof useStores>>;

function normalizeProviderId(provider: string | null | undefined): string {
  return (provider ?? "").trim().toLowerCase();
}

function isModelUsable(
  modelId: string | null | undefined,
  catalog: ModelInfo[],
  configuredProviders: string[],
): boolean {
  if (!modelId) {
    return false;
  }

  const match = catalog.find((candidate) => candidate.id === modelId);
  if (!match) {
    return false;
  }

  if (configuredProviders.length === 0) {
    return true;
  }

  return configuredProviders.includes(normalizeProviderId(match.provider));
}

function sessionTypeForTab(index: number): SessionType {
  return TAB_TYPES[index] ?? "code";
}

function tabForSessionType(type: SessionType): number {
  switch (type) {
    case "chat":
      return 0;
    case "mako":
      return 2;
    default:
      return 1;
  }
}

function getLastAssistantMessage(messages: ChatMessage[]): ChatMessage | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message?.role === "assistant") {
      return message;
    }
  }
  return null;
}

function flattenToolCalls(messages: ChatMessage[]): ToolCall[] {
  const toolCalls: ToolCall[] = [];
  for (const message of messages) {
    if (message.toolCalls?.length) {
      toolCalls.push(...message.toolCalls);
    }
  }
  return toolCalls;
}

function getActiveToolCall(toolCalls: ToolCall[]): ToolCall | null {
  for (let index = toolCalls.length - 1; index >= 0; index -= 1) {
    const toolCall = toolCalls[index];
    if (
      toolCall &&
      (toolCall.status === "awaiting_approval" ||
        toolCall.status === "running" ||
        toolCall.status === "pending")
    ) {
      return toolCall;
    }
  }
  return null;
}

function getWorkspaceMode(path: string | null): WorkspaceMode {
  return path ? "selected" : "neutral";
}

export default function ChatScreen() {
  const { theme } = useThemeContext();
  const {
    status,
    error: connectionError,
    reconnect,
    isConfigured,
  } = useConnection();
  const stores = useStores();

  if (!stores) {
    const t = theme.colors;
    const isRetryable = status === "error" || status === "disconnected";

    return (
      <SafeAreaView style={[styles.bootScreen, { backgroundColor: t.background }]}>
        <View style={styles.bootInner}>
          <KrustyLogo />
          {status === "connecting" ? (
            <>
              <ActivityIndicator
                size="small"
                color={t.userMessage}
                style={styles.bootSpinner}
              />
              <Text style={[styles.bootMessage, { color: t.mutedForeground }]}>
                Reconnecting to your server...
              </Text>
            </>
          ) : null}
          {isRetryable ? (
            <View style={styles.bootActions}>
              <Text
                style={[
                  styles.bootMessage,
                  {
                    color: isConfigured ? t.error : t.mutedForeground,
                    marginTop: 0,
                  },
                ]}
              >
                {connectionError ||
                  (isConfigured
                    ? "Could not reconnect to your server."
                    : "Server connection is not configured.")}
              </Text>
              <Pressable
                onPress={() => {
                  if (isConfigured) {
                    void reconnect();
                  } else {
                    router.replace("/onboarding");
                  }
                }}
                style={[
                  styles.bootButton,
                  { backgroundColor: t.userMessage },
                ]}
              >
                <Text style={styles.bootButtonText}>
                  {isConfigured ? "Retry Connection" : "Open Setup"}
                </Text>
              </Pressable>
              {isConfigured ? (
                <Pressable
                  onPress={() => router.replace("/onboarding")}
                  style={[
                    styles.bootButtonSecondary,
                    { borderColor: t.border },
                  ]}
                >
                  <Text
                    style={[
                      styles.bootButtonSecondaryText,
                      { color: t.foreground },
                    ]}
                  >
                    Server Setup
                  </Text>
                </Pressable>
              ) : null}
            </View>
          ) : null}
        </View>
      </SafeAreaView>
    );
  }

  return <ChatScreenContent stores={stores} />;
}

function ChatScreenContent({ stores }: { stores: LoadedStores }) {
  const { theme } = useThemeContext();
  const { client, isConnected } = useConnection();
  const { isDesktop } = useBreakpoint();
  const { splashDone } = useSplashState();
  const entrance = useEntranceAnimation(splashDone);

  const { sessions: sessionsStore, session: sessionStore, workspace } = stores;

  const sessions = useSessionsStore(
    (state) => state.sessions,
  ) as SessionResponse[];
  const sessionId = useSessionStore((state) => state.sessionId) ?? null;
  const sessionTitle = useSessionStore((state) => state.title) ?? null;
  const messages = useSessionStore((state) => state.messages) ?? [];
  const isStreaming = useSessionStore((state) => state.isStreaming) ?? false;
  const isThinking = useSessionStore((state) => state.isThinking) ?? false;
  const model = useSessionStore((state) => state.model) ?? null;
  const thinkingLevel =
    useSessionStore((state) => state.thinkingLevel) ?? "medium";
  const permissionMode =
    useSessionStore((state) => state.permissionMode) ?? "supervised";
  const mode = useSessionStore((state) => state.mode) ?? "build";
  const tokenCount = useSessionStore((state) => state.tokenCount) ?? 0;
  const error = useSessionStore((state) => state.error) ?? null;
  const isLoading = useSessionStore((state) => state.isLoading) ?? false;
  const workspaceDirectory =
    useWorkspaceStore((state) => state.directory) ?? null;
  const fastModeSupported = supportsFastMode(model);
  const fastModeEnabled = isFastModeModel(model);

  const [models, setModels] = useState<ModelInfo[]>([]);
  const [defaultModelId, setDefaultModelId] = useState<string | null>(null);
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([]);
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState(1);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [reportsOpen, setReportsOpen] = useState(false);
  const [researchEnabled, setResearchEnabled] = useState(false);
  const [composerReserveHeight, setComposerReserveHeight] =
    useState(CHAT_BAR_ZONE);

  const previousStreamingRef = useRef(false);
  const currentStreamSessionIdRef = useRef<string | null>(null);
  const streamStartedAtRef = useRef<number | null>(null);
  const liveActivityOpenRef = useRef(false);
  const notifiedApprovalIdsRef = useRef<Set<string>>(new Set());
  const suppressCompletionRef = useRef(false);

  const lastAssistantMessage = useMemo(
    () => getLastAssistantMessage(messages),
    [messages],
  );
  const toolCalls = useMemo(() => flattenToolCalls(messages), [messages]);
  const awaitingApprovalCalls = useMemo(
    () =>
      toolCalls.filter((toolCall) => toolCall.status === "awaiting_approval"),
    [toolCalls],
  );
  const activeToolCall = useMemo(
    () => getActiveToolCall(toolCalls),
    [toolCalls],
  );

  const lastAssistantSnippet =
    lastAssistantMessage?.content?.slice(0, 200) ?? "";

  const handleToolApprovalAction = useCallback(
    async (targetSessionId: string, toolCallId: string, approved: boolean) => {
      if (!targetSessionId || activeToolCallId) {
        return;
      }

      const currentSessionId = sessionStore.getState().sessionId;
      if (!currentSessionId && !client) {
        return;
      }

      setActiveToolCallId(toolCallId);
      try {
        if (currentSessionId === targetSessionId) {
          await sessionStore.getState().submitToolApproval(toolCallId, approved);
        } else if (client) {
          await client.submitToolApproval(targetSessionId, toolCallId, approved);
        }
      } catch {
        if (currentSessionId === targetSessionId) {
          await sessionStore.getState().loadSession(targetSessionId, true);
        }
      } finally {
        setActiveToolCallId(null);
      }
    },
    [activeToolCallId, client, sessionStore],
  );

  const handleNotificationNavigate = useCallback(
    async (_route: string, params?: Record<string, string>) => {
      const focus = params?.focus;
      const targetSessionId = params?.sessionId;
      const shouldOpenReports = params?.openReports === "true";

      if (focus === "mako") {
        setActiveTab(2);
      }
      if (shouldOpenReports) {
        setReportsOpen(true);
      }
      if (!targetSessionId) {
        return;
      }

      try {
        await sessionStore.getState().loadSession(targetSessionId, true);
      } catch {
        void sessionsStore.getState().loadSessions();
      }
    },
    [sessionStore, sessionsStore],
  );

  const handleRegisterNativeDevice = useCallback(
    async (deviceToken: string) => {
      if (!client || !isConnected || !deviceToken) {
        return;
      }

      try {
        await client.registerApnsDevice(deviceToken);
      } catch {}
    },
    [client, isConnected],
  );

  const { startActivity, updateActivity, endActivity } = useLiveActivity({
    onToolApproval: handleToolApprovalAction,
  });
  const { notifyToolApproval, notifyStreamComplete } = useNotifications({
    onToolApproval: handleToolApprovalAction,
    onNavigate: handleNotificationNavigate,
    onRegisterNativeDevice: handleRegisterNativeDevice,
  });

  useWidgetSync({
    hasActiveSession: Boolean(sessionId),
    sessionTitle: sessionTitle || "Untitled",
    lastMessage: lastAssistantSnippet,
    model: model || "",
    isStreaming,
    tokenCount,
    serverConnected: isConnected,
  });

  const t = theme.colors;

  const loadModelCatalog = useCallback(async () => {
    if (!client || !isConnected) {
      return null;
    }

    const [response, credentials] = await Promise.all([
      client.getModels(),
      client.getCredentials().catch(() => []),
    ]);
    const nextConfiguredProviders = credentials
      .filter((provider) => provider.configured || provider.has_oauth)
      .map((provider) => normalizeProviderId(provider.name));
    setModels(response.models);
    setDefaultModelId(response.default_model ?? null);
    setConfiguredProviders(nextConfiguredProviders);
    return {
      response,
      configuredProviders: nextConfiguredProviders,
    };
  }, [client, isConnected]);

  const ensureModelReady = useCallback(async () => {
    const existingModel = sessionStore.getState().model;
    let catalog = models;
    let fallbackDefault = defaultModelId;
    let allowedProviders = configuredProviders;

    if (catalog.length === 0) {
      const result = await loadModelCatalog().catch(() => null);
      if (!result) {
        return null;
      }
      catalog = result.response.models;
      fallbackDefault = result.response.default_model ?? null;
      allowedProviders = result.configuredProviders;
    }

    if (isModelUsable(existingModel, catalog, allowedProviders)) {
      return existingModel;
    }

    const saved = await SecureStore.getItemAsync(SELECTED_MODEL_KEY);
    const firstUsableModel =
      catalog.find((candidate) =>
        allowedProviders.length === 0
          ? true
          : allowedProviders.includes(normalizeProviderId(candidate.provider)),
      )?.id ?? null;
    const selectedModel = isModelUsable(saved, catalog, allowedProviders)
      ? saved
      : isModelUsable(fallbackDefault, catalog, allowedProviders)
        ? fallbackDefault
        : firstUsableModel;

    if (selectedModel) {
      sessionStore.getState().setModel(selectedModel);
      if (saved !== selectedModel) {
        await SecureStore.setItemAsync(SELECTED_MODEL_KEY, selectedModel);
      }
    }

    return selectedModel;
  }, [configuredProviders, defaultModelId, loadModelCatalog, models, sessionStore]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    void sessionsStore.getState().loadSessions();
    void ensureModelReady();
  }, [client, ensureModelReady, isConnected, sessionsStore]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    const refreshHandle = setInterval(() => {
      if (AppState.currentState === "active") {
        void sessionsStore.getState().loadSessions();
      }
    }, 5000);

    return () => clearInterval(refreshHandle);
  }, [client, isConnected, sessionsStore]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (nextState) => {
      if (nextState !== "active") {
        return;
      }

      const currentSessionId = sessionStore.getState().sessionId;
      if (currentSessionId) {
        void sessionStore.getState().loadSession(currentSessionId, true);
      }
    });

    return () => subscription.remove();
  }, [sessionStore]);

  useEffect(() => {
    if (!sessionId) {
      return;
    }

    const activeSession = sessions.find((session) => session.id === sessionId);
    if (!activeSession) {
      return;
    }

    const nextTab = tabForSessionType(activeSession.session_type);
    setActiveTab((currentTab) =>
      currentTab === nextTab ? currentTab : nextTab,
    );
  }, [sessionId, sessions]);

  useEffect(() => {
    const nextNotifiedIds = new Set<string>();
    if (!sessionId) {
      notifiedApprovalIdsRef.current = nextNotifiedIds;
      return;
    }

    for (const toolCall of awaitingApprovalCalls) {
      nextNotifiedIds.add(toolCall.id);
      if (notifiedApprovalIdsRef.current.has(toolCall.id)) {
        continue;
      }

      void notifyToolApproval(toolCall.id, toolCall.name, sessionId);
      void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning);
    }

    notifiedApprovalIdsRef.current = nextNotifiedIds;
  }, [awaitingApprovalCalls, notifyToolApproval, sessionId]);

  useEffect(() => {
    const awaitingApproval =
      awaitingApprovalCalls[awaitingApprovalCalls.length - 1] ?? null;
    const shouldKeepActivity =
      Boolean(sessionId) && (isStreaming || Boolean(awaitingApproval));

    if (isStreaming && !previousStreamingRef.current) {
      suppressCompletionRef.current = false;
      currentStreamSessionIdRef.current = sessionId;
      streamStartedAtRef.current = Date.now();
    }

    if (shouldKeepActivity && !liveActivityOpenRef.current) {
      startActivity(sessionTitle || "Chat", model || "unknown");
      liveActivityOpenRef.current = true;
    }

    if (shouldKeepActivity) {
      updateActivity({
        chatTitle: sessionTitle || "Chat",
        model: model || "unknown",
        status: awaitingApproval
          ? "awaiting_approval"
          : isThinking
            ? "thinking"
            : activeToolCall
              ? "tool_call"
              : "streaming",
        currentText: isThinking
          ? "Thinking..."
          : (lastAssistantMessage?.content?.slice(-200) ?? ""),
        currentTool: awaitingApproval?.name || activeToolCall?.name || "",
        tokenCount,
        progress: awaitingApproval ? 0.85 : isStreaming ? 0.5 : 1,
        toolApprovalId: awaitingApproval?.id,
        toolApprovalName: awaitingApproval?.name,
        toolApprovalSessionId: awaitingApproval ? sessionId ?? undefined : undefined,
      });
    } else if (liveActivityOpenRef.current) {
      endActivity();
      liveActivityOpenRef.current = false;
    }

    if (
      previousStreamingRef.current &&
      !isStreaming &&
      !awaitingApproval &&
      !suppressCompletionRef.current &&
      currentStreamSessionIdRef.current &&
      currentStreamSessionIdRef.current === sessionId
    ) {
      const startedAt = streamStartedAtRef.current ?? Date.now();
      const elapsedSeconds = Math.max(
        0,
        Math.floor((Date.now() - startedAt) / 1000),
      );
      void notifyStreamComplete(
        currentStreamSessionIdRef.current,
        sessionTitle || "Chat",
        tokenCount,
        elapsedSeconds,
      );
      currentStreamSessionIdRef.current = null;
      streamStartedAtRef.current = null;
    }

    if (!shouldKeepActivity && !isStreaming) {
      suppressCompletionRef.current = false;
      currentStreamSessionIdRef.current = null;
      streamStartedAtRef.current = null;
    }

    previousStreamingRef.current = isStreaming;
  }, [
    activeToolCall,
    awaitingApprovalCalls,
    endActivity,
    isStreaming,
    isThinking,
    lastAssistantMessage,
    model,
    notifyStreamComplete,
    sessionId,
    sessionTitle,
    startActivity,
    tokenCount,
    updateActivity,
  ]);

  const stopCurrentStream = useCallback(
    (suppressCompletion = true) => {
      if (sessionStore.getState().isStreaming) {
        suppressCompletionRef.current = suppressCompletion;
        sessionStore.getState().stopStreaming();
      }
      setActiveToolCallId(null);
    },
    [sessionStore],
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
        sessionStore.getState().setModel(currentModel);
      }
      if (sessionStore.getState().thinkingLevel !== currentThinkingLevel) {
        sessionStore.getState().setThinkingLevel(currentThinkingLevel);
      }

      await sessionsStore.getState().loadSessions();
    },
    [sessionStore, sessionsStore, workspace],
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
    [activeTab, bootstrapSession, client, ensureModelReady, stopCurrentStream],
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
    [sessionStore, stopCurrentStream],
  );

  const loadSessionById = useCallback(
    async (id: string) => {
      stopCurrentStream();
      setDrawerOpen(false);
      setActiveTab(2);
      await sessionStore.getState().loadSession(id);
    },
    [sessionStore, stopCurrentStream],
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
    [sessionsStore, stopCurrentStream, sessionStore],
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
    [activeToolCallId, sessionStore],
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
    async (content: string, attachments: ChatBarAttachment[] = []) => {
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
          .sendMessage(
            trimmed,
            attachments as SessionAttachment[],
            researchEnabled,
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
    [client, ensureModelReady, ensureSessionForSend, researchEnabled, sessionStore],
  );

  const handleSessionToolApproval = useCallback(
    (targetSessionId: string, toolCallId: string, approved: boolean) => {
      void handleToolApprovalAction(targetSessionId, toolCallId, approved);
    },
    [handleToolApprovalAction],
  );

  const handleStop = useCallback(() => {
    stopCurrentStream();
  }, [stopCurrentStream]);

  const handleModelSelect = useCallback(
    (modelId: string) => {
      sessionStore.getState().setModel(modelId);
      void SecureStore.setItemAsync(SELECTED_MODEL_KEY, modelId);
    },
    [sessionStore],
  );

  const handleFastModeToggle = useCallback(() => {
    const currentModel = sessionStore.getState().model;
    const nextModel = toggleFastModeModel(currentModel);
    if (!nextModel || nextModel === currentModel) {
      return;
    }

    sessionStore.getState().setModel(nextModel);
    void SecureStore.setItemAsync(SELECTED_MODEL_KEY, nextModel);
  }, [sessionStore]);

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
    [sessionStore, sessions, stopCurrentStream],
  );

  const handleRenameSession = useCallback(() => {
    if (!sessionId || !sessionTitle) {
      return;
    }

    Alert.prompt(
      "Rename Session",
      undefined,
      async (newTitle?: string) => {
        const nextTitle = newTitle?.trim();
        if (!nextTitle) {
          return;
        }

        sessionStore.getState().setTitle(nextTitle);
        await sessionStore.getState().updateTitle(sessionId, nextTitle);
      },
      "plain-text",
      sessionTitle,
    );
  }, [sessionId, sessionStore, sessionTitle]);

  const chatContent = (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      <Animated.View style={[styles.topBar, entrance.topBarStyle]}>
        {!isDesktop && (
          <Pressable
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              setDrawerOpen(true);
            }}
            style={styles.menuBtn}
          >
            <Menu size={22} color={t.foreground} strokeWidth={1.8} />
          </Pressable>
        )}

        <Pressable
          onPress={handleRenameSession}
          style={styles.titleBtn}
          disabled={!sessionTitle}
        >
          <Text
            style={[
              styles.title,
              { color: sessionTitle ? t.foreground : "transparent" },
            ]}
            numberOfLines={1}
          >
            {sessionTitle || " "}
          </Text>
        </Pressable>

        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setReportsOpen(true);
          }}
          style={styles.menuBtn}
        >
          <FileSearch size={20} color={t.mutedForeground} strokeWidth={1.8} />
        </Pressable>
      </Animated.View>

      <Animated.View style={[styles.flex, entrance.contentStyle]}>
        <ChatTranscript
          messages={messages}
          sessionId={sessionId}
          isStreaming={isStreaming}
          isThinking={isThinking}
          activeToolCallId={activeToolCallId}
          onApproveTool={(targetSessionId, toolCallId) =>
            handleSessionToolApproval(targetSessionId, toolCallId, true)
          }
          onDenyTool={(targetSessionId, toolCallId) =>
            handleSessionToolApproval(targetSessionId, toolCallId, false)
          }
          onSubmitToolResult={(toolCallId, result) =>
            void handleInteractiveToolResult(toolCallId, result)
          }
          onPlanConfirm={(toolCallId, choice) =>
            void handlePlanConfirm(toolCallId, choice)
          }
          emptyState={
            <View style={styles.empty}>
              <KrustyLogo />
              {error ? (
                <Text style={[styles.emptyHint, { color: t.error }]}>
                  {error}
                </Text>
              ) : null}
            </View>
          }
          bottomPadding={composerReserveHeight}
        />
      </Animated.View>

      {messages.length > 0 && error ? (
        <View
          style={[
            styles.errorBanner,
            {
              borderColor: `${t.error}40`,
              backgroundColor: `${t.error}14`,
            },
          ]}
        >
          <Text style={[styles.errorBannerText, { color: t.error }]}>
            {error}
          </Text>
        </View>
      ) : null}

      <Animated.View style={[entrance.bottomBarStyle, { overflow: "visible" }]}>
        <ChatBar
          onSend={handleSend}
          onStop={handleStop}
          onHeightChange={setComposerReserveHeight}
          isStreaming={isStreaming}
          disabled={!isConnected}
          thinkingLevel={thinkingLevel as ThinkingLevel}
          onThinkingChange={(level) =>
            sessionStore.getState().setThinkingLevel(level)
          }
          permissionMode={permissionMode as PermissionMode}
          onPermissionModeToggle={() =>
            sessionStore.getState().togglePermissionMode()
          }
          fastModeEnabled={fastModeEnabled}
          fastModeSupported={fastModeSupported}
          onFastModeToggle={handleFastModeToggle}
          mode={mode}
          onModeToggle={() =>
            sessionStore.getState().setMode(mode === "build" ? "plan" : "build")
          }
          onModelSelect={handleModelSelect}
          model={model}
          models={models}
          sessionType={sessionTypeForTab(activeTab)}
          researchEnabled={researchEnabled}
          onResearchToggle={() => setResearchEnabled((current) => !current)}
          tokenCount={tokenCount}
        />
      </Animated.View>

      <ReportsViewer
        visible={reportsOpen}
        onClose={() => setReportsOpen(false)}
      />
    </SafeAreaView>
  );

  const makoContent = (
    <Animated.View style={[styles.flex, entrance.contentStyle]}>
      <MakoScreen
        workspaceDirectory={workspaceDirectory}
        activeRunId={sessionId}
        onOpenRunById={loadSessionById}
        onDeleteRun={handleDeleteSession}
        onOpenMenu={() => {
          void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          setDrawerOpen(true);
        }}
        chat={{
          sessionId,
          title: sessionTitle,
          messages,
          error,
          isLoading,
          isStreaming,
          isThinking,
          activeToolCallId,
          thinkingLevel: thinkingLevel as ThinkingLevel,
          permissionMode: permissionMode as PermissionMode,
          fastModeEnabled,
          fastModeSupported,
          mode,
          model,
          models,
          researchEnabled,
          tokenCount,
          onApproveTool: (targetSessionId, toolCallId) =>
            handleSessionToolApproval(targetSessionId, toolCallId, true),
          onDenyTool: (targetSessionId, toolCallId) =>
            handleSessionToolApproval(targetSessionId, toolCallId, false),
          onSubmitToolResult: (toolCallId, result) =>
            void handleInteractiveToolResult(toolCallId, result),
          onPlanConfirm: (toolCallId, choice) =>
            void handlePlanConfirm(toolCallId, choice),
          onSend: handleSend,
          onStop: handleStop,
          onThinkingChange: (level) =>
            sessionStore.getState().setThinkingLevel(level),
          onPermissionModeToggle: () =>
            sessionStore.getState().togglePermissionMode(),
          onFastModeToggle: handleFastModeToggle,
          onModeToggle: () =>
            sessionStore.getState().setMode(mode === "build" ? "plan" : "build"),
          onModelSelect: handleModelSelect,
          onResearchToggle: () => setResearchEnabled((current) => !current),
        }}
      />
    </Animated.View>
  );

  return (
    <DesktopShell
      sessions={sessions}
      activeSessionId={sessionId}
      onSelectSession={(session) => void loadSession(session)}
      onNewSession={() => void handleNewSession()}
      onNewSessionWithDir={(path) => void handleDirectorySelected(path)}
      onDeleteSession={handleDeleteSession}
      onOpenSettings={() => router.push("/(tabs)/settings")}
      activeTab={activeTab}
      onTabChange={handleTabChange}
    >
      {activeTab === 2 ? makoContent : chatContent}

      {!isDesktop && (
        <SessionDrawer
          isOpen={drawerOpen}
          onClose={() => setDrawerOpen(false)}
          sessions={sessions}
          activeSessionId={sessionId}
          onSelectSession={(session) => void loadSession(session)}
          onNewSession={() => void handleNewSession()}
          onNewSessionWithDir={(path) => void handleDirectorySelected(path)}
          onDeleteSession={handleDeleteSession}
          onOpenSettings={() => {
            setDrawerOpen(false);
            router.push("/(tabs)/settings");
          }}
          activeTab={activeTab}
          onTabChange={handleTabChange}
        />
      )}
    </DesktopShell>
  );
}

const styles = StyleSheet.create({
  bootScreen: { flex: 1 },
  bootInner: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingHorizontal: 28,
  },
  bootSpinner: {
    marginTop: 20,
  },
  bootActions: {
    marginTop: 24,
    alignItems: "center",
    gap: 12,
    width: "100%",
    maxWidth: 320,
  },
  bootMessage: {
    marginTop: 14,
    fontSize: 15,
    lineHeight: 21,
    textAlign: "center",
  },
  bootButton: {
    marginTop: 4,
    borderRadius: 16,
    paddingVertical: 14,
    paddingHorizontal: 18,
    width: "100%",
    alignItems: "center",
  },
  bootButtonText: {
    color: "#fff",
    fontSize: 16,
    fontWeight: "600",
  },
  bootButtonSecondary: {
    borderRadius: 16,
    borderWidth: StyleSheet.hairlineWidth,
    paddingVertical: 14,
    paddingHorizontal: 18,
    width: "100%",
    alignItems: "center",
  },
  bootButtonSecondaryText: {
    fontSize: 16,
    fontWeight: "600",
  },
  container: { flex: 1 },
  flex: { flex: 1 },
  topBar: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 16,
    paddingVertical: 10,
    gap: 12,
  },
  menuBtn: {
    padding: 4,
  },
  titleBtn: {
    flex: 1,
  },
  title: {
    fontSize: 17,
    fontWeight: "600",
    textAlign: "center",
  },
  statusDot: {
    width: 8,
    height: 8,
    borderRadius: 4,
  },
  list: {
    paddingHorizontal: 16,
    paddingTop: 8,
  },
  listDesktop: {
    maxWidth: 800,
    alignSelf: "center",
    width: "100%",
  },
  fadeTop: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 64,
  },
  fadeBottom: {
    position: "absolute",
    bottom: 0,
    left: 0,
    right: 0,
    height: 120,
  },
  empty: {
    flex: 1,
    justifyContent: "flex-start",
    alignItems: "center",
    paddingTop: "35%",
    gap: 16,
  },
  emptyTitle: {
    fontSize: 28,
    fontWeight: "700",
    letterSpacing: -0.5,
  },
  emptyHint: {
    fontSize: 17,
  },
  errorBanner: {
    marginHorizontal: 16,
    marginBottom: 10,
    borderWidth: 1,
    borderRadius: 14,
    paddingHorizontal: 14,
    paddingVertical: 12,
  },
  errorBannerText: {
    fontSize: 13,
    lineHeight: 18,
    fontWeight: "500",
  },
  stubTitle: {
    fontSize: 24,
    fontWeight: "700",
    letterSpacing: -0.3,
  },
  stubText: {
    fontSize: 15,
    marginTop: 8,
  },
  modalBackdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: "rgba(0,0,0,0.6)",
    justifyContent: "flex-end",
    zIndex: 200,
  },
  modelPicker: {
    borderTopLeftRadius: 20,
    borderTopRightRadius: 20,
    maxHeight: "60%",
    paddingTop: 20,
    paddingBottom: 40,
    backgroundColor: "#1a1f2e",
    borderTopWidth: StyleSheet.hairlineWidth,
    borderTopColor: "rgba(255,255,255,0.1)",
  },
  modelPickerTitle: {
    fontSize: 18,
    fontWeight: "700",
    textAlign: "center",
    marginBottom: 16,
  },
  modelList: {
    paddingHorizontal: 16,
  },
  modelItem: {
    paddingHorizontal: 16,
    paddingVertical: 14,
    borderRadius: 12,
    borderWidth: 1,
    marginBottom: 8,
  },
  modelName: {
    fontSize: 16,
    fontWeight: "500",
  },
  modelProvider: {
    fontSize: 13,
    marginTop: 2,
  },
});
