import { memo, type ReactNode, useEffect, useRef, useState } from "react";
import { Animated, Text, View } from "react-native";
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
  onSubmitToolResult: (toolCallId: string, result: string) => void;
  onPlanConfirm: (toolCallId: string, choice: "execute" | "abandon") => void;
  emptyState?: ReactNode;
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
    })),
    activeMode,
  );

  const sessionId = sessionView.sessionId ?? null;
  const messages = sessionView.messages ?? [];
  const isStreaming = sessionView.isStreaming ?? false;
  const isThinking = sessionView.isThinking ?? false;
  const isLoading = sessionView.isLoading ?? false;
  const error = sessionView.error ?? null;
  const showTranscriptError = Boolean(error) && showErrorBanner;
  const [planTrackerHeight, setPlanTrackerHeight] = useState(0);
  const planTrackerOpacity = useRef(
    new Animated.Value(hidePlanTracker ? 0 : 1),
  ).current;
  const planTrackerGap = planTrackerHeight > 0 ? 10 : 0;
  const conversationBottomPadding = bottomPadding + planTrackerHeight +
    planTrackerGap;

  useEffect(() => {
    if (!showPlanTracker) setPlanTrackerHeight(0);
  }, [showPlanTracker]);

  useEffect(() => {
    Animated.timing(planTrackerOpacity, {
      toValue: hidePlanTracker ? 0 : 1,
      duration: hidePlanTracker ? 90 : 140,
      useNativeDriver: true,
    }).start();
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

      {showPlanTracker
        ? (
          <Animated.View
            pointerEvents={hidePlanTracker ? "none" : "box-none"}
            style={{
              position: "absolute",
              left: 12,
              right: 12,
              bottom: bottomPadding + 8,
              zIndex: 40,
              opacity: planTrackerOpacity,
            }}
          >
            <PlanTracker
              sessionType={sessionType}
              onHeightChange={setPlanTrackerHeight}
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
          </View>
        )
        : null}
    </View>
  );
}

export const ActiveConversationSurface = memo(
  ActiveConversationSurfaceComponent,
);
