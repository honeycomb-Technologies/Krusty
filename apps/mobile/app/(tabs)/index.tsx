import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  ActivityIndicator,
  Alert,
  Modal,
  Pressable,
  Text,
  TextInput,
  useWindowDimensions,
  View,
} from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import { SafeAreaView } from "react-native-safe-area-context";
import { router, useLocalSearchParams } from "expo-router";
import { Toolbox } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import {
  useSessionsStore,
  useSessionStore,
  useStores,
  useWorkspaceStore,
} from "../../hooks/useStores";
import { useShallow } from "zustand/react/shallow";
import { HiveIcon } from "../../components/brand";
import {
  type Attachment as ChatBarAttachment,
  ChatBar,
} from "../../components/chat/ChatBar";
import { SessionDrawer } from "../../components/chat/SessionDrawer";
import { DesktopShell } from "../../components/layout/DesktopShell";
import { ToolboxPanel } from "../../components/ToolboxPanel";
import { HiveScreen } from "../../components/hive/HiveScreen";
import { HiveMobileThreadControls } from "../../components/hive/HiveMobileThreadControls";
import {
  assertCurrentHiveWorkerSendBinding,
  assertHiveWorkerSendAvailable,
} from "../../components/hive/workerSessionSendFence";
import { useHiveWorkers } from "../../components/hive/hooks/useHiveWorkers";
import { MobileAppHeader } from "../../components/navigation/MobileAppHeader";
import { AdaptiveMaterialMotionGate } from "../../components/ui/AdaptiveMaterial";
import { StreamSideEffectsCoordinator } from "../../components/chat/StreamSideEffectsCoordinator";
import { modeForHorizontalSwipe } from "../../components/navigation/modeSwipe";
import { createLatestIntentScheduler } from "../../components/navigation/latestIntentScheduler";
import { resolveRouteNavigationIntent } from "../../components/navigation/routeNavigationIntent";
import { isCurrentSessionNavigationIntent } from "../../components/navigation/sessionNavigationIntentFence";
import { displayThreadTitle } from "../../components/navigation/threadTitle";
import { useSplashState } from "../../hooks/useSplashState";
import { useEntranceAnimation } from "../../hooks/useEntranceAnimation";
import { useMobileDiagnosticMode } from "../../diagnostics/MobileDiagnosticsProvider";
import Animated, { runOnJS } from "react-native-reanimated";

import type { SessionResponse, SessionType } from "@mitsuro/api";
import type { HiveTopLevelView } from "../../components/hive/types";
import type { HiveChatContext } from "../../components/hive/types";
import type {
  Attachment as SessionAttachment,
  PermissionMode,
  ThinkingLevel,
} from "@mitsuro/state";
import {
  beginMitsuroPerformanceSpan,
  modelKeysEqual,
  supportsFastMode,
} from "@mitsuro/state";

