import {
  memo,
  type ReactNode,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { Animated, Pressable, Text, View } from "react-native";
import { useShallow } from "zustand/react/shallow";
import type { SessionType } from "@mitsuro/api";

import { ChatTranscript } from "../chat/ChatTranscript";
import { PlanTracker } from "../chat/PlanTracker";
import { MitsuroWordmark } from "../brand";
import { useSessionStore } from "../../hooks/useStores";
import { useThemeContext } from "../../hooks/useTheme";
import { styles } from "./styles";

interface ActiveConversationSurfaceProps {
  activeMode: SessionType;
  sessionType: SessionType;
  activeToolCallId: string | null;
  bottomPadding: number;
  topFadeHeight?: number;
  topFadeOffset?: number;
  topContentPadding?: number;
  hideJumpToLatest?: boolean;
  showPlanTracker?: boolean;
  hidePlanTracker?: boolean;
  showErrorBanner?: boolean;
  errorBannerHeight: number;
  onErrorBannerHeightChange: (height: number) => void;
  onApproveTool: (sessionId: string, toolCallId: string) => void;
  onDenyTool: (sessionId: string, toolCallId: string) => void;
  onSubmitToolResult: (
    sessionId: string,
    toolCallId: string,
    result: string,
  ) => void;
  onPlanConfirm: (
    sessionId: string,
    toolCallId: string,
    choice: "execute" | "abandon",
  ) => void;
  renderThreadControls?: (
    activity: ActiveConversationActivity,
  ) => ReactNode;
  emptyState?: ReactNode;
}

export interface ActiveConversationActivity {
  sessionId: string | null;
  transcriptTailKey: string;
  isStreaming: boolean;
  isThinking: boolean;
  messageCount: number;
}

function ActiveConversationSurfaceComponent({
  activeMode,
  sessionType,
  activeToolCallId,
  bottomPadding,
  topFadeHeight,
  topFadeOffset,
  topContentPadding,
  hideJumpToLatest = false,
  showPlanTracker = true,
  hidePlanTracker = false,
  showErrorBanner = true,
  errorBannerHeight,
  onErrorBannerHeightChange,
  onApproveTool,
  onDenyTool,
  onSubmitToolResult,
  onPlanConfirm,
  renderThreadControls,
  emptyState,
}: ActiveConversationSurfaceProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  // Messages live here only. The outer shell must not subscribe to them.
  const sessionView = useSessionStore(
    useShallow((state) => ({
      sessionId: state.sessionId,
      messages: state.messages,
      isStreaming: state.isStreaming,
      isThinking: state.isThinking,
      error: state.error,
      isLoading: state.isLoading,
      queuedRecoveryBlocked: state.queuedRecoveryBlocked,
      retryQueuedRecovery: state.retryQueuedRecovery,
      discardQueuedRecovery: state.discardQueuedRecovery,
    })),
    activeMode,
  );

  const sessionId = sessionView.sessionId ?? null;
  const messages = sessionView.messages ?? [];
  const isStreaming = sessionView.isStreaming ?? false;
  const isThinking = sessionView.isThinking ?? false;
  const isLoading = sessionView.isLoading ?? false;
  const error = sessionView.error ?? null;
  const queuedRecoveryBlocked = sessionView.queuedRecoveryBlocked ?? false;
  const tail = messages.at(-1);
  const messageCount = messages.length;
  const tailId = tail?.id ?? null;
  const tailRole = tail?.role ?? null;
  const activity = useMemo<ActiveConversationActivity>(() => ({
    sessionId,
    transcriptTailKey: `${messageCount}:${tailId ?? "none"}:${
      tailRole ?? "none"
    }`,
    isStreaming,
    isThinking,
    messageCount,
  }), [sessionId, messageCount, tailId, tailRole, isStreaming, isThinking]);
  const showTranscriptError = Boolean(error) && showErrorBanner;
  const [planTrackerHeight, setPlanTrackerHeight] = useState(0);
  const [planTrackerMounted, setPlanTrackerMounted] = useState(
    !hidePlanTracker,
  );
  const planTrackerOpacity = useRef(
    new Animated.Value(hidePlanTracker ? 0 : 1),
  ).current;
  const planTrackerGap = planTrackerHeight > 0 ? 10 : 0;
  const conversationBottomPadding = bottomPadding + planTrackerHeight +
    planTrackerGap;
  // Keep collapsed goal/plan chips out of the Agent FAB column so the
  // accordion can sit in its original right/bottom alignment.
  const planTrackerRightInset = 12 + 56 + 10;

  useEffect(() => {
    if (!showPlanTracker) setPlanTrackerHeight(0);
  }, [showPlanTracker]);

  useEffect(() => {
    if (hidePlanTracker) setPlanTrackerHeight(0);
  }, [hidePlanTracker]);

  useEffect(() => {
    if (!hidePlanTracker) setPlanTrackerMounted(true);
    Animated.timing(planTrackerOpacity, {
      toValue: hidePlanTracker ? 0 : 1,
      duration: hidePlanTracker ? 90 : 140,
      useNativeDriver: true,
    }).start(({ finished }) => {
      if (finished && hidePlanTracker) setPlanTrackerMounted(false);
    });
  }, [hidePlanTracker, planTrackerOpacity]);

  return (
    <View style={styles.flex}>
      <ChatTranscript
        messages={messages}
        sessionId={sessionId}
        sessionType={sessionType}
        scrollStateKey={`${activeMode}:${sessionId ?? "new"}`}
        isStreaming={isStreaming}
        isThinking={isThinking}
        isLoading={isLoading}
        activeToolCallId={activeToolCallId}
        onApproveTool={onApproveTool}
        onDenyTool={onDenyTool}
        onSubmitToolResult={onSubmitToolResult}
        onPlanConfirm={onPlanConfirm}
        emptyState={emptyState ?? (
          <View style={styles.empty}>
            <MitsuroWordmark />
            {error
              ? (
                <Text style={[styles.emptyHint, { color: t.error }]}>
                  {error}
                </Text>
              )
              : null}
          </View>
        )}
        bottomPadding={conversationBottomPadding +
          (showTranscriptError ? errorBannerHeight + 10 : 0)}
        topFadeHeight={topFadeHeight}
        topFadeOffset={topFadeOffset}
        topContentPadding={topContentPadding}
        hideJumpToLatest={hideJumpToLatest}
      />

      {showPlanTracker && planTrackerMounted
        ? (
          <Animated.View
            pointerEvents={hidePlanTracker ? "none" : "box-none"}
            style={{
              position: "absolute",
              left: 12,
              right: planTrackerRightInset,
              bottom: bottomPadding + 8,
              zIndex: 40,
              opacity: planTrackerOpacity,
            }}
          >
            <PlanTracker
              sessionType={sessionType}
              onHeightChange={hidePlanTracker
                ? undefined
                : setPlanTrackerHeight}
            />
          </Animated.View>
        )
        : null}

      {showTranscriptError
        ? (
          <View
            accessibilityRole="alert"
            accessibilityLiveRegion="polite"
            onLayout={(event) => {
              const nextHeight = Math.ceil(event.nativeEvent.layout.height);
              onErrorBannerHeightChange(nextHeight);
            }}
            style={[
              styles.errorBanner,
              {
                position: "absolute",
                left: 0,
                right: 0,
                bottom: conversationBottomPadding + 10,
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
            {queuedRecoveryBlocked
              ? (
                <View style={styles.errorBannerActions}>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel="Retry uncertain queued message"
                    onPress={() => void sessionView.retryQueuedRecovery()}
                    style={({ pressed }) => [
                      styles.errorBannerAction,
                      { borderColor: `${t.error}70`, opacity: pressed ? 0.7 : 1 },
                    ]}
                  >
                    <Text style={[styles.errorBannerActionText, { color: t.error }]}>Retry</Text>
                  </Pressable>
                  <Pressable
                    accessibilityRole="button"
                    accessibilityLabel="Discard uncertain queued message"
                    onPress={() => void sessionView.discardQueuedRecovery()}
                    style={({ pressed }) => [
                      styles.errorBannerAction,
                      { borderColor: `${t.error}70`, opacity: pressed ? 0.7 : 1 },
                    ]}
                  >
                    <Text style={[styles.errorBannerActionText, { color: t.error }]}>Discard</Text>
                  </Pressable>
                </View>
              )
              : null}
          </View>
        )
        : null}

      {renderThreadControls?.(activity)}
    </View>
  );
}

export const ActiveConversationSurface = memo(
  ActiveConversationSurfaceComponent,
);
