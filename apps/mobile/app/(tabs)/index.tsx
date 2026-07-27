import { useState, useRef, useCallback, useEffect, useMemo } from "react";
import {
  AppState,
  View,
  Text,
  Pressable,
  Alert,
  Modal,
  TextInput,
  useWindowDimensions,
} from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import { SafeAreaView } from "react-native-safe-area-context";
import { router, useLocalSearchParams } from "expo-router";
import { Toolbox } from "lucide-react-native";
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
import { MakoSharkIcon } from "../../components/ui/MakoSharkIcon";
import {
  ChatBar,
  type Attachment as ChatBarAttachment,
} from "../../components/chat/ChatBar";
import { SessionDrawer } from "../../components/chat/SessionDrawer";
import { DesktopShell } from "../../components/layout/DesktopShell";
import { ToolboxPanel } from "../../components/ToolboxPanel";
import { MakoScreen } from "../../components/mako/MakoScreen";
import { MakoThreadSurface } from "../../components/mako/MakoThreadSurface";
import { MobileAppHeader } from "../../components/navigation/MobileAppHeader";
import { modeForHorizontalSwipe } from "../../components/navigation/modeSwipe";
import { displayThreadTitle } from "../../components/navigation/threadTitle";
import { useSplashState } from "../../hooks/useSplashState";
import { useEntranceAnimation } from "../../hooks/useEntranceAnimation";
import { useLiveActivity } from "../../hooks/useLiveActivity";
import { useWidgetSync } from "../../hooks/useWidgetSync";
import { useNotifications } from "../../hooks/useNotifications";
import { getToolDiffStats } from "../../components/chat/toolDiffModel";
import Animated, {
  runOnJS,
  SlideInLeft,
  SlideInRight,
  SlideOutLeft,
  SlideOutRight,
} from "react-native-reanimated";

import type {
  ModelInfo,
  ModelKey,
  SessionResponse,
  SessionType,
} from "@krusty/api";
import type { MakoTopLevelView } from "../../components/mako/types";
import type { MakoChatContext } from "../../components/mako/types";
import type {
  Attachment as SessionAttachment,
  PermissionMode,
  ThinkingLevel,
} from "@krusty/state";
import {
  modelKeysEqual,
  resolveUsableModel,
  supportsFastMode,
} from "@krusty/state";

import { ChatBootScreen } from "./chat-screen/BootScreen";
import {
  CHAT_BAR_ZONE,
  SELECTED_MODEL_KEY,
  flattenToolCalls,
  getActiveToolCall,
  getLastAssistantMessage,
  normalizeProviderId,
  sessionTypeForTab,
  tabForSessionType,
} from "./chat-screen/helpers";
import {
  DESKTOP_CHAT_MAX_WIDTH,
  TOOLBOX_DOCK_WIDTH,
  resolveDesktopChatMaxWidth,
  styles,
} from "./chat-screen/styles";
import { useSessionActions } from "./chat-screen/useSessionActions";

type LoadedStores = NonNullable<ReturnType<typeof useStores>>;
type MobileSheet = "threads" | "toolbox" | null;

export default function ChatScreen() {
  const {
    status,
    error: connectionError,
    reconnect,
    isConfigured,
  } = useConnection();
  const stores = useStores();
  const { splashDone } = useSplashState();

  if (!stores) {
    return (
      <ChatBootScreen
        status={status}
        isConfigured={isConfigured}
        connectionError={connectionError}
        showLogo={!splashDone}
        onRetryConnection={() => {
          void reconnect();
        }}
        onOpenSetup={() => {
          router.replace("/onboarding");
        }}
      />
    );
  }

  return <ChatScreenContent stores={stores} />;
}

