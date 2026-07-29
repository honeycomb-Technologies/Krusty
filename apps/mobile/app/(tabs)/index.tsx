import {
  useState,
  useRef,
  useCallback,
  useEffect,
  useMemo,
  startTransition,
} from "react";
import {
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
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import {
  useSessionStore,
  useSessionsStore,
  useStores,
  useWorkspaceStore,
} from "../../hooks/useStores";
import { useShallow } from "zustand/react/shallow";
import { HiveIcon } from "../../components/brand";
import {
  ChatBar,
  type Attachment as ChatBarAttachment,
} from "../../components/chat/ChatBar";
import { SessionDrawer } from "../../components/chat/SessionDrawer";
import { DesktopShell } from "../../components/layout/DesktopShell";
import { ToolboxPanel } from "../../components/ToolboxPanel";
import { MakoScreen } from "../../components/mako/MakoScreen";
import { MobileAppHeader } from "../../components/navigation/MobileAppHeader";
import { StreamSideEffectsCoordinator } from "../../components/chat/StreamSideEffectsCoordinator";
import { modeForHorizontalSwipe } from "../../components/navigation/modeSwipe";
import { createLatestIntentScheduler } from "../../components/navigation/latestIntentScheduler";
import { displayThreadTitle } from "../../components/navigation/threadTitle";
import { useSplashState } from "../../hooks/useSplashState";
import { useEntranceAnimation } from "../../hooks/useEntranceAnimation";
import { useMobileDiagnosticMode } from "../../diagnostics/MobileDiagnosticsProvider";
import Animated, { runOnJS } from "react-native-reanimated";

import type {
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
  beginKrustyPerformanceSpan,
  modelKeysEqual,
  supportsFastMode,
} from "@krusty/state";

import { ChatBootScreen } from "./chat-screen/BootScreen";
import {
  CHAT_BAR_ZONE,
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
import { useSessionController } from "./chat-screen/useSessionController";
import { ActiveConversationSurface } from "./chat-screen/ActiveConversationSurface";

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
  // Header selection responds immediately; heavy surface/store work can skip
  // superseded intermediate requests and settle on the latest mode. Unlike a
  // deferred value, the hard deadline prevents continuous taps from starving
  // the destination surface forever.
  const modeIntentSchedulerRef = useRef<ReturnType<
    typeof createLatestIntentScheduler<SessionType>
  > | null>(null);
  if (!modeIntentSchedulerRef.current) {
    modeIntentSchedulerRef.current = createLatestIntentScheduler({
      quietDelayMs: 24,
      maxDelayMs: 80,
      onFlush: (mode) => {
        startTransition(() => setActiveMode(mode));
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
  const finishModeSwitchSpanRef = useRef<(() => number | null) | null>(null);
  const finishToolboxOpenSpanRef = useRef<(() => number | null) | null>(null);
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [renameSaving, setRenameSaving] = useState(false);
  // Every action/store binding follows the committed deferred mode. Only the
  // header reflects requestedMode immediately, so a rapid send can never pair
  // the destination tab with the previous mode's session store.
  const activeTab = tabForSessionType(activeMode);
  const setActiveTab = useCallback((index: number) => {
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
  const makoThinking = useSessionStore(
    (state) => (activeMode === "mako" ? state.isThinking : false),
    activeMode,
  );
  const makoLoading = useSessionStore(
    (state) => (activeMode === "mako" ? state.isLoading : false),
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
  const [makoTopLevel, setMakoTopLevel] = useState<MakoTopLevelView>("mako");
  const [makoNotificationTarget, setMakoNotificationTarget] = useState<{
    messageId?: string;
    reportId?: string;
  } | null>(null);
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
  const {
    models,
    ensureModelReady,
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
      current.model === model
      && modelKeysEqual(current.modelKey, selectedModelInfo.key ?? null)
      && current.modelProvider === (selectedModelInfo.provider ?? null)
      && JSON.stringify(current.modelInfo) === JSON.stringify(selectedModelInfo)
    ) {
      return;
    }
    current.setModel(model, selectedModelInfo.provider, selectedModelInfo);
  }, [model, selectedModelInfo, sessionStore]);

  const suppressCompletionRef = useRef(false);

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

  const t = theme.colors;

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
    activateSessionType,
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
            error: "Start the Hive thread with text, then attach files in the conversation.",
          });
          return;
        }
        const resolvedModel = await ensureModelReady();
        if (!resolvedModel) {
          sessionStore.setState({
            error: "Choose an available model before starting Hive.",
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
      workspaceDirectory,
    ],
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
  const handleToolboxOpen = useCallback(() => {
    finishToolboxOpenSpanRef.current?.();
    finishToolboxOpenSpanRef.current = beginKrustyPerformanceSpan(
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

  const handleModeChange = useCallback(
    (mode: SessionType) => {
      if (mode === requestedMode) return;
      finishModeSwitchSpanRef.current?.();
      finishModeSwitchSpanRef.current = beginKrustyPerformanceSpan(
        "mode.switch",
        `${activeMode}->${mode}`,
      );
      setActiveSheet(null);
      setActiveTab(tabForSessionType(mode));
    },
    [activeMode, requestedMode, setActiveTab],
  );
  useEffect(() => {
    finishModeSwitchSpanRef.current?.();
    finishModeSwitchSpanRef.current = null;
    activateSessionType(activeMode);
  }, [activateSessionType, activeMode]);

  const handleNewMakoSession = useCallback(() => {
    handleModeChange("mako");
    setActiveSheet(null);
    const makoStore = stores.modes.mako.session;
    makoStore.getState().detachSession();
    makoStore.getState().clearSession();
  }, [handleModeChange, stores.modes]);

  const handleSelectMakoView = useCallback(
    (view: MakoTopLevelView) => {
      handleModeChange("mako");
      setMakoTopLevel(view);
      setDrawerOpen(false);
    },
    [handleModeChange],
  );
  const modeSwipeBlocked =
    isDesktop || drawerOpen || toolboxOpen || bottomControlsOpen;
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
  ) : (
    <Animated.View style={entrance.topBarStyle}>
      <MobileAppHeader
        mode={requestedMode}
        title={displayTitle}
        onModeChange={handleModeChange}
        onOpenThreads={() => setActiveSheet("threads")}
        onOpenToolbox={handleToolboxOpen}
        onTitlePress={
          sessionId && displayTitle && activeMode !== "mako"
            ? handleRenameSession
            : undefined
        }
      />
    </Animated.View>
  );

  // One transcript tree only. Keeping three absolute FlatLists mounted for
  // "warm modes" crashed native on mode switch (especially leaving chat).
  // Session switches still avoid remount via no key + scroll restore.
  // Messages subscription is isolated inside ActiveConversationSurface.
  const chatTranscriptSurface = (
      <Animated.View style={[styles.flex, entrance.contentStyle]}>
        <ActiveConversationSurface
          activeMode={activeMode}
          sessionType={activeMode}
          activeToolCallId={activeToolCallId}
          bottomPadding={composerReserveHeight}
          hideJumpToLatest={bottomControlsOpen}
          showPlanTracker={activeMode !== "mako"}
          errorBannerHeight={errorBannerHeight}
          onErrorBannerHeightChange={(nextHeight) => {
            setErrorBannerHeight((current) =>
              current === nextHeight ? current : nextHeight,
            );
          }}
          onApproveTool={handleApproveTranscriptTool}
          onDenyTool={handleDenyTranscriptTool}
          onSubmitToolResult={handleSubmitTranscriptTool}
          onPlanConfirm={handleTranscriptPlanConfirm}
        />
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
              onOpenSettings={() => router.navigate("/(tabs)/settings")}
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
              router.navigate("/(tabs)/settings");
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
    error,
    isLoading: makoLoading,
    isStreaming,
    isThinking: makoThinking,
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

  // Desktop owns the full Mako product surface. Mobile uses a single active
  // conversation surface so mode switches stay crash-free.
  const makoContent = (
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
          {/* One stable native transcript tree for every mobile mode. Parallel
              FlatLists crashed under New Architecture, while swapping the
              Mako tree forced expensive Fabric unmount/mount transactions. */}
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
          router.navigate("/(tabs)/settings");
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
      onOpenSettings={() => router.navigate("/(tabs)/settings")}
      activeTab={activeTab}
      onTabChange={(index) => handleModeChange(sessionTypeForTab(index))}
      activeMakoView={makoTopLevel}
      onSelectMakoView={handleSelectMakoView}
    >
      <StreamSideEffectsCoordinator
        activeMode={activeMode}
        suppressCompletionRef={suppressCompletionRef}
      />
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
            router.navigate("/(tabs)/settings");
          }}
          activeMode={activeMode}
        />
      )}
    </DesktopShell>
  );
}
