import { useCallback, useEffect, useMemo, useRef, type ReactNode } from "react";
import {
  FlatList,
  Keyboard,
  Pressable,
  StyleSheet,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { LinearGradient } from "../../platform/linear-gradient";
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

function lastMessageContentLength(messages: ChatMessage[]): number {
  return messages[messages.length - 1]?.content?.length ?? 0;
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
  const insets = useSafeAreaInsets();
  const flatListRef = useRef<FlatList>(null);
  const listHeightRef = useRef(0);
  const contentHeightRef = useRef(0);
  const t = theme.colors;

  const messageCount = messages.length;
  const lastContentLength = useMemo(
    () => lastMessageContentLength(messages),
    [messages],
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
  }, [lastContentLength, messageCount, scrollToBottom]);

  if (messages.length === 0) {
    return <Pressable style={styles.empty} onPress={Keyboard.dismiss}>{emptyState}</Pressable>;
  }

  return (
    <View style={styles.flex}>
      <FlatList
        ref={flatListRef}
        data={messages}
        keyExtractor={(_, index) => String(index)}
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
          { paddingBottom: bottomPadding + insets.bottom },
        ]}
        onLayout={(event) => {
          listHeightRef.current = event.nativeEvent.layout.height;
        }}
        onContentSizeChange={(_width, height) => {
          contentHeightRef.current = height;
        }}
        keyboardDismissMode="interactive"
        keyboardShouldPersistTaps="handled"
      />

      <LinearGradient
        colors={[t.background, `${t.background}00`]}
        style={styles.fadeTop}
        pointerEvents="none"
      />
      {!isDesktop && showPlanTracker ? <PlanTracker /> : null}
      <LinearGradient
        colors={[`${t.background}00`, t.background]}
        style={styles.fadeBottom}
        pointerEvents="none"
      />
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
});
