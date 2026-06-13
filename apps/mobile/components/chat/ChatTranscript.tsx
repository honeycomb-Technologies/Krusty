import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  FlatList,
  Keyboard,
  NativeScrollEvent,
  NativeSyntheticEvent,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { ArrowDown } from "lucide-react-native";
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
  scrollToMessageId?: string | null;
  onScrollTargetHandled?: () => void;
}

const TOP_EDGE_HEIGHT = 64;
const BOTTOM_EDGE_HEIGHT = 88;
const EDGE_GAP = 12;
const TRACKER_GAP = 10;
const SCROLL_FOLLOW_THRESHOLD = 72;

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
  const attachmentSignature =
    lastMessage.attachments
      ?.map(
        (attachment) =>
          [
            attachment.type,
            attachment.name ?? "",
            attachment.uri?.length ?? 0,
            attachment.base64?.length ?? 0,
          ].join(":"),
      )
      .join("|") ?? "";

  return [
    lastMessage.id,
    lastMessage.content.length,
    lastMessage.thinking?.length ?? 0,
    attachmentSignature,
    toolSignature,
    lastMessage.isQueued ? "queued" : "steady",
    lastMessage.kind ?? "none",
  ].join("::");
}

function distanceFromBottom(
  contentHeight: number,
  viewportHeight: number,
  offsetY: number,
) {
  return Math.max(0, contentHeight - (offsetY + viewportHeight));
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
  scrollToMessageId,
  onScrollTargetHandled,
}: ChatTranscriptProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const flatListRef = useRef<FlatList>(null);
  const listHeightRef = useRef(0);
  const contentHeightRef = useRef(0);
  const scrollOffsetRef = useRef(0);
  const autoFollowRef = useRef(true);
  const pendingAutoScrollRef = useRef(false);
  const pendingAutoScrollAnimatedRef = useRef(false);
  const isUserDraggingRef = useRef(false);
  const loadedSessionIdRef = useRef<string | null>(null);
  const [planTrackerHeight, setPlanTrackerHeight] = useState(0);
  const [isNearBottom, setIsNearBottom] = useState(true);
  const t = theme.colors;
  const blurTint =
    theme.scheme === "dark"
      ? "systemChromeMaterialDark"
      : "systemChromeMaterialLight";
  const jumpTint =
    theme.scheme === "dark"
      ? "systemMaterialDark"
      : "systemMaterialLight";
  const jumpOverlay =
    theme.scheme === "dark"
      ? "rgba(11,17,25,0.78)"
      : "rgba(255,255,255,0.78)";

  const messageCount = messages.length;
  const layoutSignature = useMemo(
    () => lastMessageLayoutSignature(messages),
    [messages],
  );
  const topFadeHeight = isDesktop ? 22 : TOP_EDGE_HEIGHT;
  const bottomFadeHeight = Math.max(
    isDesktop ? 116 : BOTTOM_EDGE_HEIGHT,
    Math.min(bottomPadding + 40, isDesktop ? 188 : 236),
  );
  const listTopPadding = isDesktop
    ? 8
    : topFadeHeight +
      EDGE_GAP +
      (showPlanTracker && planTrackerHeight > 0
        ? planTrackerHeight + TRACKER_GAP
        : 0);
  const listBottomPadding = bottomPadding + bottomFadeHeight + EDGE_GAP;
  const showJumpToLatest = messageCount > 0 && isStreaming && !isNearBottom;

  const scrollToBottom = useCallback((animated: boolean) => {
    const contentHeight = contentHeightRef.current;
    const listHeight = listHeightRef.current;
    if (!listHeight) {
      return;
    }

    if (!contentHeight || contentHeight <= listHeight) {
      scrollOffsetRef.current = 0;
      setIsNearBottom(true);
      flatListRef.current?.scrollToOffset({ animated: false, offset: 0 });
      return;
    }

    scrollOffsetRef.current = Math.max(0, contentHeight - listHeight);
    setIsNearBottom(true);
    flatListRef.current?.scrollToEnd({ animated });
  }, []);

  const queueAutoScroll = useCallback((animated: boolean) => {
    pendingAutoScrollRef.current = true;
    pendingAutoScrollAnimatedRef.current = animated;
  }, []);

  const updateNearBottom = useCallback(
    (offsetY = scrollOffsetRef.current) => {
      const nextNearBottom =
        distanceFromBottom(
          contentHeightRef.current,
          listHeightRef.current,
          offsetY,
        ) <= SCROLL_FOLLOW_THRESHOLD;

      setIsNearBottom((current) =>
        current === nextNearBottom ? current : nextNearBottom,
      );

      if (nextNearBottom) {
        autoFollowRef.current = true;
      } else if (isUserDraggingRef.current) {
        autoFollowRef.current = false;
      }

      return nextNearBottom;
    },
    [],
  );

  const flushAutoScroll = useCallback(() => {
    if (!pendingAutoScrollRef.current || isUserDraggingRef.current) {
      return;
    }

    if (!autoFollowRef.current) {
      pendingAutoScrollRef.current = false;
      return;
    }

    pendingAutoScrollRef.current = false;
    const animated = pendingAutoScrollAnimatedRef.current;
    requestAnimationFrame(() => {
      scrollToBottom(animated);
    });
  }, [scrollToBottom]);

  const handleListScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      scrollOffsetRef.current = event.nativeEvent.contentOffset.y;
      updateNearBottom(event.nativeEvent.contentOffset.y);
    },
    [updateNearBottom],
  );

  const handleJumpToLatest = useCallback(() => {
    autoFollowRef.current = true;
    setIsNearBottom(true);
    queueAutoScroll(false);
    scrollToBottom(false);
  }, [queueAutoScroll, scrollToBottom]);

  useEffect(() => {
    if (!sessionId) {
      loadedSessionIdRef.current = null;
      autoFollowRef.current = true;
      pendingAutoScrollRef.current = false;
      scrollOffsetRef.current = 0;
      setPlanTrackerHeight(0);
      setIsNearBottom(true);
      return;
    }

    if (loadedSessionIdRef.current === sessionId) {
      return;
    }

    loadedSessionIdRef.current = sessionId;
    autoFollowRef.current = true;
    pendingAutoScrollRef.current = false;
    scrollOffsetRef.current = 0;
    setPlanTrackerHeight(0);
    setIsNearBottom(true);
    queueAutoScroll(false);
  }, [queueAutoScroll, sessionId]);

  useEffect(() => {
    if (messageCount === 0) {
      pendingAutoScrollRef.current = false;
      autoFollowRef.current = true;
      scrollOffsetRef.current = 0;
      setIsNearBottom(true);
      return;
    }

    if (autoFollowRef.current) {
      queueAutoScroll(!isStreaming);
    }
  }, [isStreaming, layoutSignature, messageCount, queueAutoScroll]);

  useEffect(() => {
    if (messageCount === 0 || !autoFollowRef.current) {
      return;
    }

    queueAutoScroll(false);
  }, [
    bottomPadding,
    listTopPadding,
    messageCount,
    planTrackerHeight,
    queueAutoScroll,
  ]);

  useEffect(() => {
    if (!scrollToMessageId) {
      return;
    }

    const targetIndex = messages.findIndex(
      (message) => message.id === scrollToMessageId,
    );
    if (targetIndex < 0) {
      onScrollTargetHandled?.();
      return;
    }

    autoFollowRef.current = false;
    requestAnimationFrame(() => {
      flatListRef.current?.scrollToIndex({
        index: targetIndex,
        animated: true,
        viewPosition: 0.35,
      });
      onScrollTargetHandled?.();
    });
  }, [messages, onScrollTargetHandled, scrollToMessageId]);

  if (messages.length === 0) {
    return (
      <Pressable style={styles.empty} onPress={Keyboard.dismiss}>
        {emptyState}
      </Pressable>
    );
  }

  return (
    <View style={styles.flex}>
      <FlatList
        ref={flatListRef}
        data={messages}
        keyExtractor={(message) => message.id}
        onScrollBeginDrag={() => {
          isUserDraggingRef.current = true;
          Keyboard.dismiss();
        }}
        onScrollEndDrag={() => {
          isUserDraggingRef.current = false;
          updateNearBottom();
          flushAutoScroll();
        }}
        onMomentumScrollEnd={() => {
          isUserDraggingRef.current = false;
          updateNearBottom();
          flushAutoScroll();
        }}
        onScroll={handleListScroll}
        scrollEventThrottle={16}
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
          {
            paddingTop: listTopPadding,
            paddingBottom: listBottomPadding,
          },
        ]}
        onLayout={(event) => {
          listHeightRef.current = event.nativeEvent.layout.height;
          updateNearBottom();
          flushAutoScroll();
        }}
        onContentSizeChange={(_width, height) => {
          contentHeightRef.current = height;
          updateNearBottom();
          flushAutoScroll();
        }}
        onScrollToIndexFailed={({ index }) => {
          const clampedIndex = Math.max(0, Math.min(index, messages.length - 1));
          requestAnimationFrame(() => {
            flatListRef.current?.scrollToIndex({
              index: clampedIndex,
              animated: true,
              viewPosition: 0.35,
            });
          });
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
      {!isDesktop && showPlanTracker ? (
        <PlanTracker onHeightChange={setPlanTrackerHeight} />
      ) : null}
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
      {showJumpToLatest ? (
        <Pressable
          onPress={handleJumpToLatest}
          style={[styles.jumpToLatest, { bottom: bottomPadding + EDGE_GAP }]}
        >
          <BlurView
            intensity={28}
            tint={jumpTint}
            style={StyleSheet.absoluteFill}
          />
          <View
            style={[
              StyleSheet.absoluteFill,
              { backgroundColor: jumpOverlay },
            ]}
          />
          <ArrowDown size={15} color={t.foreground} strokeWidth={2} />
          <Text style={[styles.jumpToLatestText, { color: t.foreground }]}>
            Jump to latest
          </Text>
        </Pressable>
      ) : null}
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
  jumpToLatest: {
    position: "absolute",
    left: 16,
    alignSelf: "flex-start",
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderRadius: 999,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.12)",
    zIndex: 60,
  },
  jumpToLatestText: {
    fontSize: 13,
    fontWeight: "600",
  },
});
