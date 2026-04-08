import { useCallback, useEffect, useMemo, useRef, type ReactNode } from "react";
import {
  FlatList,
  Keyboard,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import { LinearGradient } from "../../platform/linear-gradient";
import { BlurView } from "../../platform/blur";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { MessageBubble } from "./MessageBubble";
import { PlanTracker } from "./PlanTracker";
import type { ChatMessage } from "@krusty/api";

interface ChatTranscriptProps {
  messages: ChatMessage[];
  sessionId?: string | null;
  isStreaming: boolean;
  isThinking?: boolean;
  activeToolCallId?: string | null;
  onApproveTool?: (sessionId: string, toolCallId: string) => void;
  onDenyTool?: (sessionId: string, toolCallId: string) => void;
  onSubmitToolResult?: (
    toolCallId: string,
    result: string,
  ) => void | Promise<void>;
  onPlanConfirm?: (
    toolCallId: string,
    choice: "execute" | "abandon",
  ) => void | Promise<void>;
  emptyState?: ReactNode;
  bottomPadding?: number;
  showPlanTracker?: boolean;
}

function lastMessageLayoutSignature(messages: ChatMessage[]): string {
  const lastMessage = messages[messages.length - 1];
  if (!lastMessage) return "empty";

  const toolSignature =
    lastMessage.toolCalls
      ?.map(
        (toolCall) =>
          [
            toolCall.id,
            toolCall.status,
            toolCall.output?.length ?? 0,
            toolCall.delegated?.thinking?.length ?? 0,
          ].join(":"),
      )
      .join("|") ?? "";

  return [
    lastMessage.id,
    lastMessage.content.length,
    lastMessage.thinking?.length ?? 0,
    toolSignature,
    lastMessage.isQueued ? "queued" : "steady",
    lastMessage.kind ?? "none",
  ].join("::");
}

export function ChatTranscript({
  messages,
  sessionId,
  isStreaming,
  isThinking,
  activeToolCallId,
  onApproveTool,
  onDenyTool,
  onSubmitToolResult,
  onPlanConfirm,
  emptyState,
  bottomPadding = 130,
  showPlanTracker = true,
}: ChatTranscriptProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const flatListRef = useRef<FlatList>(null);
  const listHeightRef = useRef(0);
  const contentHeightRef = useRef(0);
  const t = theme.colors;
  const blurTint =
    theme.scheme === "dark"
      ? "systemChromeMaterialDark"
      : "systemChromeMaterialLight";

  const messageCount = messages.length;
  const layoutSignature = useMemo(
    () => lastMessageLayoutSignature(messages),
    [messages],
  );
  const topFadeHeight = isDesktop ? 22 : 28;
  const bottomFadeHeight = Math.max(
    isDesktop ? 116 : 144,
    Math.min(bottomPadding + 40, isDesktop ? 188 : 236),
  );

  const scrollToBottom = useCallback(() => {
    const contentHeight = contentHeightRef.current;
    const listHeight = listHeightRef.current;
    if (!contentHeight || !listHeight || contentHeight <= listHeight) {
      return;
    }

    flatListRef.current?.scrollToEnd({ animated: !isStreaming });
  }, [isStreaming]);

  useEffect(() => {
    if (messageCount > 0) {
      requestAnimationFrame(scrollToBottom);
    }
  }, [bottomPadding, layoutSignature, messageCount, scrollToBottom]);

  if (messages.length === 0) {
    return <Pressable style={styles.empty} onPress={Keyboard.dismiss}>{emptyState}</Pressable>;
  }

  return (
    <View style={styles.flex}>
      <FlatList
        ref={flatListRef}
        data={messages}
        keyExtractor={(message) => message.id}
        onScrollBeginDrag={Keyboard.dismiss}
        renderItem={({ item, index }) => (
          <MessageBubble
            message={item}
            isLast={index === messages.length - 1}
            isStreaming={isStreaming && index === messages.length - 1}
            isThinking={isThinking && index === messages.length - 1}
            activeToolCallId={activeToolCallId}
            onApproveTool={
              sessionId && onApproveTool
                ? (toolCallId) => onApproveTool(sessionId, toolCallId)
                : undefined
            }
            onDenyTool={
              sessionId && onDenyTool
                ? (toolCallId) => onDenyTool(sessionId, toolCallId)
                : undefined
            }
            onSubmitToolResult={onSubmitToolResult}
            onPlanConfirm={onPlanConfirm}
          />
        )}
        style={styles.flex}
        contentContainerStyle={[
          styles.list,
          isDesktop && styles.listDesktop,
          { paddingBottom: bottomPadding + 16 },
        ]}
        onLayout={(event) => {
          listHeightRef.current = event.nativeEvent.layout.height;
          if (messages.length > 0) {
            requestAnimationFrame(scrollToBottom);
          }
        }}
        onContentSizeChange={(_width, height) => {
          contentHeightRef.current = height;
          if (messages.length > 0) {
            requestAnimationFrame(scrollToBottom);
          }
        }}
        keyboardDismissMode="interactive"
        keyboardShouldPersistTaps="handled"
        showsVerticalScrollIndicator={false}
      />

      <View
        style={[styles.edgeMask, styles.edgeMaskTop, { height: topFadeHeight }]}
        pointerEvents="none"
      >
        <BlurView
          intensity={10}
          tint={blurTint}
          style={StyleSheet.absoluteFill}
        />
        <LinearGradient
          colors={[`${t.background}88`, `${t.background}00`]}
          style={StyleSheet.absoluteFill}
        />
      </View>
      {!isDesktop && showPlanTracker ? <PlanTracker /> : null}
      <View
        style={[
          styles.edgeMask,
          styles.edgeMaskBottom,
          { height: bottomFadeHeight, bottom: 0 },
        ]}
        pointerEvents="none"
      >
        <BlurView
          intensity={28}
          tint={blurTint}
          style={StyleSheet.absoluteFill}
        />
        <LinearGradient
          colors={[`${t.background}00`, `${t.background}d0`, t.background]}
          style={StyleSheet.absoluteFill}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  flex: {
    flex: 1,
  },
  empty: {
    flex: 1,
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
  edgeMask: {
    position: "absolute",
    left: 0,
    right: 0,
  },
  edgeMaskTop: {
    position: "absolute",
    top: 0,
  },
  edgeMaskBottom: {
    position: "absolute",
  },
});
