import {
  useCallback,
  useEffect,
  memo,
  useLayoutEffect,
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
  findTurnIndexForMessage,
  splitTranscriptTurnsCached,
  turnContainsMessage,
  type TranscriptTurnsCache,
} from "./transcriptTurns";
import {
  findTranscriptRowIndex,
  splitTranscriptRowsCached,
  type TranscriptMessageRow,
  type TranscriptRowsCache,
} from "./transcriptRows";
import { PlanTracker } from "./PlanTracker";
import { ConversationSkeleton } from "../ui/Skeleton";
import type { ChatMessage } from "@krusty/api";
import type { SessionType } from "@krusty/api";
import {
  beginKrustyPerformanceSpan,
  recordKrustyPerformanceMetric,
} from "@krusty/state";
import { summarizeTranscriptRenderBudget } from "./transcriptRenderBudget";

const MAX_COMMITTED_TRANSCRIPT_CACHES = 4;

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
/** Coalesce streaming stick-to-bottom work onto one rAF per animation frame. */
const STREAM_STICK_MIN_INTERVAL_MS = 32;
/** One delayed measurement pass after stream layout settles. */
const STREAM_STICK_FALLBACK_MS = 120;
/** Keep cold mode activation bounded; older turns enter only as the user scrolls up. */
const INITIAL_HISTORICAL_TURN_COUNT = 1;
const HISTORICAL_TURN_PAGE_SIZE = 2;

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
  const flatListRef = useRef<FlatList<TranscriptMessageRow>>(null);
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
  const streamStickFrameRef = useRef<number | null>(null);
  const streamStickThrottleRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const streamStickFallbackRef = useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const streamStickPendingRef = useRef(false);
  const lastStreamStickAtRef = useRef(0);
  const isUserDraggingRef = useRef(false);
  const historyRevealArmedRef = useRef(true);
  const programmaticScrollUntilRef = useRef(0);
  const isStreamingRef = useRef(isStreaming);
  const loadedSessionIdRef = useRef<string | null>(
    restoredScrollStateRef.current
      ? `${scrollStateKey}::${sessionId ?? "new"}`
      : null,
  );
  const committedTurnCachesRef = useRef(
    new Map<string, TranscriptTurnsCache>(),
  );
  const committedRowCachesRef = useRef(
    new Map<string, TranscriptRowsCache>(),
  );
  const finishFirstPaintSpanRef = useRef<(() => number | null) | null>(null);
  const firstPaintGenerationRef = useRef(0);
  const firstPaintFrameRef = useRef<number | null>(null);
  const firstPaintTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
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
  const jumpOverlay = t.surfaceOverlaySubtle;

  isStreamingRef.current = isStreaming;

  const messageCount = messages.length;
  const transcriptCacheKey = `${scrollStateKey}::${sessionId ?? "new"}`;
  const turnSplit = useMemo(
    () => {
      const previous = committedTurnCachesRef.current.get(transcriptCacheKey);
      const cacheState = !previous
        ? "miss"
        : previous.sourceMessages === messages
          && previous.isStreaming === isStreaming
          ? "hit"
          : "tail";
      const finishDeriveSpan = beginKrustyPerformanceSpan(
        "transcript.derive",
        `${sessionType}:${cacheState}`,
      );
      try {
        return splitTranscriptTurnsCached(
          messages,
          isStreaming,
          previous,
        );
      } finally {
        finishDeriveSpan();
      }
    },
    [isStreaming, messages, transcriptCacheKey],
  );
  const { historicalTurns, liveTurn } = turnSplit;
  const cachedScrollState = transcriptScrollCache.get(scrollStateKey) ?? null;
  const initialHistoricalTurnCount =
    cachedScrollState?.autoFollow === false
      ? historicalTurns.length
      : Math.min(INITIAL_HISTORICAL_TURN_COUNT, historicalTurns.length);
  const [historyWindow, setHistoryWindow] = useState(() => ({
    key: transcriptCacheKey,
    count: initialHistoricalTurnCount,
    sourceLength: historicalTurns.length,
    preserveRevealedWindow: cachedScrollState?.autoFollow === false,
  }));
  const visibleHistoricalTurnCount =
    historyWindow.key === transcriptCacheKey
      ? Math.min(historyWindow.count, historicalTurns.length)
      : initialHistoricalTurnCount;
  const hiddenHistoricalTurnCount = Math.max(
    0,
    historicalTurns.length - visibleHistoricalTurnCount,
  );
  const visibleHistoricalTurns = useMemo(
    () =>
      hiddenHistoricalTurnCount > 0
        ? historicalTurns.slice(hiddenHistoricalTurnCount)
        : historicalTurns,
    [
      hiddenHistoricalTurnCount,
      historicalTurns,
    ],
  );
  const rowSplit = useMemo(
    () => splitTranscriptRowsCached(
      visibleHistoricalTurns,
      liveTurn,
      committedRowCachesRef.current.get(transcriptCacheKey),
    ),
    [liveTurn, transcriptCacheKey, visibleHistoricalTurns],
  );
  const { rows: transcriptRows, liveFooterRow } = rowSplit;
  const visibleLatestTurn =
    liveTurn
    ?? visibleHistoricalTurns[visibleHistoricalTurns.length - 1]
    ?? null;
  const visibleLatestTurnBudget = useMemo(
    () =>
      visibleLatestTurn
        ? summarizeTranscriptRenderBudget(visibleLatestTurn.messages)
        : null,
    [visibleLatestTurn],
  );
  useEffect(() => {
    if (isStreaming || !visibleLatestTurnBudget) return;
    recordKrustyPerformanceMetric("transcript.visible_messages", {
      count: visibleLatestTurnBudget.messageCount,
    });
    recordKrustyPerformanceMetric("transcript.visible_render_parts", {
      count: visibleLatestTurnBudget.renderPartCount,
    });
    recordKrustyPerformanceMetric("transcript.visible_tools", {
      count: visibleLatestTurnBudget.toolCount,
    });
    recordKrustyPerformanceMetric("transcript.visible_markdown_characters", {
      count: visibleLatestTurnBudget.markdownCharacterCount,
    });
  }, [isStreaming, visibleLatestTurnBudget]);
  useEffect(() => {
    setHistoryWindow((current) => {
      if (current.key !== transcriptCacheKey) {
        return {
          key: transcriptCacheKey,
          count: initialHistoricalTurnCount,
          sourceLength: historicalTurns.length,
          preserveRevealedWindow: cachedScrollState?.autoFollow === false,
        };
      }

      if (current.sourceLength === historicalTurns.length) {
        if (current.count === 0 && initialHistoricalTurnCount > 0) {
          return {
            ...current,
            count: initialHistoricalTurnCount,
          };
        }
        return current;
      }

      let count = Math.min(current.count, historicalTurns.length);
      if (
        historicalTurns.length > current.sourceLength
        && current.preserveRevealedWindow
      ) {
        // Keep deliberately revealed rows mounted when the former live turn
        // becomes historical. Default auto-follow stays bounded at one row;
        // only an explicit upward page/scroll target grows with new turns.
        count = Math.min(
          historicalTurns.length,
          current.count + historicalTurns.length - current.sourceLength,
        );
      } else if (count === 0 && initialHistoricalTurnCount > 0) {
        count = initialHistoricalTurnCount;
      }

      return {
        key: transcriptCacheKey,
        count,
        sourceLength: historicalTurns.length,
        preserveRevealedWindow: current.preserveRevealedWindow,
      };
    });
  }, [
    cachedScrollState?.autoFollow,
    historicalTurns.length,
    initialHistoricalTurnCount,
    transcriptCacheKey,
  ]);
  useEffect(() => {
    const caches = committedTurnCachesRef.current;
    caches.delete(transcriptCacheKey);
    caches.set(transcriptCacheKey, turnSplit.cache);
    while (caches.size > MAX_COMMITTED_TRANSCRIPT_CACHES) {
      const oldestKey = caches.keys().next().value;
      if (oldestKey === undefined) break;
      caches.delete(oldestKey);
    }
  }, [transcriptCacheKey, turnSplit.cache]);
  useEffect(() => {
    const caches = committedRowCachesRef.current;
    caches.delete(transcriptCacheKey);
    caches.set(transcriptCacheKey, rowSplit.cache);
    while (caches.size > MAX_COMMITTED_TRANSCRIPT_CACHES) {
      const oldestKey = caches.keys().next().value;
      if (oldestKey === undefined) break;
      caches.delete(oldestKey);
    }
  }, [rowSplit.cache, transcriptCacheKey]);
  const finishFirstPaint = useCallback((generation?: number) => {
    if (
      generation !== undefined
      && generation !== firstPaintGenerationRef.current
    ) {
      return;
    }
    if (firstPaintFrameRef.current !== null) {
      cancelAnimationFrame(firstPaintFrameRef.current);
      firstPaintFrameRef.current = null;
    }
    if (firstPaintTimeoutRef.current !== null) {
      clearTimeout(firstPaintTimeoutRef.current);
      firstPaintTimeoutRef.current = null;
    }
    finishFirstPaintSpanRef.current?.();
    finishFirstPaintSpanRef.current = null;
  }, []);
  const markFirstPaintReady = useCallback(() => {
    if (
      !finishFirstPaintSpanRef.current
      || firstPaintFrameRef.current !== null
    ) {
      return;
    }
    const generation = firstPaintGenerationRef.current;
    firstPaintFrameRef.current = requestAnimationFrame(() => {
      firstPaintFrameRef.current = null;
      finishFirstPaint(generation);
    });
  }, [finishFirstPaint]);
  useLayoutEffect(() => {
    finishFirstPaint();
    const generation = firstPaintGenerationRef.current + 1;
    firstPaintGenerationRef.current = generation;
    finishFirstPaintSpanRef.current = beginKrustyPerformanceSpan(
      "transcript.first_paint",
      transcriptCacheKey,
    );
    // A stable same-size FlatList may not emit another layout callback. Never
    // let a stale span survive until an unrelated session change.
    firstPaintTimeoutRef.current = setTimeout(
      () => finishFirstPaint(generation),
      1_000,
    );
    return () => {
      finishFirstPaint(generation);
    };
  }, [finishFirstPaint, transcriptCacheKey]);
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
    if (streamStickFrameRef.current !== null) {
      cancelAnimationFrame(streamStickFrameRef.current);
      streamStickFrameRef.current = null;
    }
    if (streamStickThrottleRef.current !== null) {
      clearTimeout(streamStickThrottleRef.current);
      streamStickThrottleRef.current = null;
    }
    if (streamStickFallbackRef.current !== null) {
      clearTimeout(streamStickFallbackRef.current);
      streamStickFallbackRef.current = null;
    }
    streamStickPendingRef.current = false;
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

  const stickToBottomNow = useCallback(
    (animated: boolean) => {
      if (!autoFollowRef.current || isUserDraggingRef.current) {
        return;
      }
      scrollToBottom(animated);
    },
    [scrollToBottom],
  );

  /**
   * Streaming stick path: coalesce content-size / layout / delta noise into at
   * most one rAF stick per interval while auto-follow remains enabled.
   */
  const requestStreamStick = useCallback(() => {
    if (!autoFollowRef.current || isUserDraggingRef.current) {
      return;
    }

    streamStickPendingRef.current = true;

    const runStick = () => {
      if (!autoFollowRef.current || isUserDraggingRef.current) {
        streamStickPendingRef.current = false;
        return;
      }
      streamStickPendingRef.current = false;
      lastStreamStickAtRef.current = Date.now();
      stickToBottomNow(false);
    };

    const scheduleFrame = () => {
      if (streamStickFrameRef.current !== null) {
        return;
      }
      streamStickFrameRef.current = requestAnimationFrame(() => {
        streamStickFrameRef.current = null;
        if (!streamStickPendingRef.current) {
          return;
        }
        runStick();
      });
    };

    const elapsed = Date.now() - lastStreamStickAtRef.current;
    if (elapsed >= STREAM_STICK_MIN_INTERVAL_MS) {
      scheduleFrame();
    } else if (streamStickThrottleRef.current === null) {
      streamStickThrottleRef.current = setTimeout(() => {
        streamStickThrottleRef.current = null;
        if (!streamStickPendingRef.current) return;
        scheduleFrame();
      }, STREAM_STICK_MIN_INTERVAL_MS - elapsed);
    }

    // One delayed measurement pass for late Markdown/layout growth. This has a
    // separate slot from the throttle so neither timer suppresses the other.
    if (streamStickFallbackRef.current !== null) {
      clearTimeout(streamStickFallbackRef.current);
    }
    streamStickFallbackRef.current = setTimeout(() => {
      streamStickFallbackRef.current = null;
      if (!autoFollowRef.current || isUserDraggingRef.current) return;
      streamStickPendingRef.current = true;
      scheduleFrame();
    }, STREAM_STICK_FALLBACK_MS);
  }, [stickToBottomNow]);

  const scheduleBottomAnchor = useCallback(
    (animated: boolean) => {
      // Streaming auto-follow uses the throttled rAF stick path so content-size
      // blips do not issue a scroll storm.
      if (isStreamingRef.current) {
        requestStreamStick();
        return;
      }

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
        setTimeout(() => anchor(false), STREAM_STICK_FALLBACK_MS),
      ];
    },
    [clearBottomAnchorTimers, requestStreamStick, scrollToBottom],
  );

  const queueAutoScroll = useCallback((animated: boolean) => {
    pendingAutoScrollRef.current = true;
    pendingAutoScrollAnimatedRef.current = animated;
  }, []);

  const revealOlderHistory = useCallback(() => {
    if (
      hiddenHistoricalTurnCount === 0
      || !historyRevealArmedRef.current
    ) {
      return;
    }
    historyRevealArmedRef.current = false;

    setHistoryWindow((current) => {
      const currentCount =
        current.key === transcriptCacheKey
          ? current.count
          : initialHistoricalTurnCount;
      return {
        key: transcriptCacheKey,
        count: Math.min(
          historicalTurns.length,
          currentCount + HISTORICAL_TURN_PAGE_SIZE,
        ),
        sourceLength: historicalTurns.length,
        preserveRevealedWindow: true,
      };
    });
  }, [
    hiddenHistoricalTurnCount,
    historicalTurns.length,
    initialHistoricalTurnCount,
    transcriptCacheKey,
  ]);

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
      const offsetY = event.nativeEvent.contentOffset.y;
      scrollOffsetRef.current = offsetY;
      if (
        isUserDraggingRef.current
        && offsetY <= SCROLL_FOLLOW_THRESHOLD
      ) {
        revealOlderHistory();
      } else if (offsetY > SCROLL_FOLLOW_THRESHOLD * 2) {
        historyRevealArmedRef.current = true;
      }
      updateNearBottom(offsetY, {
        allowDisable: Date.now() >= programmaticScrollUntilRef.current,
      });
    },
    [revealOlderHistory, updateNearBottom],
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
      historyRevealArmedRef.current = true;
      autoFollowRef.current = true;
      pendingAutoScrollRef.current = false;
      clearBottomAnchorTimers();
      scrollOffsetRef.current = 0;
      contentHeightRef.current = 0;
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
    historyRevealArmedRef.current = true;
    autoFollowRef.current = cached?.autoFollow ?? true;
    pendingAutoScrollRef.current = false;
    clearBottomAnchorTimers();
    scrollOffsetRef.current = cached?.offset ?? 0;
    contentHeightRef.current = 0;
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

    if (!autoFollowRef.current) {
      return;
    }

    // While streaming, only queue/coalesce stick work. Avoid dual schedule
    // from both layoutSignature ticks and onContentSizeChange.
    if (isStreaming) {
      queueAutoScroll(false);
      requestStreamStick();
      return;
    }

    queueAutoScroll(true);
    scheduleBottomAnchor(true);
  }, [
    clearBottomAnchorTimers,
    isActive,
    isStreaming,
    layoutSignature,
    messageCount,
    queueAutoScroll,
    requestStreamStick,
    scheduleBottomAnchor,
  ]);

  useEffect(() => {
    if (!isActive || messageCount === 0 || !autoFollowRef.current) {
      return;
    }

    // Padding/tracker geometry changes still need an immediate re-anchor, but
    // stream mode stays on the coalesced stick path.
    if (isStreaming) {
      requestStreamStick();
      return;
    }

    scheduleBottomAnchor(false);
  }, [
    bottomPadding,
    isActive,
    isStreaming,
    listTopPadding,
    messageCount,
    planTrackerHeight,
    requestStreamStick,
    scheduleBottomAnchor,
  ]);

  useEffect(() => {
    if (!isActive || !scrollToMessageId) {
      return;
    }

    if (turnContainsMessage(liveTurn, scrollToMessageId)) {
      autoFollowRef.current = false;
      requestAnimationFrame(() => {
        markProgrammaticScroll();
        const rowIndex = findTranscriptRowIndex(
          transcriptRows,
          scrollToMessageId,
        );
        if (rowIndex >= 0) {
          flatListRef.current?.scrollToIndex({
            index: rowIndex,
            animated: true,
            viewPosition: 0.35,
          });
        } else {
          // Only the actively changing tail message remains in the footer.
          flatListRef.current?.scrollToEnd({ animated: true });
        }
        onScrollTargetHandled?.();
      });
      return;
    }

    const targetIndex = findTurnIndexForMessage(
      historicalTurns,
      scrollToMessageId,
    );
    if (targetIndex < 0) {
      onScrollTargetHandled?.();
      return;
    }

    const firstVisibleHistoricalIndex =
      historicalTurns.length - visibleHistoricalTurns.length;
    if (targetIndex < firstVisibleHistoricalIndex) {
      setHistoryWindow({
        key: transcriptCacheKey,
        count: historicalTurns.length - targetIndex,
        sourceLength: historicalTurns.length,
        preserveRevealedWindow: true,
      });
      return;
    }

    autoFollowRef.current = false;
    requestAnimationFrame(() => {
      markProgrammaticScroll();
      const visibleRowIndex = findTranscriptRowIndex(
        transcriptRows,
        scrollToMessageId,
      );
      if (visibleRowIndex < 0) {
        onScrollTargetHandled?.();
        return;
      }
      flatListRef.current?.scrollToIndex({
        index: visibleRowIndex,
        animated: true,
        viewPosition: 0.35,
      });
      onScrollTargetHandled?.();
    });
  }, [
    historicalTurns,
    isActive,
    liveTurn,
    markProgrammaticScroll,
    onScrollTargetHandled,
    scrollToMessageId,
    transcriptRows,
    transcriptCacheKey,
    visibleHistoricalTurns.length,
  ]);

  const renderTranscriptRow = useCallback(
    ({ item }: { item: TranscriptMessageRow }) => (
      <TranscriptMessageRowView
        row={item}
        isLastTranscriptMessage={false}
        isStreaming={false}
        isThinking={false}
        activeToolCallId={activeToolCallId}
        sessionId={sessionId}
        onApproveTool={onApproveTool}
        onDenyTool={onDenyTool}
        onSubmitToolResult={onSubmitToolResult}
        onPlanConfirm={onPlanConfirm}
      />
    ),
    [
      activeToolCallId,
      onApproveTool,
      onDenyTool,
      onPlanConfirm,
      onSubmitToolResult,
      sessionId,
    ],
  );

  const liveFooter = useMemo(() => {
    if (!liveFooterRow) {
      return <View style={styles.liveFooterSpacer} />;
    }

    return (
      <View style={styles.liveFooter}>
        <TranscriptMessageRowView
          row={liveFooterRow}
          isLastTranscriptMessage
          isStreaming={isStreaming}
          isThinking={isThinking}
          activeToolCallId={activeToolCallId}
          sessionId={sessionId}
          onApproveTool={onApproveTool}
          onDenyTool={onDenyTool}
          onSubmitToolResult={onSubmitToolResult}
          onPlanConfirm={onPlanConfirm}
        />
      </View>
    );
  }, [
    activeToolCallId,
    isStreaming,
    isThinking,
    liveFooterRow,
    onApproveTool,
    onDenyTool,
    onPlanConfirm,
    onSubmitToolResult,
    sessionId,
  ]);

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
        data={transcriptRows}
        keyExtractor={(row) => row.id}
        // Completed messages are isolated from live tail stream ticks.
        extraData={liveFooterRow?.id ?? "no-live"}
        windowSize={3}
        maxToRenderPerBatch={1}
        initialNumToRender={1}
        updateCellsBatchingPeriod={80}
        // removeClippedSubviews is a known native crash source on iOS New
        // Architecture with nested message/tool cells and absolute chrome.
        removeClippedSubviews={false}
        onScrollBeginDrag={() => {
          isUserDraggingRef.current = true;
          historyRevealArmedRef.current = true;
          clearBottomAnchorTimers();
          if (scrollOffsetRef.current <= SCROLL_FOLLOW_THRESHOLD) {
            revealOlderHistory();
          }
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
        renderItem={renderTranscriptRow}
        ListFooterComponent={liveFooter}
        maintainVisibleContentPosition={{ minIndexForVisible: 0 }}
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
          markFirstPaintReady();
          listHeightRef.current = event.nativeEvent.layout.height;
          const shouldMaintainBottom =
            autoFollowRef.current && !isUserDraggingRef.current;
          updateNearBottom();
          if (shouldMaintainBottom) {
            if (isStreamingRef.current) {
              requestStreamStick();
            } else {
              scheduleBottomAnchor(false);
            }
          } else {
            flushAutoScroll();
          }
        }}
        onContentSizeChange={(_width, height) => {
          markFirstPaintReady();
          // Ignore no-op measurement blips while streaming.
          if (
            isStreamingRef.current &&
            height === contentHeightRef.current
          ) {
            return;
          }
          contentHeightRef.current = height;
          const shouldMaintainBottom =
            autoFollowRef.current && !isUserDraggingRef.current;
          updateNearBottom();
          if (shouldMaintainBottom) {
            if (isStreamingRef.current) {
              requestStreamStick();
            } else {
              scheduleBottomAnchor(false);
            }
          } else {
            flushAutoScroll();
          }
        }}
        onScrollToIndexFailed={({ index }) => {
          const clampedIndex = Math.max(
            0,
            Math.min(index, Math.max(transcriptRows.length - 1, 0)),
          );
          requestAnimationFrame(() => {
            if (transcriptRows.length === 0) {
              flatListRef.current?.scrollToEnd({ animated: true });
              return;
            }
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

interface TranscriptMessageRowViewProps {
  row: TranscriptMessageRow;
  isLastTranscriptMessage: boolean;
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

const TranscriptMessageRowView = memo(function TranscriptMessageRowView({
  row,
  isLastTranscriptMessage,
  isStreaming,
  isThinking,
  activeToolCallId,
  sessionId,
  onApproveTool,
  onDenyTool,
  onSubmitToolResult,
  onPlanConfirm,
}: TranscriptMessageRowViewProps) {
  return (
    <View
      style={[
        row.isLastMessageInTurn && styles.turn,
        row.isLastMessageInTurn && row.isLive && styles.turnLive,
      ]}
    >
      <MessageBubble
        message={row.message}
        isLast={isLastTranscriptMessage}
        isStreaming={isStreaming}
        isThinking={isThinking}
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
    </View>
  );
}, (previous, next) => (
  previous.row.renderSignature === next.row.renderSignature &&
  previous.row.isLive === next.row.isLive &&
  previous.row.isLastMessageInTurn === next.row.isLastMessageInTurn &&
  previous.isLastTranscriptMessage === next.isLastTranscriptMessage &&
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
  liveFooter: {
    // Keep footer spacing identical to previous in-list last turn spacing.
  },
  liveFooterSpacer: {
    height: 0,
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
