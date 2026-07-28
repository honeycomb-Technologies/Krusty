import { memo, useEffect, useMemo, useRef, type ReactNode } from "react";
import { Text, View } from "react-native";
import { useShallow } from "zustand/react/shallow";
import type { SessionType } from "@krusty/api";

import { ChatTranscript } from "../../../components/chat/ChatTranscript";
import { getToolDiffStats } from "../../../components/chat/toolDiffModel";
import { KrustyLogo } from "../../../components/ui/KrustyLogo";
import { useSessionStore } from "../../../hooks/useStores";
import { useThemeContext } from "../../../hooks/useTheme";
import {
  flattenToolCalls,
  getActiveToolCall,
  getLastAssistantMessage,
} from "./helpers";
import { styles } from "./styles";

interface ActiveConversationSurfaceProps {
  activeMode: SessionType;
  sessionType: "chat" | "code";
  activeToolCallId: string | null;
  bottomPadding: number;
  hideJumpToLatest?: boolean;
  showErrorBanner?: boolean;
  errorBannerHeight: number;
  onErrorBannerHeightChange: (height: number) => void;
  onApproveTool: (sessionId: string, toolCallId: string) => void;
  onDenyTool: (sessionId: string, toolCallId: string) => void;
  onSubmitToolResult: (toolCallId: string, result: string) => void;
  onPlanConfirm: (toolCallId: string, choice: "execute" | "abandon") => void;
  /**
   * Optional bridge for shell-level side effects that still need semantic
   * stream summaries (live activity / approvals). Keep this narrow.
   */
  onStreamSemantics?: (semantics: {
    sessionId: string | null;
    isStreaming: boolean;
    isThinking: boolean;
    title: string | null;
    tokenCount: number;
    lastAssistantSnippet: string;
    awaitingApprovalCalls: ReturnType<typeof flattenToolCalls>;
    activeToolCall: ReturnType<typeof getActiveToolCall>;
    activityDiff: { additions: number; deletions: number };
  }) => void;
  emptyState?: ReactNode;
}

function ActiveConversationSurfaceComponent({
  activeMode,
  sessionType,
  activeToolCallId,
  bottomPadding,
  hideJumpToLatest = false,
  showErrorBanner = true,
  errorBannerHeight,
  onErrorBannerHeightChange,
  onApproveTool,
  onDenyTool,
  onSubmitToolResult,
  onPlanConfirm,
  onStreamSemantics,
  emptyState,
}: ActiveConversationSurfaceProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const toolActivityRef = useRef<{
    signature: string;
    toolCalls: ReturnType<typeof flattenToolCalls>;
    awaitingApprovalCalls: ReturnType<typeof flattenToolCalls>;
    activeToolCall: ReturnType<typeof getActiveToolCall>;
    activityDiff: { additions: number; deletions: number };
  } | null>(null);

  // Messages live here only. The outer shell must not subscribe to them.
  const sessionView = useSessionStore(
    useShallow((state) => ({
      sessionId: state.sessionId,
      title: state.title,
      messages: state.messages,
      isStreaming: state.isStreaming,
      isThinking: state.isThinking,
      tokenCount: state.tokenCount,
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
  const tokenCount = sessionView.tokenCount ?? 0;
  const title = sessionView.title ?? null;

  const toolActivity = useMemo(() => {
    const toolCalls = flattenToolCalls(messages);
    const previous = toolActivityRef.current;
    const signature = toolCalls
      .map((toolCall) =>
        [
          toolCall.id,
          toolCall.status,
          toolCall.output?.length ?? 0,
          toolCall.delegated?.thinking?.length ?? 0,
        ].join(":"),
      )
      .join("|");
    if (previous && previous.signature === signature) {
      return previous;
    }
    const next = {
      signature,
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

  const lastAssistantSnippet = useMemo(() => {
    const last = getLastAssistantMessage(messages);
    const content = last?.content?.trim() ?? "";
    return content.length > 180 ? `${content.slice(0, 177)}...` : content;
  }, [messages]);

  // Publish narrow semantics without forcing parent message subscription.
  useEffect(() => {
    onStreamSemantics?.({
      sessionId,
      isStreaming,
      isThinking,
      title,
      tokenCount,
      lastAssistantSnippet,
      awaitingApprovalCalls: toolActivity.awaitingApprovalCalls,
      activeToolCall: toolActivity.activeToolCall,
      activityDiff: toolActivity.activityDiff,
    });
  }, [
    isStreaming,
    isThinking,
    lastAssistantSnippet,
    onStreamSemantics,
    sessionId,
    title,
    tokenCount,
    toolActivity,
  ]);

  const showTranscriptError = Boolean(error) && showErrorBanner;

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
        emptyState={
          emptyState ?? (
            <View style={styles.empty}>
              <KrustyLogo />
              {error ? (
                <Text style={[styles.emptyHint, { color: t.error }]}>{error}</Text>
              ) : null}
            </View>
          )
        }
        bottomPadding={
          bottomPadding + (showTranscriptError ? errorBannerHeight + 10 : 0)
        }
        hideJumpToLatest={hideJumpToLatest}
      />

      {showTranscriptError ? (
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
              bottom: bottomPadding + 10,
              marginBottom: 0,
              zIndex: 30,
              borderColor: `${t.error}40`,
              backgroundColor: `${t.error}14`,
            },
          ]}
        >
          <Text selectable style={[styles.errorBannerText, { color: t.error }]}>
            {error}
          </Text>
        </View>
      ) : null}
    </View>
  );
}

export const ActiveConversationSurface = memo(ActiveConversationSurfaceComponent);
