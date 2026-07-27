import {
  useCallback,
  useEffect,
  memo,
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
  View,
} from "react-native";
import { ArrowDown } from "lucide-react-native";
import { LinearGradient } from "../../platform/linear-gradient";
import { BlurView } from "../../platform/blur";
import { useBreakpoint } from "../../hooks/useBreakpoint";
import { useThemeContext } from "../../hooks/useTheme";
import { MessageBubble } from "./MessageBubble";
import {
  buildTranscriptTurns,
  findTurnIndexForMessage,
  type TranscriptTurn,
} from "./transcriptTurns";
import { PlanTracker } from "./PlanTracker";
import { ConversationSkeleton } from "../ui/Skeleton";
import type { ChatMessage } from "@krusty/api";
import type { SessionType } from "@krusty/api";

interface ChatTranscriptProps {
  messages: ChatMessage[];
  sessionId?: string | null;
  sessionType?: SessionType;
  scrollStateKey?: string;
  isStreaming: boolean;
  isThinking?: boolean;
  isLoading?: boolean;
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
  hideJumpToLatest?: boolean;
  /** When false, skip auto-scroll and interaction work for warm-but-hidden shells. */
  isActive?: boolean;
}

const DESKTOP_TOP_EDGE_HEIGHT = 64;
const MOBILE_TOP_EDGE_HEIGHT = 78;
const DESKTOP_TOP_EDGE_OFFSET = -28;
const MOBILE_TOP_EDGE_OFFSET = -34;
const DESKTOP_BOTTOM_EDGE_HEIGHT = 116;
const MOBILE_BOTTOM_SCRIM_MIN_HEIGHT = 148;
const MOBILE_BOTTOM_SCRIM_MAX_HEIGHT = 228;
const EDGE_GAP = 12;
const TRACKER_GAP = 10;
const SCROLL_FOLLOW_THRESHOLD = 72;
const BOTTOM_SCROLL_OVERSHOOT = 240;
const BOTTOM_CONTROL_INSET = 10;
const BOTTOM_CONTROL_SIZE = 56;
const BOTTOM_CONTROL_RADIUS = 18;
const PROGRAMMATIC_SCROLL_SETTLE_MS = 700;

interface CachedTranscriptScrollState {
  offset: number;
  autoFollow: boolean;
}

const MAX_TRANSCRIPT_SCROLL_CACHE = 40;
const transcriptScrollCache = new Map<string, CachedTranscriptScrollState>();

function setTranscriptScrollCache(
  key: string,
  value: CachedTranscriptScrollState,
) {
  transcriptScrollCache.delete(key);
  transcriptScrollCache.set(key, value);
  while (transcriptScrollCache.size > MAX_TRANSCRIPT_SCROLL_CACHE) {
    const oldest = transcriptScrollCache.keys().next().value;
    if (!oldest) break;
    transcriptScrollCache.delete(oldest);
  }
}