import { ChatBootScreen } from "../../components/chat-screen/BootScreen";
import {
  CHAT_BAR_ZONE,
  sessionTypeForTab,
  tabForSessionType,
} from "../../components/chat-screen/helpers";
import {
  DESKTOP_CHAT_MAX_WIDTH,
  resolveDesktopChatMaxWidth,
  styles,
  TOOLBOX_DOCK_WIDTH,
} from "../../components/chat-screen/styles";
import { useSessionActions } from "../../components/chat-screen/useSessionActions";
import { useSessionController } from "../../components/chat-screen/useSessionController";
import { ActiveConversationSurface } from "../../components/chat-screen/ActiveConversationSurface";
import { buildEmptyHiveDispatchSelection } from "../../components/chat-screen/emptyHiveDispatchSelection";

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

  const [requestedMode, setRequestedMode] = useState<SessionType>("chat");
  const [activeMode, setActiveMode] = useState<SessionType>("chat");
  const activeModeRef = useRef(activeMode);
  activeModeRef.current = activeMode;
  const finishModeSwitchSpanRef = useRef<(() => number | null) | null>(null);
  // Header selection responds immediately; heavy surface/store work waits for
  // a short quiet window and commits only the latest requested mode. A hard
  // deadline here admitted another expensive surface every 80ms during
  // sustained stress input, allowing surface activations to accumulate.
  const modeIntentSchedulerRef = useRef<
    ReturnType<
      typeof createLatestIntentScheduler<SessionType>
    > | null
  >(null);
  if (!modeIntentSchedulerRef.current) {
    modeIntentSchedulerRef.current = createLatestIntentScheduler({
      quietDelayMs: 72,
      onFlush: (mode) => {
        if (mode === activeModeRef.current) {
          finishModeSwitchSpanRef.current?.();
          finishModeSwitchSpanRef.current = null;
          return;
        }
        // The scheduler has already coalesced bursty input. Commit the winner
        // synchronously so continuous transcript updates cannot starve the
        // actual surface change behind an immediately-updated header pill.
        setActiveMode(mode);
      },
    });
  }
  useEffect(() => {
    const scheduler = modeIntentSchedulerRef.current;
    return () => {
      scheduler?.cancel();
    };
  }, []);
  useMobileDiagnosticMode(activeMode);
  const finishToolboxOpenSpanRef = useRef<(() => number | null) | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameSaving, setRenameSaving] = useState(false);
  // Every action/store binding follows the committed deferred mode. Only the
  // header reflects requestedMode immediately, so a rapid send can never pair
  // the destination tab with the previous mode's session store.
  const activeTab = tabForSessionType(activeMode);
  const navigationIntentGenerationRef = useRef(0);
  const setActiveTab = useCallback((index: number) => {
    navigationIntentGenerationRef.current += 1;
    const mode = sessionTypeForTab(index);
    setRequestedMode(mode);
    modeIntentSchedulerRef.current?.submit(mode);
  }, []);
  const {
    session: sessionStore,
    workspace,
  } = stores.modes[activeMode];
  const { sessions: sessionsStore } = stores;

  const sessions = useSessionsStore(
    (state) => state.sessions,
  ) as SessionResponse[];
  // Shell chrome only. Live transcript messages stay in ActiveConversationSurface
  // so stream tokens do not re-render the whole product shell.
  const sessionView = useSessionStore(
    useShallow((state) => ({
      sessionId: state.sessionId,
      title: state.title,
      isStreaming: state.isStreaming,
      model: state.model,
      modelKey: state.modelKey,
      thinkingLevel: state.thinkingLevel,
      permissionMode: state.permissionMode,
      fastModeEnabled: state.fastModeEnabled,
      mode: state.mode,
      tokenCount: state.tokenCount,
      error: state.error,
    })),
    activeMode,
  );
  const hiveThinking = useSessionStore(
    (state) => (activeMode === "hive" ? state.isThinking : false),
    activeMode,
  );
  const hiveLoading = useSessionStore(
    (state) => (activeMode === "hive" ? state.isLoading : false),
    activeMode,
  );
  const sessionId = sessionView.sessionId ?? null;
  const sessionTitle = sessionView.title ?? null;
  const isStreaming = sessionView.isStreaming ?? false;
  const model = sessionView.model ?? null;
  const modelKey = sessionView.modelKey ?? null;
  const thinkingLevel = sessionView.thinkingLevel ?? "medium";
  const permissionMode = sessionView.permissionMode ?? "autonomous";
  const fastModeStoreEnabled = sessionView.fastModeEnabled ?? false;
  const mode = sessionView.mode ?? "build";
  const tokenCount = sessionView.tokenCount ?? 0;
  const error = sessionView.error ?? null;
  const workspaceDirectory =
    useWorkspaceStore((state) => state.directory, activeMode) ?? null;
  const workspaceTargetBranch =
    useWorkspaceStore((state) => state.targetBranch, activeMode) ?? null;
  const [activeToolCallId, setActiveToolCallId] = useState<string | null>(null);
  const [activeSheet, setActiveSheet] = useState<MobileSheet>(null);
  const [desktopToolboxOpen, setDesktopToolboxOpen] = useState(false);
  const drawerOpen = activeSheet === "threads";
  const setDrawerOpen = useCallback((open: boolean) => {
    setActiveSheet(open ? "threads" : null);
  }, []);
  const [hiveTopLevel, setHiveTopLevel] = useState<HiveTopLevelView>("hive");
  // A Worker selection is visible immediately, but transcript hydration stays
  // behind the existing quiet-window scheduler. Keep the old Hive session
  // fenced off until the store has synchronously adopted the exact target.
  const [pendingHiveThreadSessionId, setPendingHiveThreadSessionId] = useState<
    string | null
  >(null);
  const [hiveNotificationTarget, setHiveNotificationTarget] = useState<
    {
      messageId?: string;
      reportId?: string;
    } | null
  >(null);
  const toolboxOpen = isDesktop
    ? desktopToolboxOpen
    : activeSheet === "toolbox";
  useEffect(() => {
    if (!toolboxOpen) return;
    finishToolboxOpenSpanRef.current?.();
    finishToolboxOpenSpanRef.current = null;
  }, [toolboxOpen]);
  const [toolboxTabByMode, setToolboxTabByMode] = useState<
    Record<SessionType, number>
  >({
    chat: 0,
    code: 0,
    hive: 0,
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
  const [composerReserveHeight, setComposerReserveHeight] = useState(
    CHAT_BAR_ZONE,
  );
  const [
    mobileHiveIntroductionReserveHeight,
    setMobileHiveIntroductionReserveHeight,
  ] = useState(0);
  const [
    mobileHiveGoalTrackerReserveHeight,
    setMobileHiveGoalTrackerReserveHeight,
  ] = useState(0);
  const [mobileHeaderHeight, setMobileHeaderHeight] = useState(88);
  const [errorBannerHeight, setErrorBannerHeight] = useState(0);
  /** Measured desktop chat pane width (split host, before soft-cap). */
  const [desktopPaneWidth, setDesktopPaneWidth] = useState(0);
  const {
    models,
    ensureModelReady,
    recordSharedModelSelection,
    lastSessionIdByTypeRef,
  } = useSessionController({
    client,
    isConnected,
    activeMode,
    sessionStore,
    sessionsStore,
    modeStores: stores.modes,
    sessions,
  });
  // One roster owner serves both the desktop Hive surface and the stable
  // mobile transcript controls. Only the committed Hive mode enables reads.
  const hiveWorkers = useHiveWorkers(activeMode === "hive");

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
    const current = sessionStore.getState();
    if (
      current.model === model &&
      modelKeysEqual(current.modelKey, selectedModelInfo.key ?? null) &&
      current.modelProvider === (selectedModelInfo.provider ?? null) &&
      JSON.stringify(current.modelInfo) === JSON.stringify(selectedModelInfo)
    ) {
      return;
    }
    current.setModel(model, selectedModelInfo.provider, selectedModelInfo);
  }, [model, selectedModelInfo, sessionStore]);

  const suppressCompletionRef = useRef(false);
  const handledRouteIntentKeyRef = useRef<string | null>(null);

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
          await sessionStore.getState().submitToolApproval(
            toolCallId,
            approved,
          );
        } else if (client) {
          await client.submitToolApproval(
            targetSessionId,
            toolCallId,
            approved,
          );
        }
      } catch {
        if (
          currentSessionId === targetSessionId &&
          sessionStore.getState().sessionId === targetSessionId
        ) {
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
      const focusedSessionType =
        focus === "chat" || focus === "code" || focus === "hive" ? focus : null;
      const targetType = targetSession?.session_type ?? focusedSessionType ??
        activeMode;

      if (focus === "hive") {
        setHiveTopLevel("hive");
        setHiveNotificationTarget({
          ...(params?.messageId ? { messageId: params.messageId } : {}),
          ...(params?.reportId ? { reportId: params.reportId } : {}),
        });
      }
      setActiveTab(tabForSessionType(targetType));
      if (!targetSessionId) {
        if (focus === "hive") {
          await stores.modes.hive.session.getState().ensureHiveMainSession();
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
    const intent = resolveRouteNavigationIntent(routeParams);
    if (!intent) {
      handledRouteIntentKeyRef.current = null;
      return;
    }
    if (handledRouteIntentKeyRef.current === intent.key) return;

    handledRouteIntentKeyRef.current = intent.key;
    void handleNotificationNavigate("/(tabs)", intent.params);
  }, [
    handleNotificationNavigate,
    routeParams.focus,
    routeParams.messageId,
    routeParams.reportId,
    routeParams.sessionId,
  ]);

  const t = theme.colors;

  const {
    stopCurrentStream,
    loadSession,
    loadSessionById,
    openProjectInCode,
    handleNewSession,
    handleDirectorySelected,
    handleDeleteSession,
    handleSetSessionPinned,
    handleSetSessionArchived,
    handleSetProjectPinned,
    handleSetProjectArchived,
    handleDeleteProjectSessions,
    handleInteractiveToolResult,
    handlePlanConfirm,
    handleSend,
    handleModelSelect,
    handleFastModeToggle,
    cancelPendingSessionSelection,
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
    onSharedModelSelect: recordSharedModelSelection,
    suppressCompletionRef,
    lastSessionIdByTypeRef,
    navigationIntentGenerationRef,
  });

  const handleOpenHiveWorkerDm = useCallback(
    (targetSessionId: string) => {
      setPendingHiveThreadSessionId(targetSessionId);
      void loadSessionById(targetSessionId);
    },
    [loadSessionById],
  );

  const handleOpenSession = useCallback(
    (session: SessionResponse) => {
      setPendingHiveThreadSessionId(null);
      void loadSession(session);
    },
    [loadSession],
  );

  const handleOpenSessionById = useCallback(
    (targetSessionId: string) => {
      setPendingHiveThreadSessionId(null);
      return loadSessionById(targetSessionId);
    },
    [loadSessionById],
  );

  useEffect(() => {
    if (!pendingHiveThreadSessionId) {
      return;
    }
    if (activeMode !== "hive") {
      // A drawer-origin Worker selection requests Hive immediately, while the
      // committed mode still waits behind the same 72 ms quiet-window used by
      // every mode switch. Preserve the exact-session fence during that gap;
      // explicit navigation away from Hive clears both requested intent and
      // the pending target through the handlers above.
      if (requestedMode !== "hive") {
        setPendingHiveThreadSessionId(null);
      }
      return;
    }
    if (sessionId !== pendingHiveThreadSessionId) return;

    // loadSession adopts the destination shell synchronously before network
    // hydration. Only now may the single transcript surface replace Workers.
    setPendingHiveThreadSessionId(null);
    setHiveTopLevel("hive");
  }, [activeMode, pendingHiveThreadSessionId, requestedMode, sessionId]);

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
    (targetSessionId: string, toolCallId: string, result: string) => {
      void handleInteractiveToolResult(targetSessionId, toolCallId, result);
    },
    [handleInteractiveToolResult],
  );
  const handleTranscriptPlanConfirm = useCallback(
    (
      targetSessionId: string,
      toolCallId: string,
      choice: "execute" | "abandon",
    ) => {
      void handlePlanConfirm(targetSessionId, toolCallId, choice);
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

      if (activeMode === "hive" && !sessionStore.getState().sessionId) {
        const task = content.trim();
        if (!client || !task) {
          return;
        }
        const hiveStore = stores.modes.hive.session;
        const dispatchIntentGeneration = navigationIntentGenerationRef.current;
        const isCurrentDispatchPhase = (expectedSessionId: string | null) =>
          activeModeRef.current === "hive" &&
          isCurrentSessionNavigationIntent(
            dispatchIntentGeneration,
            navigationIntentGenerationRef.current,
            expectedSessionId,
            hiveStore.getState().sessionId,
          );
        if (normalizedAttachments.length > 0) {
          if (!isCurrentDispatchPhase(null)) return;
          hiveStore.setState({
            error:
              "Start the Hive thread with text, then attach files in the conversation.",
          });
          return;
        }
        if (!isCurrentDispatchPhase(null)) return;
        const resolvedModel = await ensureModelReady();
        if (!isCurrentDispatchPhase(null)) return;
        if (!resolvedModel) {
          hiveStore.setState({
            error: "Choose an available model before starting Hive.",
          });
          return;
        }
        const dispatchModelSelection = buildEmptyHiveDispatchSelection(
          resolvedModel,
          hiveStore.getState().modelKey,
        );
        let dispatchedSessionId: string | null = null;
        try {
          if (!isCurrentDispatchPhase(null)) return;
          const response = await client.dispatchHive(task, {
            projectDir: workspaceDirectory ?? undefined,
            ...dispatchModelSelection,
          });
          if (!isCurrentDispatchPhase(null)) return;
          dispatchedSessionId = response.session_id;
          lastSessionIdByTypeRef.current.hive = response.session_id;
          await sessionsStore.getState().loadSessions();
          if (!isCurrentDispatchPhase(null)) return;
          await hiveStore.getState().loadSession(response.session_id, true);
          if (!isCurrentDispatchPhase(response.session_id)) return;
          void Haptics.notificationAsync(
            Haptics.NotificationFeedbackType.Success,
          );
        } catch (dispatchError) {
          const isCurrentFailure = isCurrentDispatchPhase(null) ||
            (dispatchedSessionId != null &&
              isCurrentDispatchPhase(dispatchedSessionId));
          if (!isCurrentFailure) return;
          hiveStore.setState({
            error: dispatchError instanceof Error
              ? dispatchError.message
              : "Failed to start the Hive thread.",
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
      stores.modes.hive.session,
      workspaceDirectory,
    ],
  );

  const handleHiveWorkerSend = useCallback(
    async (expectedSessionId: string, content: string) => {
      const assertCurrentTarget = () => {
        assertCurrentHiveWorkerSendBinding(expectedSessionId, {
          activeMode: activeModeRef.current,
          sessionId: stores.modes.hive.session.getState().sessionId,
        });
      };
      assertCurrentTarget();
      // The dedicated composer clears optimistically. Reject before entering
      // the generic sender when disconnected so this exact Worker's draft is
      // restored instead of being silently discarded.
      assertHiveWorkerSendAvailable(Boolean(client) && isConnected);
      await handleSend(content, [], {
        assertCurrent: assertCurrentTarget,
        skipModelReadiness: true,
        rethrowErrors: true,
        sendOptions: { hiveConversationKind: "worker_dm" },
      });
    },
    [client, handleSend, isConnected, stores.modes.hive.session],
  );

  const handleHiveWorkerStop = useCallback(
    (expectedSessionId: string) => {
      try {
        assertCurrentHiveWorkerSendBinding(expectedSessionId, {
          activeMode: activeModeRef.current,
          sessionId: stores.modes.hive.session.getState().sessionId,
        });
      } catch {
        return;
      }
      stopCurrentStream(true, {
        expectedSessionId,
        hiveConversationKind: "worker_dm",
      });
    },
    [stopCurrentStream, stores.modes.hive.session],
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
        error instanceof Error
          ? error.message
          : "Could not update session title.",
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
  const handleToolboxOpen = useCallback(() => {
    finishToolboxOpenSpanRef.current?.();
    finishToolboxOpenSpanRef.current = beginMitsuroPerformanceSpan(
      "toolbox.open",
      activeMode,
    );
    if (isDesktop) {
      setDesktopToolboxOpen(true);
    } else {
      setActiveSheet("toolbox");
    }
  }, [activeMode, isDesktop]);

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
  // The selected pill reflects requestedMode immediately while expensive
  // surface activation waits for the quiet window. Read the requested store's
  // already-known title during that short gap so Code never appears above the
  // previous mode's title. The committed selector resumes ownership once both
  // modes agree.
  const requestedModeDisplayTitle = requestedMode === activeMode
    ? displayTitle
    : displayThreadTitle(
      stores.modes[requestedMode].session.getState().title,
    );

  const handleModeChange = useCallback(
    (mode: SessionType) => {
      if (mode === requestedMode) return;
      cancelPendingSessionSelection();
      setPendingHiveThreadSessionId(null);
      finishModeSwitchSpanRef.current?.();
      finishModeSwitchSpanRef.current = beginMitsuroPerformanceSpan(
        "mode.switch",
        `${activeMode}->${mode}`,
      );
      setActiveSheet(null);
      setActiveTab(tabForSessionType(mode));
    },
    [
      activeMode,
      cancelPendingSessionSelection,
      requestedMode,
      setActiveTab,
    ],
  );
  useEffect(() => {
    finishModeSwitchSpanRef.current?.();
    finishModeSwitchSpanRef.current = null;
  }, [activeMode]);

  const handleNewHiveSession = useCallback(() => {
    // handleModeChange intentionally elides same-mode requests. New Hive is a
    // distinct navigation intent even while already in Hive, so invalidate
    // every older async continuation before clearing the current thread.
    navigationIntentGenerationRef.current += 1;
    cancelPendingSessionSelection();
    setPendingHiveThreadSessionId(null);
    handleModeChange("hive");
    setActiveSheet(null);
    const hiveStore = stores.modes.hive.session;
    hiveStore.getState().detachSession();
    hiveStore.getState().clearSession();
  }, [cancelPendingSessionSelection, handleModeChange, stores.modes]);

  const handleSelectHiveView = useCallback(
    (view: HiveTopLevelView) => {
      cancelPendingSessionSelection();
      setPendingHiveThreadSessionId(null);
      handleModeChange("hive");
      setHiveTopLevel(view);
      setDrawerOpen(false);
    },
    [cancelPendingSessionSelection, handleModeChange],
  );
  const modeSwipeBlocked = isDesktop || drawerOpen || toolboxOpen ||
    bottomControlsOpen;
  const modeSwipeBlockedRef = useRef(modeSwipeBlocked);
  modeSwipeBlockedRef.current = modeSwipeBlocked;
  const handleModeSwipe = useCallback(
    (translationX: number, velocityX: number) => {
      // A pan can begin before a drawer, toolbox, or composer overlay claims
      // the interaction. Re-check current ownership when its native end event
      // returns to JS so stale gestures cannot close newly opened chrome.
      if (modeSwipeBlockedRef.current) return;
      const nextMode = modeForHorizontalSwipe(
        requestedMode,
        translationX,
        velocityX,
      );
      if (!nextMode) {
        return;
      }
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      handleModeChange(nextMode);
    },
    [handleModeChange, requestedMode],
  );
  const modeSwipeGesture = useMemo(
    () =>
      Gesture.Pan()
        .enabled(!modeSwipeBlocked)
        .activeOffsetX([-28, 28])
        .failOffsetY([-20, 20])
        .onEnd((event) => {
          runOnJS(handleModeSwipe)(event.translationX, event.velocityX);
        }),
    [
      handleModeSwipe,
      modeSwipeBlocked,
    ],
  );

  const topBar = isDesktop
    ? (
      <Animated.View
        style={[
          styles.topBar,
          styles.topBarDesktop,
          entrance.settled ? null : entrance.topBarStyle,
        ]}
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
            onPress={() => handleSelectHiveView("hive")}
            style={styles.toolboxCornerBtn}
            accessibilityRole="button"
            accessibilityLabel="Open Hive"
          >
            <HiveIcon
              size={20}
              color={t.mutedForeground}
              strokeWidth={1.8}
            />
          </Pressable>
          <Pressable
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
              if (toolboxOpen) {
                handleToolboxClose();
              } else {
                handleToolboxOpen();
              }
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
    )
    : (
      <Animated.View
        style={[
          styles.mobileTopBarOverlay,
          entrance.settled ? null : entrance.topBarStyle,
        ]}
      >
        <MobileAppHeader
          mode={requestedMode}
          title={requestedModeDisplayTitle}
          onModeChange={handleModeChange}
          onOpenThreads={() => setActiveSheet("threads")}
          onOpenToolbox={handleToolboxOpen}
          onHeightChange={(nextHeight) =>
            setMobileHeaderHeight((current) =>
              current === nextHeight ? current : nextHeight
            )}
          onTitlePress={requestedMode === activeMode &&
              sessionId &&
              requestedModeDisplayTitle &&
              activeMode !== "hive"
            ? handleRenameSession
            : undefined}
        />
      </Animated.View>
    );

  const sharedComposer = (
    <Animated.View
      style={[
        entrance.settled ? null : entrance.bottomBarStyle,
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
          sessionStore.getState().setThinkingLevel(level)}
        permissionMode={permissionMode as PermissionMode}
        onPermissionModeToggle={() =>
          sessionStore.getState().togglePermissionMode()}
        fastModeEnabled={fastModeEnabled}
        fastModeSupported={fastModeSupported}
        onFastModeToggle={handleFastModeToggle}
        mode={mode}
        onModeToggle={() =>
          sessionStore
            .getState()
            .setMode(mode === "build" ? "plan" : "build")}
        onModelSelect={handleModelSelect}
        model={model}
        modelKey={modelKey}
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

  // One transcript tree only. Keeping three absolute FlatLists mounted for
  // "warm modes" crashed native on mode switch (especially leaving chat).
  // Session switches still avoid remount via no key + scroll restore.
  // Messages and their primitive Worker-control projection are both owned by
  // ActiveConversationSurface; the outer shell never subscribes to messages.
  const chatTranscriptSurface = (
    <Animated.View
      style={[styles.flex, entrance.settled ? null : entrance.contentStyle]}
    >
      <ActiveConversationSurface
        activeMode={activeMode}
        sessionType={activeMode}
        activeToolCallId={activeToolCallId}
        bottomPadding={composerReserveHeight +
          (activeMode === "hive" && !isDesktop
            ? mobileHiveIntroductionReserveHeight +
              mobileHiveGoalTrackerReserveHeight +
              (mobileHiveGoalTrackerReserveHeight > 0 ? 8 : 0)
            : 0)}
        topFadeHeight={isDesktop ? undefined : mobileHeaderHeight + 128}
        topFadeOffset={isDesktop ? undefined : 0}
        topContentPadding={isDesktop ? undefined : mobileHeaderHeight + 16}
        hideJumpToLatest={bottomControlsOpen}
        showPlanTracker={activeMode !== "hive"}
        hidePlanTracker={bottomControlsOpen}
        errorBannerHeight={errorBannerHeight}
        onErrorBannerHeightChange={(nextHeight) => {
          setErrorBannerHeight((current) =>
            current === nextHeight ? current : nextHeight
          );
        }}
        onApproveTool={handleApproveTranscriptTool}
        onDenyTool={handleDenyTranscriptTool}
        onSubmitToolResult={handleSubmitTranscriptTool}
        onPlanConfirm={handleTranscriptPlanConfirm}
        renderThreadControls={!isDesktop && activeMode === "hive"
          ? (activity) => (
            <HiveMobileThreadControls
              {...activity}
              workers={hiveWorkers}
              primaryComposer={sharedComposer}
              onSend={handleHiveWorkerSend}
              onStop={handleHiveWorkerStop}
              composerHeight={composerReserveHeight}
              introductionHeight={mobileHiveIntroductionReserveHeight}
              onComposerHeightChange={setComposerReserveHeight}
              onIntroductionHeightChange={setMobileHiveIntroductionReserveHeight}
              onGoalTrackerHeightChange={setMobileHiveGoalTrackerReserveHeight}
            />
          )
          : undefined}
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
      {isDesktop
        ? (
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
        )
        : (
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
      {isDesktop
        ? (
          // Desktop: chat fills remaining width; toolbox is a fixed-width rail.
          <View style={styles.desktopSplit}>
            <View style={styles.desktopSplitChat}>{chatMain}</View>
            {toolboxOpen
              ? (
                <ToolboxPanel
                  variant="dock"
                  visible={toolboxOpen}
                  onClose={handleToolboxClose}
                  activeTab={toolboxTab}
                  onTabChange={setToolboxTab}
                  sessionType={activeMode}
                  projectDirectory={workspaceDirectory}
                  onOpenSettings={() => router.navigate("/(tabs)/settings")}
                  onOpenHiveRun={(id) => void handleOpenSessionById(id)}
                  onOpenProject={(path, branch) =>
                    void openProjectInCode(path, branch)}
                />
              )
              : null}
          </View>
        )
        : (
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
                router.navigate("/(tabs)/settings");
              }}
              onOpenHiveRun={(id) => void handleOpenSessionById(id)}
              onOpenProject={(path, branch) =>
                void openProjectInCode(path, branch)}
            />
          </>
        )}
    </SafeAreaView>
  );

  const hiveChat: HiveChatContext = {
    sessionId,
    title: sessionTitle,
    error,
    isLoading: hiveLoading,
    isStreaming,
    isThinking: hiveThinking,
    activeToolCallId,
    thinkingLevel: thinkingLevel as ThinkingLevel,
    permissionMode: permissionMode as PermissionMode,
    fastModeEnabled,
    fastModeSupported,
    mode,
    model,
    modelKey,
    models,
    tokenCount,
    onApproveTool: (targetSessionId, toolCallId) =>
      handleSessionToolApproval(targetSessionId, toolCallId, true),
    onDenyTool: (targetSessionId, toolCallId) =>
      handleSessionToolApproval(targetSessionId, toolCallId, false),
    onSubmitToolResult: (targetSessionId, toolCallId, result) =>
      void handleInteractiveToolResult(targetSessionId, toolCallId, result),
    onPlanConfirm: (targetSessionId, toolCallId, choice) =>
      void handlePlanConfirm(targetSessionId, toolCallId, choice),
    onSend: handleChatBarSend,
    onWorkerSend: handleHiveWorkerSend,
    onWorkerStop: handleHiveWorkerStop,
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

  // Desktop owns the full Hive product surface. Mobile mounts this tree only
  // for non-thread management views; its thread stays in the single stable
  // transcript/composer surface below.
  const hiveContent = (
    <Animated.View
      style={[styles.flex, entrance.settled ? null : entrance.contentStyle]}
    >
      <HiveScreen
        workspaceDirectory={workspaceDirectory}
        requestedTopLevel={hiveTopLevel}
        requestedThreadMessageId={hiveNotificationTarget?.messageId}
        requestedReportId={hiveNotificationTarget?.reportId}
        onOpenRunById={handleOpenSessionById}
        onOpenWorkerDm={handleOpenHiveWorkerDm}
        onOpenProject={openProjectInCode}
        onDeleteRun={(id) =>
          handleDeleteSession(id, "hive")}
        onOpenMenu={!isDesktop ? () => setDrawerOpen(true) : undefined}
        onTopLevelChange={setHiveTopLevel}
        chat={hiveChat}
        workers={hiveWorkers}
      />
    </Animated.View>
  );

  const showMobileHiveManagement = activeMode === "hive" &&
    hiveTopLevel !== "hive";
  const showMobileHiveThreadTransition = activeMode === "hive" &&
    pendingHiveThreadSessionId !== null &&
    sessionId !== pendingHiveThreadSessionId;

  const mobileHiveThreadTransition = (
    <SafeAreaView
      style={[styles.container, { backgroundColor: t.background }]}
      edges={["top"]}
    >
      <View
        style={{
          flex: 1,
          alignItems: "center",
          justifyContent: "center",
          gap: 12,
        }}
        accessibilityRole="progressbar"
        accessibilityLabel="Opening Hive Worker conversation"
      >
        <ActivityIndicator color={t.thinking} />
        <Text style={{ color: t.mutedForeground }}>Opening Worker…</Text>
      </View>
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
        <View
          style={{
            flex: 1,
            backgroundColor: t.background,
          }}
        >
          {
            /* One stable native transcript tree for every mobile mode. Parallel
              FlatLists crashed under New Architecture, while swapping the
              Hive tree forced expensive Fabric unmount/mount transactions. */
          }
          {chatTranscriptSurface}
        </View>
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
      {activeMode === "hive" ? null : sharedComposer}
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
          router.navigate("/(tabs)/settings");
        }}
        onOpenHiveRun={(id) => void loadSessionById(id)}
        onOpenProject={(path, branch) => void openProjectInCode(path, branch)}
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
              backgroundColor: t.background,
              borderColor: t.border,
            },
          ]}
          onPress={(event) => event.stopPropagation()}
        >
          <Text style={[styles.renameTitle, { color: t.foreground }]}>
            Rename session
          </Text>
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
              <Text
                style={[styles.renameButtonText, { color: t.mutedForeground }]}
              >
                Cancel
              </Text>
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
              <Text style={[styles.renameButtonText, { color: t.onAccent }]}>
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
      onSelectSession={handleOpenSession}
      onNewSession={() => void handleNewSession("chat")}
      onNewSessionWithDir={(path) => void handleDirectorySelected(path)}
      onDeleteSession={handleDeleteSession}
      onOpenSettings={() => router.navigate("/(tabs)/settings")}
      activeTab={activeTab}
      onTabChange={(index) => handleModeChange(sessionTypeForTab(index))}
      activeHiveView={hiveTopLevel}
      onSelectHiveView={handleSelectHiveView}
    >
      <AdaptiveMaterialMotionGate safe={entrance.materialSafe}>
        <StreamSideEffectsCoordinator
          activeMode={activeMode}
          suppressCompletionRef={suppressCompletionRef}
        />
        {renameModal}
        {isDesktop
          ? (
            activeTab === 2 ? hiveContent : chatContent
          )
          : showMobileHiveManagement
          ? hiveContent
          : showMobileHiveThreadTransition
          ? mobileHiveThreadTransition
          : mobileContent}

        {!isDesktop && (
          <SessionDrawer
            isOpen={drawerOpen}
            onClose={() => setDrawerOpen(false)}
            sessions={sessions}
            activeSessionId={sessionId}
            onSelectSession={handleOpenSession}
            onSelectHiveSession={handleOpenHiveWorkerDm}
            activeHiveView={hiveTopLevel}
            onSelectHiveView={handleSelectHiveView}
            onNewSession={(type) => void handleNewSession(type)}
            onNewHiveSession={handleNewHiveSession}
            onNewSessionWithDir={(path) => void handleDirectorySelected(path)}
            onDeleteSession={handleDeleteSession}
            onSetSessionPinned={handleSetSessionPinned}
            onSetSessionArchived={handleSetSessionArchived}
            onSetProjectPinned={handleSetProjectPinned}
            onSetProjectArchived={handleSetProjectArchived}
            onDeleteProjectSessions={handleDeleteProjectSessions}
            onOpenSettings={() => {
              setDrawerOpen(false);
              router.navigate("/(tabs)/settings");
            }}
            activeMode={activeMode}
          />
        )}
      </AdaptiveMaterialMotionGate>
    </DesktopShell>
  );
}