function ChatScreenContent({ stores }: { stores: LoadedStores }) {
  const routeParams = useLocalSearchParams<{
    sessionId?: string | string[];
    focus?: string | string[];
    messageId?: string | string[];
    reportId?: string | string[];
  }>();
  const { theme } = useThemeContext();
  const { client, isConnected } = useConnection();
  const { isDesktop } = useBreakpoint();
  const { width: windowWidth } = useWindowDimensions();
  const { splashDone } = useSplashState();
  const entrance = useEntranceAnimation(splashDone);

  const [activeMode, setActiveMode] = useState<SessionType>("chat");
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameSaving, setRenameSaving] = useState(false);
  const [modeTransitionDirection, setModeTransitionDirection] =
    useState<1 | -1>(1);
  const activeTab = tabForSessionType(activeMode);
  const setActiveTab = useCallback((index: number) => {
    setActiveMode(sessionTypeForTab(index));
  }, []);
  const {
    session: sessionStore,
    workspace,
  } = stores.modes[activeMode];
  const { sessions: sessionsStore } = stores;

  const sessions = useSessionsStore(
    (state) => state.sessions,
  ) as SessionResponse[];
  const sessionId =
    useSessionStore((state) => state.sessionId, activeMode) ?? null;
  const sessionTitle =
    useSessionStore((state) => state.title, activeMode) ?? null;
  const messages =
    useSessionStore((state) => state.messages, activeMode) ?? [];
  const isStreaming =
    useSessionStore((state) => state.isStreaming, activeMode) ?? false;
  const isThinking =
    useSessionStore((state) => state.isThinking, activeMode) ?? false;
  const model = useSessionStore((state) => state.model, activeMode) ?? null;
  const modelKey = useSessionStore((state) => state.modelKey, activeMode) ?? null;
  const thinkingLevel =
    useSessionStore((state) => state.thinkingLevel, activeMode) ?? "medium";
  const permissionMode =
    useSessionStore((state) => state.permissionMode, activeMode) ?? "autonomous";
  const fastModeStoreEnabled =
    useSessionStore((state) => state.fastModeEnabled, activeMode) ?? false;
  const mode =
    useSessionStore((state) => state.mode, activeMode) ?? "build";
  const tokenCount =
    useSessionStore((state) => state.tokenCount, activeMode) ?? 0;
  const error = useSessionStore((state) => state.error, activeMode) ?? null;
  const isLoading =
    useSessionStore((state) => state.isLoading, activeMode) ?? false;
  const workspaceDirectory =
    useWorkspaceStore((state) => state.directory, activeMode) ?? null;
  const workspaceTargetBranch =
    useWorkspaceStore((state) => state.targetBranch, activeMode) ?? null;
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [defaultModelId, setDefaultModelId] = useState<string | null>(null);
  const [defaultModelKey, setDefaultModelKey] = useState<ModelKey | null>(null);
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([]);
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);
  const [activeSheet, setActiveSheet] = useState<MobileSheet>(null);
  const [desktopToolboxOpen, setDesktopToolboxOpen] = useState(false);
  const drawerOpen = activeSheet === "threads";
  const setDrawerOpen = useCallback((open: boolean) => {
    setActiveSheet(open ? "threads" : null);
  }, []);
  const [makoTopLevel, setMakoTopLevel] = useState<MakoTopLevelView>("mako");
  const [makoNotificationTarget, setMakoNotificationTarget] = useState<{
    messageId?: string;
    reportId?: string;
  } | null>(null);
  const toolboxOpen = isDesktop
    ? desktopToolboxOpen
    : activeSheet === "toolbox";
  const [toolboxTabByMode, setToolboxTabByMode] = useState<
    Record<SessionType, number>
  >({
    chat: 0,
    code: 0,
    mako: 0,
  });
  const toolboxTab = toolboxTabByMode[activeMode];
  const setToolboxTab = useCallback(
    (tab: number) => {
      setToolboxTabByMode((current) => ({
        ...current,
        [activeMode]: tab,
      }));
    },
    [activeMode],
  );
  const [bottomControlsOpen, setBottomControlsOpen] = useState(false);
  const [composerReserveHeight, setComposerReserveHeight] =
    useState(CHAT_BAR_ZONE);
  const [errorBannerHeight, setErrorBannerHeight] = useState(0);
  /** Measured desktop chat pane width (split host, before soft-cap). */
  const [desktopPaneWidth, setDesktopPaneWidth] = useState(0);
  const selectedModelInfo = useMemo(
    () =>
      (modelKey
        ? models.find((candidate) => modelKeysEqual(candidate.key, modelKey))
        : models.find((candidate) => candidate.id === model)) ?? null,
    [model, modelKey, models],
  );
  const fastModeSupported = supportsFastMode(
    selectedModelInfo ?? model,
    selectedModelInfo?.provider ?? null,
  );
  const fastModeEnabled = fastModeSupported && fastModeStoreEnabled;

  useEffect(() => {
    if (!model || !selectedModelInfo) {
      return;
    }
    sessionStore.getState().setModel(model, selectedModelInfo.provider, selectedModelInfo);
  }, [model, selectedModelInfo, sessionStore]);

  const previousStreamingRef = useRef(false);
  const currentStreamSessionIdRef = useRef<string | null>(null);
  const streamStartedAtRef = useRef<number | null>(null);
  const liveActivitySessionIdRef = useRef<string | null>(null);
  const notifiedApprovalIdsRef = useRef<Set<string>>(new Set());
  const suppressCompletionRef = useRef(false);
  const sessionsRefreshInFlightRef = useRef(false);
  const toolActivityRef = useRef<{
    toolCalls: ReturnType<typeof flattenToolCalls>;
    awaitingApprovalCalls: ReturnType<typeof flattenToolCalls>;
    activeToolCall: ReturnType<typeof getActiveToolCall>;
    activityDiff: { additions: number; deletions: number };
  } | null>(null);
  const lastSessionIdByTypeRef = useRef<Record<SessionType, string | null>>({
    chat: null,
    code: null,
    mako: null,
  });
  const attemptedWorkspaceSessionHydrationRef = useRef<
    Record<SessionType, string | null>
  >({
    chat: null,
    code: null,
    mako: null,
  });
  const lastAssistantMessage = useMemo(
    () => getLastAssistantMessage(messages),
    [messages],
  );
  const toolActivity = useMemo(() => {
    const toolCalls = flattenToolCalls(messages);
    const previous = toolActivityRef.current;
    const unchanged =
      previous?.toolCalls.length === toolCalls.length &&
      toolCalls.every((toolCall, index) => previous.toolCalls[index] === toolCall);
    if (unchanged && previous) return previous;

    const next = {
      toolCalls,
      awaitingApprovalCalls: toolCalls.filter(
        (toolCall) => toolCall.status === "awaiting_approval",
      ),
      activeToolCall: getActiveToolCall(toolCalls),
      activityDiff: toolCalls.reduce(
        (total, toolCall) => {
          const stats = getToolDiffStats(toolCall);
          if (stats) {
            total.additions += stats.additions;
            total.deletions += stats.deletions;
          }
          return total;
        },
        { additions: 0, deletions: 0 },
      ),
    };
    toolActivityRef.current = next;
    return next;
  }, [messages]);
  const {
    toolCalls,
    awaitingApprovalCalls,
    activeToolCall,
    activityDiff,
  } = toolActivity;

  const lastAssistantSnippet =
    lastAssistantMessage?.content?.slice(0, 200) ?? "";
  const showTranscriptError = Boolean(error && messages.length > 0);

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
      const targetSession = targetSessionId
        ? sessions.find((candidate) => candidate.id === targetSessionId)
        : null;
      const targetType =
        targetSession?.session_type ?? (focus === "mako" ? "mako" : activeMode);

      if (focus === "mako") {
        setMakoTopLevel("mako");
        setMakoNotificationTarget({
          ...(params?.messageId ? { messageId: params.messageId } : {}),
          ...(params?.reportId ? { reportId: params.reportId } : {}),
        });
      }
      setActiveTab(tabForSessionType(targetType));
      if (!targetSessionId) {
        if (focus === "mako") {
          await stores.modes.mako.session.getState().ensureMakoMainSession();
        }
        return;
      }

      try {
        await stores.modes[targetType].session
          .getState()
          .loadSession(targetSessionId, true);
      } catch {
        void sessionsStore.getState().loadSessions();
      }
    },
    [activeMode, sessions, sessionsStore, setActiveTab, stores.modes],
  );

  useEffect(() => {
    const targetSessionId = Array.isArray(routeParams.sessionId)
      ? routeParams.sessionId[0]
      : routeParams.sessionId;
    const focus = Array.isArray(routeParams.focus)
      ? routeParams.focus[0]
      : routeParams.focus;
    const messageId = Array.isArray(routeParams.messageId)
      ? routeParams.messageId[0]
      : routeParams.messageId;
    const reportId = Array.isArray(routeParams.reportId)
      ? routeParams.reportId[0]
      : routeParams.reportId;
    if ((targetSessionId && targetSessionId !== sessionId) || focus === "mako") {
      void handleNotificationNavigate("/(tabs)", {
        ...(targetSessionId ? { sessionId: targetSessionId } : {}),
        ...(focus ? { focus } : {}),
        ...(messageId ? { messageId } : {}),
        ...(reportId ? { reportId } : {}),
      });
    }
  }, [
    handleNotificationNavigate,
    routeParams.focus,
    routeParams.messageId,
    routeParams.reportId,
    routeParams.sessionId,
    sessionId,
  ]);

  const {
    notificationLevel,
    notifyToolApproval,
    notifyStreamComplete,
    submitToolApprovalAction,
  } = useNotifications();
  const handleLiveActivityToolApproval = useCallback(
    (targetSessionId: string, toolCallId: string, approved: boolean) => {
      void submitToolApprovalAction(targetSessionId, toolCallId, approved);
    },
    [submitToolApprovalAction],
  );
  const { startActivity, updateActivity, endActivity } = useLiveActivity({
    onToolApproval: handleLiveActivityToolApproval,
  });

  useWidgetSync({
    sessionId,
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
    setDefaultModelKey(response.default_model_key ?? null);
    setConfiguredProviders(nextConfiguredProviders);
    return {
      response,
      configuredProviders: nextConfiguredProviders,
    };
  }, [client, isConnected]);

  const ensureModelReady = useCallback(async (
    targetStore: LoadedStores["session"] = sessionStore,
  ) => {
    const existingModel = targetStore.getState().model;
    let catalog = models;
    let fallbackDefault = defaultModelId;
    let fallbackDefaultKey = defaultModelKey;
    let allowedProviders = configuredProviders;

    if (catalog.length === 0) {
      const result = await loadModelCatalog().catch(() => null);
      if (!result) {
        return null;
      }
      catalog = result.response.models;
      fallbackDefault = result.response.default_model ?? null;
      fallbackDefaultKey = result.response.default_model_key ?? null;
      allowedProviders = result.configuredProviders;
    }

    const selectedModel = resolveUsableModel(
      existingModel,
      fallbackDefault,
      catalog,
      allowedProviders,
      targetStore.getState().modelKey,
      fallbackDefaultKey,
    );

    if (selectedModel) {
      targetStore
        .getState()
        .setModel(selectedModel.id, selectedModel.provider ?? null, selectedModel);
      await SecureStore.setItemAsync(SELECTED_MODEL_KEY, selectedModel.id);
      return selectedModel.id;
    }

    targetStore.getState().setModel(null);
    await SecureStore.deleteItemAsync(SELECTED_MODEL_KEY).catch(() => {});
    return null;
  }, [
    configuredProviders,
    defaultModelId,
    defaultModelKey,
    loadModelCatalog,
    models,
  ]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    void sessionsStore.getState().loadSessions();
    for (const type of ["chat", "code", "mako"] as const) {
      void ensureModelReady(stores.modes[type].session);
    }
  }, [client, ensureModelReady, isConnected, sessionsStore, stores.modes]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    const refreshHandle = setInterval(() => {
      if (AppState.currentState === "active") {
        void loadModelCatalog().catch(() => null);
      }
    }, 5 * 60 * 1000);

    return () => clearInterval(refreshHandle);
  }, [client, isConnected, loadModelCatalog]);

  useEffect(() => {
    if (!client || !isConnected || sessions.length === 0) {
      return;
    }

    for (const type of ["chat", "code", "mako"] as const) {
      const slot = stores.modes[type];
      if (slot.session.getState().sessionId) {
        continue;
      }
      const persistedId = slot.workspace.getState().sessionId;
      const persisted = persistedId
        ? sessions.find(
            (candidate) =>
              candidate.id === persistedId && candidate.session_type === type,
          )
        : null;
      const recent = sessions
        .filter((candidate) => candidate.session_type === type)
        .sort(
          (left, right) =>
            new Date(right.updated_at).getTime() -
            new Date(left.updated_at).getTime(),
        )[0];
      const targetId = persisted?.id ?? recent?.id ?? null;
      if (
        !targetId ||
        attemptedWorkspaceSessionHydrationRef.current[type] === targetId
      ) {
        continue;
      }

      attemptedWorkspaceSessionHydrationRef.current[type] = targetId;
      lastSessionIdByTypeRef.current[type] = targetId;
      void slot.session.getState().loadSession(targetId, true).catch(() => {
        void sessionsStore.getState().loadSessions();
      });
    }
  }, [client, isConnected, sessions, sessionsStore, stores.modes]);

  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    const refreshHandle = setInterval(() => {
      if (
        AppState.currentState !== "active" ||
        sessionStore.getState().isStreaming ||
        sessionsRefreshInFlightRef.current
      ) return;

      sessionsRefreshInFlightRef.current = true;
      void sessionsStore.getState().loadSessions().finally(() => {
        sessionsRefreshInFlightRef.current = false;
      });
    }, 30_000);

    return () => clearInterval(refreshHandle);
  }, [client, isConnected, sessionStore, sessionsStore]);

  useEffect(() => {
    const subscription = AppState.addEventListener("change", (nextState) => {
      if (nextState !== "active") {
        return;
      }

      // Refresh only the active mode on resume. Background modes warm lazily
      // when the user switches to them, which avoids a resume network storm.
      const activeSlot = stores.modes[activeMode];
      const currentSessionId =
        activeSlot.session.getState().sessionId ??
        activeSlot.workspace.getState().sessionId;
      if (
        currentSessionId &&
        !activeSlot.session.getState().isStreaming
      ) {
        void activeSlot.session.getState().loadSession(currentSessionId, true);
      }
      void loadModelCatalog().catch(() => null);
    });

    return () => subscription.remove();
  }, [activeMode, loadModelCatalog, stores.modes]);

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
      if (notificationLevel !== "silent") {
        void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning);
      }
    }

    notifiedApprovalIdsRef.current = nextNotifiedIds;
  }, [awaitingApprovalCalls, notificationLevel, notifyToolApproval, sessionId]);

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

    if (
      shouldKeepActivity &&
      liveActivitySessionIdRef.current !== sessionId
    ) {
      startActivity(sessionId!, sessionTitle || "Chat");
      liveActivitySessionIdRef.current = sessionId;
    }

    if (shouldKeepActivity) {
      updateActivity({
        chatTitle: sessionTitle || "Chat",
        status: awaitingApproval ? "needs_input" : "working",
        toolCount: toolCalls.length,
        filesAdded: activityDiff.additions,
        filesRemoved: activityDiff.deletions,
        toolApprovalId: awaitingApproval?.id,
        toolApprovalName: awaitingApproval?.name,
        toolApprovalSessionId: awaitingApproval ? sessionId ?? undefined : undefined,
      });
    } else if (liveActivitySessionIdRef.current === sessionId) {
      endActivity();
      liveActivitySessionIdRef.current = null;
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
    activityDiff.additions,
    activityDiff.deletions,
    awaitingApprovalCalls,
    endActivity,
    isStreaming,
    notifyStreamComplete,
    sessionId,
    sessionTitle,
    startActivity,
    tokenCount,
    toolCalls.length,
    updateActivity,
  ]);

  const {
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
  } = useSessionActions({
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
    modeStores: stores.modes,
    sessions,
    models,
    suppressCompletionRef,
    lastSessionIdByTypeRef,
  });

  const handleSessionToolApproval = useCallback(
    (targetSessionId: string, toolCallId: string, approved: boolean) => {
      void handleToolApprovalAction(targetSessionId, toolCallId, approved);
    },
    [handleToolApprovalAction],
  );
  const handleApproveTranscriptTool = useCallback(
    (targetSessionId: string, toolCallId: string) => {
      handleSessionToolApproval(targetSessionId, toolCallId, true);
    },
    [handleSessionToolApproval],
  );
  const handleDenyTranscriptTool = useCallback(
    (targetSessionId: string, toolCallId: string) => {
      handleSessionToolApproval(targetSessionId, toolCallId, false);
    },
    [handleSessionToolApproval],
  );
  const handleSubmitTranscriptTool = useCallback(
    (toolCallId: string, result: string) => {
      void handleInteractiveToolResult(toolCallId, result);
    },
    [handleInteractiveToolResult],
  );
  const handleTranscriptPlanConfirm = useCallback(
    (toolCallId: string, choice: "execute" | "abandon") => {
      void handlePlanConfirm(toolCallId, choice);
    },
    [handlePlanConfirm],
  );

  const handleStop = useCallback(() => {
    stopCurrentStream();
  }, [stopCurrentStream]);

  const handleChatBarSend = useCallback(
    async (content: string, attachments: ChatBarAttachment[] = []) => {
      const normalizedAttachments = attachments.map((attachment) => ({
        ...attachment,
        mimeType: attachment.mimeType ?? "application/octet-stream",
      })) as SessionAttachment[];

      if (activeMode === "mako" && !sessionStore.getState().sessionId) {
        const task = content.trim();
        if (!client || !task) {
          return;
        }
        if (normalizedAttachments.length > 0) {
          sessionStore.setState({
            error: "Start the Mako thread with text, then attach files in the conversation.",
          });
          return;
        }
        const resolvedModel = await ensureModelReady();
        if (!resolvedModel) {
          sessionStore.setState({
            error: "Choose an available model before starting Mako.",
          });
          return;
        }
        try {
          const response = await client.dispatchMako(task, {
            projectDir: workspaceDirectory ?? undefined,
            model: resolvedModel,
          });
          lastSessionIdByTypeRef.current.mako = response.session_id;
          await sessionsStore.getState().loadSessions();
          await sessionStore.getState().loadSession(response.session_id, true);
          void Haptics.notificationAsync(
            Haptics.NotificationFeedbackType.Success,
          );
        } catch (dispatchError) {
          sessionStore.setState({
            error:
              dispatchError instanceof Error
                ? dispatchError.message
                : "Failed to start the Mako thread.",
          });
        }
        return;
      }

      await handleSend(content, normalizedAttachments);
    },
    [
      activeMode,
      client,
      ensureModelReady,
      handleSend,
      sessionStore,
      sessionsStore,
      workspaceDirectory,
    ],
  );

  const handleNewMakoSession = useCallback(() => {
    setActiveMode("mako");
    setActiveSheet(null);
    const makoStore = stores.modes.mako.session;
    makoStore.getState().detachSession();
    makoStore.getState().clearSession();
  }, [stores.modes]);

  const handleSelectMakoView = useCallback(
    (view: MakoTopLevelView) => {
      handleTabChange(2);
      setMakoTopLevel(view);
      setDrawerOpen(false);
    },
    [handleTabChange],
  );

  const handleRenameSession = useCallback(() => {
    if (!sessionId) {
      return;
    }
    setRenameDraft((sessionTitle || "").trim() || "Untitled");
    setRenameOpen(true);
  }, [sessionId, sessionTitle]);

  const handleRenameCancel = useCallback(() => {
    if (renameSaving) return;
    setRenameOpen(false);
  }, [renameSaving]);

  const handleRenameSave = useCallback(async () => {
    if (!sessionId || renameSaving) return;
    const nextTitle = renameDraft.trim();
    const currentTitle = (sessionTitle || "").trim();
    if (!nextTitle) {
      Alert.alert("Title required", "Enter a session title to continue.");
      return;
    }
    if (nextTitle === currentTitle) {
      setRenameOpen(false);
      return;
    }

    setRenameSaving(true);
    try {
      sessionStore.getState().setTitle(nextTitle);
      await sessionStore.getState().updateTitle(sessionId, nextTitle);
      setRenameOpen(false);
    } catch (error) {
      Alert.alert(
        "Rename failed",
        error instanceof Error ? error.message : "Could not update session title.",
      );
    } finally {
      setRenameSaving(false);
    }
  }, [renameDraft, renameSaving, sessionId, sessionStore, sessionTitle]);

  const handleToolboxClose = useCallback(() => {
    if (isDesktop) {
      setDesktopToolboxOpen(false);
    } else {
      setActiveSheet(null);
    }
  }, [isDesktop]);

  // Prefer measured host; fall back to window so the band never full-bleeds.
  const effectivePaneWidth = useMemo(() => {
    if (desktopPaneWidth > 0) return desktopPaneWidth;
    if (!isDesktop) return windowWidth;
    const toolboxSlice = toolboxOpen ? TOOLBOX_DOCK_WIDTH : 0;
    return Math.max(0, windowWidth - toolboxSlice);
  }, [desktopPaneWidth, isDesktop, toolboxOpen, windowWidth]);

  const desktopChatMaxWidth = resolveDesktopChatMaxWidth(effectivePaneWidth);

  // Empty and placeholder sessions stay visually quiet. A real persisted title
  // appears only after the conversation has one.
  const displayTitle = displayThreadTitle(sessionTitle);

  const handleModeChange = useCallback(
    (mode: SessionType) => {
      if (mode === activeMode) {
        return;
      }
      const order: SessionType[] = ["chat", "code", "mako"];
      setModeTransitionDirection(
        order.indexOf(mode) > order.indexOf(activeMode) ? 1 : -1,
      );
      setActiveSheet(null);
      void handleTabChange(tabForSessionType(mode));
    },
    [activeMode, handleTabChange],
  );
  const handleModeSwipe = useCallback(
    (translationX: number, velocityX: number) => {
      const nextMode = modeForHorizontalSwipe(
        activeMode,
        translationX,
        velocityX,
      );
      if (!nextMode) {
        return;
      }
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      handleModeChange(nextMode);
    },
    [activeMode, handleModeChange],
  );
  const modeSwipeGesture = useMemo(
    () =>
      Gesture.Pan()
        .enabled(
          !isDesktop &&
            !drawerOpen &&
            !toolboxOpen &&
            !bottomControlsOpen,
        )
        .activeOffsetX([-28, 28])
        .failOffsetY([-20, 20])
        .onEnd((event) => {
          runOnJS(handleModeSwipe)(event.translationX, event.velocityX);
        }),
    [
      bottomControlsOpen,
      drawerOpen,
      handleModeSwipe,
      isDesktop,
      toolboxOpen,
    ],
  );

  const topBar = isDesktop ? (
    <Animated.View
      style={[styles.topBar, styles.topBarDesktop, entrance.topBarStyle]}
    >
      <Pressable
        onPress={handleRenameSession}
        style={[
          styles.titleBtn,
          displayTitle
            ? {
                borderColor: t.glass.border,
                backgroundColor: t.glass.background,
              }
            : { borderColor: "transparent", backgroundColor: "transparent" },
        ]}
        disabled={!sessionId || !displayTitle}
        accessibilityRole="button"
        accessibilityLabel="Rename thread"
      >
        <Text
          style={[
            styles.title,
            {
              color: displayTitle ? t.foreground : "transparent",
            },
          ]}
          numberOfLines={1}
        >
          {displayTitle || " "}
        </Text>
      </Pressable>

      <View style={styles.topBarActions}>
        <Pressable
          onPress={() => handleSelectMakoView("mako")}
          style={styles.toolboxCornerBtn}
          accessibilityRole="button"
          accessibilityLabel="Open Mako"
        >
          <MakoSharkIcon
            size={20}
            color={t.mutedForeground}
            strokeWidth={1.8}
          />
        </Pressable>
        <Pressable
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            setDesktopToolboxOpen((open) => !open);
          }}
          style={[
            styles.toolboxCornerBtn,
            toolboxOpen ? { backgroundColor: `${t.thinking}22` } : null,
          ]}
          accessibilityRole="button"
          accessibilityLabel={toolboxOpen ? "Close toolbox" : "Open toolbox"}
        >
          <Toolbox
            size={20}
            color={toolboxOpen ? t.thinking : t.mutedForeground}
            strokeWidth={1.8}
          />
        </Pressable>
      </View>
    </Animated.View>
  ) : (
    <Animated.View style={entrance.topBarStyle}>
      <MobileAppHeader
        mode={activeMode}
        title={displayTitle}
        onModeChange={handleModeChange}
        onOpenThreads={() => setActiveSheet("threads")}
        onOpenToolbox={() => setActiveSheet("toolbox")}
        onTitlePress={
          sessionId && displayTitle && activeMode !== "mako"
            ? handleRenameSession
            : undefined
        }
      />
    </Animated.View>
  );

  const chatTranscriptSurface = (
      <Animated.View style={[styles.flex, entrance.contentStyle]}>
        <ChatTranscript
          key={`${activeMode}:${sessionId ?? "new"}`}
          messages={messages}
          sessionId={sessionId}
          sessionType={activeMode}
          scrollStateKey={`${activeMode}:${sessionId ?? "new"}`}
          isStreaming={isStreaming}
          isThinking={isThinking}
          isLoading={isLoading}
          activeToolCallId={activeToolCallId}
          onApproveTool={handleApproveTranscriptTool}
          onDenyTool={handleDenyTranscriptTool}
          onSubmitToolResult={handleSubmitTranscriptTool}
          onPlanConfirm={handleTranscriptPlanConfirm}
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
          bottomPadding={
            composerReserveHeight
            + (showTranscriptError ? errorBannerHeight + 10 : 0)
          }
          hideJumpToLatest={bottomControlsOpen}
        />

        {showTranscriptError ? (
          <View
            accessibilityRole="alert"
            accessibilityLiveRegion="polite"
            onLayout={(event) => {
              const nextHeight = Math.ceil(event.nativeEvent.layout.height);
              setErrorBannerHeight((current) =>
                current === nextHeight ? current : nextHeight,
              );
            }}
            style={[
              styles.errorBanner,
              {
                position: "absolute",
                left: 0,
                right: 0,
                bottom: composerReserveHeight + 10,
                marginBottom: 0,
                zIndex: 30,
                borderColor: `${t.error}40`,
                backgroundColor: `${t.error}14`,
              },
            ]}
          >
            <Text
              selectable
              style={[styles.errorBannerText, { color: t.error }]}
            >
              {error}
            </Text>
          </View>
        ) : null}
      </Animated.View>
  );

  const sharedComposer = (
      <Animated.View
        style={[
          entrance.bottomBarStyle,
          { overflow: "visible", zIndex: 300 },
        ]}
      >
        <ChatBar
          draftKey={activeMode}
          onSend={handleChatBarSend}
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
            sessionStore
              .getState()
              .setMode(mode === "build" ? "plan" : "build")
          }
          onModelSelect={handleModelSelect}
          model={model}
          models={models}
          sessionType={sessionTypeForTab(activeTab)}
          workspaceDirectory={workspaceDirectory}
          targetBranch={workspaceTargetBranch}
          tokenCount={tokenCount}
          onOverlayOpenChange={setBottomControlsOpen}
          contentMaxWidth={isDesktop ? desktopChatMaxWidth : undefined}
        />
      </Animated.View>
  );

  const transcriptAndComposer = (
    <View style={styles.flex}>
      {chatTranscriptSurface}
      {sharedComposer}
    </View>
  );

  const chatMain = (
    <View style={styles.flex}>
      {isDesktop ? (
        <View
          style={styles.desktopChatColumnHost}
          onLayout={(event) => {
            const next = Math.round(event.nativeEvent.layout.width);
            setDesktopPaneWidth((prev) => (prev === next ? prev : next));
          }}
        >
          {/* Full-pane chrome: title + toolbox button in the true top-right. */}
          {topBar}
          {/* Messages + composer share a centered soft-capped band. */}
          <View
            style={[
              styles.desktopChatColumn,
              {
                maxWidth: desktopChatMaxWidth || DESKTOP_CHAT_MAX_WIDTH,
                width: "100%",
              },
            ]}
          >
            {transcriptAndComposer}
          </View>
        </View>
      ) : (
        <>
          {topBar}
          {transcriptAndComposer}
        </>
      )}
    </View>
  );

  const chatContent = (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={isDesktop ? [] : ["top"]}
    >
      {isDesktop ? (
        // Desktop: chat fills remaining width; toolbox is a fixed-width rail.
        <View style={styles.desktopSplit}>
          <View style={styles.desktopSplitChat}>{chatMain}</View>
          {toolboxOpen ? (
            <ToolboxPanel
              variant="dock"
              visible={toolboxOpen}
              onClose={handleToolboxClose}
              activeTab={toolboxTab}
              onTabChange={setToolboxTab}
              sessionType={activeMode}
              projectDirectory={workspaceDirectory}
              onOpenSettings={() => router.push("/(tabs)/settings")}
              onOpenMakoRun={(id) => void loadSessionById(id)}
              onOpenProject={(path, branch) =>
                void openProjectInCode(path, branch)
              }
            />
          ) : null}
        </View>
      ) : (
        <>
          {chatMain}
          <ToolboxPanel
            variant="overlay"
            visible={toolboxOpen}
            onClose={handleToolboxClose}
            activeTab={toolboxTab}
            onTabChange={setToolboxTab}
            sessionType={activeMode}
            projectDirectory={workspaceDirectory}
            onOpenSettings={() => {
              setActiveSheet(null);
              router.push("/(tabs)/settings");
            }}
            onOpenMakoRun={(id) => void loadSessionById(id)}
            onOpenProject={(path, branch) =>
              void openProjectInCode(path, branch)
            }
          />
        </>
      )}
    </SafeAreaView>
  );

  const makoChat: MakoChatContext = {
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
    tokenCount,
    onApproveTool: (targetSessionId, toolCallId) =>
      handleSessionToolApproval(targetSessionId, toolCallId, true),
    onDenyTool: (targetSessionId, toolCallId) =>
      handleSessionToolApproval(targetSessionId, toolCallId, false),
    onSubmitToolResult: (toolCallId, result) =>
      void handleInteractiveToolResult(toolCallId, result),
    onPlanConfirm: (toolCallId, choice) =>
      void handlePlanConfirm(toolCallId, choice),
    onSend: handleChatBarSend,
    onStop: handleStop,
    onThinkingChange: (level) =>
      sessionStore.getState().setThinkingLevel(level),
    onPermissionModeToggle: () =>
      sessionStore.getState().togglePermissionMode(),
    onFastModeToggle: handleFastModeToggle,
    onModeToggle: () =>
      sessionStore.getState().setMode(mode === "build" ? "plan" : "build"),
    onModelSelect: handleModelSelect,
  };

  const makoContent = isDesktop ? (
    <Animated.View style={[styles.flex, entrance.contentStyle]}>
      <MakoScreen
        workspaceDirectory={workspaceDirectory}
        requestedTopLevel={makoTopLevel}
        requestedThreadMessageId={makoNotificationTarget?.messageId}
        requestedReportId={makoNotificationTarget?.reportId}
        onOpenRunById={loadSessionById}
        onOpenProject={openProjectInCode}
        onDeleteRun={handleDeleteSession}
        onTopLevelChange={setMakoTopLevel}
        chat={makoChat}
      />
    </Animated.View>
  ) : (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={["top"]}
    >
      {topBar}
      <Animated.View style={[styles.flex, entrance.contentStyle]}>
        <MakoThreadSurface
          chat={makoChat}
        />
      </Animated.View>
      <ToolboxPanel
        variant="overlay"
        visible={toolboxOpen}
        onClose={handleToolboxClose}
        activeTab={toolboxTab}
        onTabChange={setToolboxTab}
        sessionType="mako"
        projectDirectory={workspaceDirectory}
        onOpenSettings={() => {
          setActiveSheet(null);
          router.push("/(tabs)/settings");
        }}
        onOpenMakoRun={(id) => void loadSessionById(id)}
        onOpenProject={(path, branch) =>
          void openProjectInCode(path, branch)
        }
      />
    </SafeAreaView>
  );

  const mobileConversationSurface = (
    <GestureDetector gesture={modeSwipeGesture}>
      <View
        style={[
          styles.flex,
          {
            overflow: "hidden",
            backgroundColor: t.background,
          },
        ]}
      >
        <Animated.View
          key={activeMode}
          entering={
            modeTransitionDirection > 0
              ? SlideInRight.duration(240)
              : SlideInLeft.duration(240)
          }
          exiting={
            modeTransitionDirection > 0
              ? SlideOutLeft.duration(210)
              : SlideOutRight.duration(210)
          }
          style={{
            position: "absolute",
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
            backgroundColor: t.background,
          }}
        >
          {activeMode === "mako" ? (
            <MakoThreadSurface
              chat={makoChat}
              showComposer={false}
              externalBottomPadding={composerReserveHeight}
            />
          ) : (
            chatTranscriptSurface
          )}
        </Animated.View>
      </View>
    </GestureDetector>
  );

  const mobileContent = (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={["top"]}
    >
      {topBar}
      {mobileConversationSurface}
      {sharedComposer}
      <ToolboxPanel
        variant="overlay"
        visible={toolboxOpen}
        onClose={handleToolboxClose}
        activeTab={toolboxTab}
        onTabChange={setToolboxTab}
        sessionType={activeMode}
        projectDirectory={workspaceDirectory}
        onOpenSettings={() => {
          setActiveSheet(null);
          router.push("/(tabs)/settings");
        }}
        onOpenMakoRun={(id) => void loadSessionById(id)}
        onOpenProject={(path, branch) =>
          void openProjectInCode(path, branch)
        }
      />
    </SafeAreaView>
  );

  const renameModal = (
    <Modal
      visible={renameOpen}
      transparent
      animationType="fade"
      onRequestClose={handleRenameCancel}
    >
      <Pressable style={styles.renameBackdrop} onPress={handleRenameCancel}>
        <Pressable
          style={[
            styles.renameCard,
            {
              backgroundColor: t.card,
              borderColor: t.border,
            },
          ]}
          onPress={(event) => event.stopPropagation()}
        >
          <Text style={[styles.renameTitle, { color: t.foreground }]}>Rename session</Text>
          <TextInput
            value={renameDraft}
            onChangeText={setRenameDraft}
            autoFocus
            editable={!renameSaving}
            placeholder="Session title"
            placeholderTextColor={t.mutedForeground}
            style={[
              styles.renameInput,
              {
                color: t.foreground,
                borderColor: t.border,
                backgroundColor: t.background,
              },
            ]}
            onSubmitEditing={() => {
              void handleRenameSave();
            }}
          />
          <View style={styles.renameActions}>
            <Pressable
              onPress={handleRenameCancel}
              disabled={renameSaving}
              style={[styles.renameButton, { borderColor: t.border }]}
            >
              <Text style={[styles.renameButtonText, { color: t.mutedForeground }]}>Cancel</Text>
            </Pressable>
            <Pressable
              onPress={() => {
                void handleRenameSave();
              }}
              disabled={renameSaving}
              style={[
                styles.renameButton,
                styles.renameButtonPrimary,
                { backgroundColor: t.userMessage },
              ]}
            >
              <Text style={[styles.renameButtonText, { color: "#fff" }]}>
                {renameSaving ? "Saving…" : "Save"}
              </Text>
            </Pressable>
          </View>
        </Pressable>
      </Pressable>
    </Modal>
  );

  return (
    <DesktopShell
      sessions={sessions}
      activeSessionId={sessionId}
      onSelectSession={(session) => void loadSession(session)}
      onNewSession={() => void handleNewSession("chat")}
      onNewSessionWithDir={(path) => void handleDirectorySelected(path)}
      onDeleteSession={handleDeleteSession}
      onOpenSettings={() => router.push("/(tabs)/settings")}
      activeTab={activeTab}
      onTabChange={handleTabChange}
      activeMakoView={makoTopLevel}
      onSelectMakoView={handleSelectMakoView}
    >
      {renameModal}
      {isDesktop ? (
        activeTab === 2 ? makoContent : chatContent
      ) : (
        mobileContent
      )}

      {!isDesktop && (
        <SessionDrawer
          isOpen={drawerOpen}
          onClose={() => setDrawerOpen(false)}
          sessions={sessions}
          activeSessionId={sessionId}
          onSelectSession={(session) => void loadSession(session)}
          onSelectMakoSession={(id) => void loadSessionById(id)}
          onNewSession={(type) => void handleNewSession(type)}
          onNewMakoSession={handleNewMakoSession}
          onNewSessionWithDir={(path) => void handleDirectorySelected(path)}
          onDeleteSession={handleDeleteSession}
          onOpenSettings={() => {
            setDrawerOpen(false);
            router.push("/(tabs)/settings");
          }}
          activeMode={activeMode}
        />
      )}
    </DesktopShell>
  );
}