function lastMessageLayoutSignature(messages: ChatMessage[]): string {
  const lastMessage = messages[messages.length - 1];
  if (!lastMessage) return "empty";

  const toolSignature =
    lastMessage.toolCalls
      ?.map((toolCall) =>
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
      ?.map((attachment) =>
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

function ChatTranscriptComponent({
  messages,
  sessionId,
  sessionType = "chat",
  scrollStateKey = sessionId ?? "empty",
  isStreaming,
  isThinking,
  isLoading = false,
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
  hideJumpToLatest = false,
  isActive = true,
}: ChatTranscriptProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const restoredScrollStateRef = useRef(
    transcriptScrollCache.get(scrollStateKey) ?? null,
  );
  const flatListRef = useRef<FlatList>(null);
  const listHeightRef = useRef(0);
  const contentHeightRef = useRef(0);
  const scrollOffsetRef = useRef(
    restoredScrollStateRef.current?.offset ?? 0,
  );
  const autoFollowRef = useRef(
    restoredScrollStateRef.current?.autoFollow ?? true,
  );
  const pendingAutoScrollRef = useRef(false);
  const pendingAutoScrollAnimatedRef = useRef(false);
  const bottomAnchorTimersRef = useRef<ReturnType<typeof setTimeout>[]>([]);
  const bottomAnchorFrameRef = useRef<number | null>(null);
  const isUserDraggingRef = useRef(false);
  const programmaticScrollUntilRef = useRef(0);
  const loadedSessionIdRef = useRef<string | null>(
    restoredScrollStateRef.current
      ? `${scrollStateKey}::${sessionId ?? "new"}`
      : null,
  );
  const [planTrackerHeight, setPlanTrackerHeight] = useState(0);
  const [isNearBottom, setIsNearBottom] = useState(
    restoredScrollStateRef.current?.autoFollow ?? true,
  );
  const t = theme.colors;
  const blurTint =
    theme.scheme === "dark"
      ? "systemChromeMaterialDark"
      : "systemChromeMaterialLight";
  const jumpTint =
    theme.scheme === "dark" ? "systemMaterialDark" : "systemMaterialLight";
  const jumpOverlay =
    theme.scheme === "dark" ? "rgba(11,17,25,0.6)" : "rgba(255,255,255,0.6)";

  const messageCount = messages.length;
  const turns = useMemo(
    () => buildTranscriptTurns(messages, isStreaming),
    [isStreaming, messages],
  );
  const layoutSignature = useMemo(
    () => lastMessageLayoutSignature(messages),
    [messages],
  );
  const topFadeHeight = isDesktop ? DESKTOP_TOP_EDGE_HEIGHT : MOBILE_TOP_EDGE_HEIGHT;
  const topFadeOffset = isDesktop ? DESKTOP_TOP_EDGE_OFFSET : MOBILE_TOP_EDGE_OFFSET;
  // Content starts lower than the raised fade so the blur sits under header chrome.
  const topContentGap = isDesktop ? 28 : 34;
  const bottomScrimHeight = isDesktop
    ? Math.max(DESKTOP_BOTTOM_EDGE_HEIGHT, Math.min(bottomPadding + 40, 188))
    : Math.max(
        MOBILE_BOTTOM_SCRIM_MIN_HEIGHT,
        Math.min(bottomPadding + 96, MOBILE_BOTTOM_SCRIM_MAX_HEIGHT),
      );
  const bottomScrimOffset = 0;
  const listTopPadding = isDesktop
    ? topContentGap
    : topContentGap +
      (showPlanTracker && planTrackerHeight > 0
        ? planTrackerHeight + TRACKER_GAP
        : 0);
  const listBottomPadding = bottomPadding + EDGE_GAP;
  const showJumpToLatest = messageCount > 0 && !isNearBottom && !hideJumpToLatest;

  const clearBottomAnchorTimers = useCallback(() => {
    bottomAnchorTimersRef.current.forEach((timer) => clearTimeout(timer));
    bottomAnchorTimersRef.current = [];
    if (bottomAnchorFrameRef.current !== null) {
      cancelAnimationFrame(bottomAnchorFrameRef.current);
      bottomAnchorFrameRef.current = null;
    }
  }, []);

  const markProgrammaticScroll = useCallback((durationMs = PROGRAMMATIC_SCROLL_SETTLE_MS) => {
    programmaticScrollUntilRef.current = Date.now() + durationMs;
  }, []);

  const scrollToBottom = useCallback((animated: boolean) => {
    const contentHeight = contentHeightRef.current;
    const listHeight = listHeightRef.current;
    if (!listHeight) {
      return;
    }

    if (!contentHeight || contentHeight <= listHeight) {
      scrollOffsetRef.current = 0;
      setIsNearBottom(true);
      markProgrammaticScroll(120);
      flatListRef.current?.scrollToOffset({ animated: false, offset: 0 });
      return;
    }

    const targetOffset = Math.max(
      0,
      contentHeight - listHeight + BOTTOM_SCROLL_OVERSHOOT,
    );
    scrollOffsetRef.current = targetOffset;
    setIsNearBottom(true);
    markProgrammaticScroll(animated ? PROGRAMMATIC_SCROLL_SETTLE_MS : 180);
    flatListRef.current?.scrollToOffset({ animated, offset: targetOffset });
  }, [markProgrammaticScroll]);

  const scheduleBottomAnchor = useCallback(
    (animated: boolean) => {
      clearBottomAnchorTimers();

      const anchor = (useAnimated: boolean) => {
        if (!autoFollowRef.current || isUserDraggingRef.current) {
          return;
        }
        scrollToBottom(useAnimated);
      };

      bottomAnchorFrameRef.current = requestAnimationFrame(() => {
        bottomAnchorFrameRef.current = null;
        anchor(animated);
      });

      // One bounded fallback catches delayed Markdown measurement without
      // issuing a multi-frame scroll storm for every streamed delta.
      bottomAnchorTimersRef.current = [
        setTimeout(() => anchor(false), 120),
      ];
    },
    [clearBottomAnchorTimers, scrollToBottom],
  );

  const queueAutoScroll = useCallback((animated: boolean) => {
    pendingAutoScrollRef.current = true;
    pendingAutoScrollAnimatedRef.current = animated;
  }, []);

  const updateNearBottom = useCallback((
    offsetY = scrollOffsetRef.current,
    options: { allowDisable?: boolean; allowEnable?: boolean } = {},
  ) => {
    const nextNearBottom =
      distanceFromBottom(
        contentHeightRef.current,
        listHeightRef.current,
        offsetY,
      ) <= SCROLL_FOLLOW_THRESHOLD;

    setIsNearBottom((current) =>
      current === nextNearBottom ? current : nextNearBottom,
    );

    if (nextNearBottom && options.allowEnable !== false) {
      autoFollowRef.current = true;
    } else if (!nextNearBottom && options.allowDisable) {
      autoFollowRef.current = false;
    }

    return nextNearBottom;
  }, []);

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
    scheduleBottomAnchor(animated);
  }, [scheduleBottomAnchor]);

  const handleListScroll = useCallback(
    (event: NativeSyntheticEvent<NativeScrollEvent>) => {
      scrollOffsetRef.current = event.nativeEvent.contentOffset.y;
      updateNearBottom(event.nativeEvent.contentOffset.y, {
        allowDisable: Date.now() >= programmaticScrollUntilRef.current,
      });
    },
    [updateNearBottom],
  );

  const handleJumpToLatest = useCallback(() => {
    autoFollowRef.current = true;
    setIsNearBottom(true);
    scrollToBottom(false);
    scheduleBottomAnchor(false);
  }, [scheduleBottomAnchor, scrollToBottom]);

  useEffect(() => clearBottomAnchorTimers, [clearBottomAnchorTimers]);

  useEffect(
    () => () => {
      setTranscriptScrollCache(scrollStateKey, {
        offset: scrollOffsetRef.current,
        autoFollow: autoFollowRef.current,
      });
    },
    [scrollStateKey],
  );

  useEffect(() => {
    if (!sessionId) {
      loadedSessionIdRef.current = null;
      restoredScrollStateRef.current = null;
      autoFollowRef.current = true;
      pendingAutoScrollRef.current = false;
      clearBottomAnchorTimers();
      scrollOffsetRef.current = 0;
      setPlanTrackerHeight(0);
      setIsNearBottom(true);
      return;
    }

    // Identity key includes mode so chat/code/mako shells stay independent.
    const selectionKey = `${scrollStateKey}::${sessionId}`;
    if (loadedSessionIdRef.current === selectionKey) {
      return;
    }

    const cached = transcriptScrollCache.get(scrollStateKey) ?? null;
    restoredScrollStateRef.current = cached;
    loadedSessionIdRef.current = selectionKey;
    autoFollowRef.current = cached?.autoFollow ?? true;
    pendingAutoScrollRef.current = false;
    clearBottomAnchorTimers();
    scrollOffsetRef.current = cached?.offset ?? 0;
    setPlanTrackerHeight(0);
    setIsNearBottom(cached?.autoFollow ?? true);

    // Restore without remounting the FlatList tree. contentOffset only applies
    // on first mount, so explicit scroll is required on thread switches.
    requestAnimationFrame(() => {
      if (!flatListRef.current) {
        return;
      }
      if (cached && cached.offset > 0 && cached.autoFollow === false) {
        markProgrammaticScroll(120);
        flatListRef.current.scrollToOffset({
          animated: false,
          offset: cached.offset,
        });
        return;
      }
      scheduleBottomAnchor(false);
    });
  }, [
    clearBottomAnchorTimers,
    markProgrammaticScroll,
    scheduleBottomAnchor,
    scrollStateKey,
    sessionId,
  ]);

  useEffect(() => {
    if (!isActive) {
      return;
    }

    if (messageCount === 0) {
      pendingAutoScrollRef.current = false;
      autoFollowRef.current = true;
      clearBottomAnchorTimers();
      scrollOffsetRef.current = 0;
      setIsNearBottom(true);
      return;
    }

    if (autoFollowRef.current) {
      queueAutoScroll(!isStreaming);
      scheduleBottomAnchor(!isStreaming);
    }
  }, [
    clearBottomAnchorTimers,
    isActive,
    isStreaming,
    layoutSignature,
    messageCount,
    queueAutoScroll,
    scheduleBottomAnchor,
  ]);

  useEffect(() => {
    if (!isActive || messageCount === 0 || !autoFollowRef.current) {
      return;
    }

    scheduleBottomAnchor(false);
  }, [
    bottomPadding,
    isActive,
    listTopPadding,
    messageCount,
    planTrackerHeight,
    scheduleBottomAnchor,
  ]);

  useEffect(() => {
    if (!isActive || !scrollToMessageId) {
      return;
    }

    const targetIndex = findTurnIndexForMessage(turns, scrollToMessageId);
    if (targetIndex < 0) {
      onScrollTargetHandled?.();
      return;
    }

    autoFollowRef.current = false;
    requestAnimationFrame(() => {
      markProgrammaticScroll();
      flatListRef.current?.scrollToIndex({
        index: targetIndex,
        animated: true,
        viewPosition: 0.35,
      });
      onScrollTargetHandled?.();
    });
  }, [isActive, markProgrammaticScroll, onScrollTargetHandled, scrollToMessageId, turns]);

  const renderTurn = useCallback(
    ({ item, index }: { item: TranscriptTurn; index: number }) => {
      const isLastTurn = index === turns.length - 1;
      return (
        <TranscriptTurnRow
          turn={item}
          isLastTurn={isLastTurn}
          isStreaming={isStreaming && isLastTurn}
          isThinking={isThinking && isLastTurn}
          activeToolCallId={activeToolCallId}
          sessionId={sessionId}
          onApproveTool={onApproveTool}
          onDenyTool={onDenyTool}
          onSubmitToolResult={onSubmitToolResult}
          onPlanConfirm={onPlanConfirm}
        />
      );
    },
    [
      activeToolCallId,
      isStreaming,
      isThinking,
      onApproveTool,
      onDenyTool,
      onPlanConfirm,
      onSubmitToolResult,
      sessionId,
      turns.length,
    ],
  );

  if (messages.length === 0) {
    return (
      <Pressable style={styles.empty} onPress={Keyboard.dismiss}>
        {isLoading ? <ConversationSkeleton /> : emptyState}
      </Pressable>
    );
  }

  return (
    <View style={styles.flex}>
      <FlatList
        ref={flatListRef}
        data={turns}
        keyExtractor={(turn) => turn.id}
        windowSize={6}
        maxToRenderPerBatch={3}
        initialNumToRender={8}
        updateCellsBatchingPeriod={64}
        removeClippedSubviews
        onScrollBeginDrag={() => {
          isUserDraggingRef.current = true;
          clearBottomAnchorTimers();
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
        contentOffset={{
          x: 0,
          y: restoredScrollStateRef.current?.offset ?? 0,
        }}
        renderItem={renderTurn}
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
          const shouldMaintainBottom =
            autoFollowRef.current && !isUserDraggingRef.current;
          updateNearBottom();
          if (shouldMaintainBottom) {
            scheduleBottomAnchor(false);
          } else {
            flushAutoScroll();
          }
        }}
        onContentSizeChange={(_width, height) => {
          contentHeightRef.current = height;
          const shouldMaintainBottom =
            autoFollowRef.current && !isUserDraggingRef.current;
          updateNearBottom();
          if (shouldMaintainBottom) {
            scheduleBottomAnchor(false);
          } else {
            flushAutoScroll();
          }
        }}
        onScrollToIndexFailed={({ index }) => {
          const clampedIndex = Math.max(0, Math.min(index, turns.length - 1));
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
        style={[
          styles.edgeMask,
          styles.edgeMaskTop,
          { height: topFadeHeight, top: topFadeOffset },
        ]}
        pointerEvents="none"
      >
        {isDesktop ? (
          <BlurView
            intensity={18}
            tint={blurTint}
            style={StyleSheet.absoluteFill}
          />
        ) : null}
        <LinearGradient
          colors={
            isDesktop
              ? [t.background, `${t.background}b8`, `${t.background}00`]
              : [
                  t.background,
                  `${t.background}c8`,
                  `${t.background}18`,
                  `${t.background}00`,
                ]
          }
          locations={isDesktop ? [0, 0.42, 1] : [0, 0.28, 0.58, 1]}
          start={{ x: 0.5, y: 0 }}
          end={{ x: 0.5, y: 1 }}
          style={StyleSheet.absoluteFill}
        />
      </View>
      {!isDesktop && showPlanTracker ? (
        <PlanTracker
          sessionType={sessionType}
          onHeightChange={setPlanTrackerHeight}
        />
      ) : null}
      <View
        style={[
          styles.edgeMask,
          styles.edgeMaskBottom,
          { height: bottomScrimHeight, bottom: bottomScrimOffset },
        ]}
        pointerEvents="none"
      >
        {isDesktop ? (
          <BlurView
            intensity={28}
            tint={blurTint}
            style={StyleSheet.absoluteFill}
          />
        ) : null}
        <LinearGradient
          colors={
            isDesktop
              ? [`${t.background}00`, `${t.background}d0`, t.background]
              : [
                  `${t.background}00`,
                  `${t.background}20`,
                  `${t.background}d8`,
                  t.background,
                ]
          }
          locations={isDesktop ? undefined : [0, 0.36, 0.7, 1]}
          start={{ x: 0.5, y: 0 }}
          end={{ x: 0.5, y: 1 }}
          style={StyleSheet.absoluteFill}
        />
      </View>
      {showJumpToLatest ? (
        <View
          pointerEvents="box-none"
          style={[styles.jumpToLatestSlot, { bottom: bottomPadding + EDGE_GAP }]}
        >
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Jump to latest"
            onPress={handleJumpToLatest}
            style={styles.jumpToLatest}
          >
            <BlurView
              intensity={20}
              tint={jumpTint}
              style={StyleSheet.absoluteFill}
            />
            <View
              style={[StyleSheet.absoluteFill, { backgroundColor: jumpOverlay }]}
            />
            <View pointerEvents="none" style={styles.jumpToLatestInner}>
              <ArrowDown size={24} color={t.foreground} strokeWidth={2.2} />
            </View>
          </Pressable>
        </View>
      ) : null}
    </View>
  );
}

export const ChatTranscript = memo(ChatTranscriptComponent);

interface TranscriptTurnRowProps {
  turn: TranscriptTurn;
  isLastTurn: boolean;
  isStreaming: boolean;
  isThinking?: boolean;
  activeToolCallId?: string | null;
  sessionId?: string | null;
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
}

const TranscriptTurnRow = memo(function TranscriptTurnRow({
  turn,
  isLastTurn,
  isStreaming,
  isThinking,
  activeToolCallId,
  sessionId,
  onApproveTool,
  onDenyTool,
  onSubmitToolResult,
  onPlanConfirm,
}: TranscriptTurnRowProps) {
  return (
    <View style={[styles.turn, turn.isLive && styles.turnLive]}>
      {turn.messages.map((message, messageIndex) => {
        const isLastMessageInTurn = messageIndex === turn.messages.length - 1;
        return (
          <MessageBubble
            key={message.id}
            message={message}
            isLast={isLastTurn && isLastMessageInTurn}
            isStreaming={isStreaming && isLastMessageInTurn}
            isThinking={isThinking && isLastMessageInTurn}
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
        );
      })}
    </View>
  );
}, (previous, next) => (
  previous.turn.renderSignature === next.turn.renderSignature &&
  previous.isLastTurn === next.isLastTurn &&
  previous.isStreaming === next.isStreaming &&
  previous.isThinking === next.isThinking &&
  previous.activeToolCallId === next.activeToolCallId &&
  previous.sessionId === next.sessionId &&
  previous.onApproveTool === next.onApproveTool &&
  previous.onDenyTool === next.onDenyTool &&
  previous.onSubmitToolResult === next.onSubmitToolResult &&
  previous.onPlanConfirm === next.onPlanConfirm
));

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
    // Parent desktopChatColumn already caps width; fill the fluid band.
    width: "100%",
    maxWidth: "100%",
    // Slightly tighter side pad so messages expand with the window.
    paddingHorizontal: 12,
  },
  turn: {
    marginBottom: 12,
  },
  turnLive: {
    marginBottom: 14,
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
  jumpToLatestSlot: {
    position: "absolute",
    right: BOTTOM_CONTROL_INSET,
    width: BOTTOM_CONTROL_SIZE,
    alignItems: "center",
    zIndex: 60,
  },
  jumpToLatest: {
    width: BOTTOM_CONTROL_SIZE,
    height: BOTTOM_CONTROL_SIZE,
    alignItems: "center",
    justifyContent: "center",
    position: "relative",
    borderRadius: BOTTOM_CONTROL_RADIUS,
    overflow: "hidden",
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.08)",
  },
  jumpToLatestInner: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    position: "relative",
    zIndex: 1,
  },
});
