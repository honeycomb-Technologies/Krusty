import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFocusEffect, useRouter } from "expo-router";
import {
  Platform,
  Pressable,
  StyleSheet,
  Text,
  useWindowDimensions,
  View,
} from "react-native";
import type { ViewStyle } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, {
  Easing,
  runOnJS,
  useAnimatedStyle,
  useSharedValue,
  withSpring,
  withTiming,
} from "react-native-reanimated";
import { useSafeAreaInsets } from "react-native-safe-area-context";

import type {
  ChatMessage,
  DelegationEventResponse,
  DelegationTaskState,
  DelegationTaskStateResponse,
  ToolCall,
} from "@mitsuro/api";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { ChatTranscript } from "./ChatTranscript";

interface AgentConversationScreenProps {
  sessionId: string;
  groupId: string;
  taskId: string;
  fallbackName?: string;
  openedFromParent?: boolean;
}

const ACTIVE_TASK_STATES = new Set([
  "created",
  "queued",
  "leased",
  "running",
  "retrying",
]);
const TASK_STATES = new Set<DelegationTaskState>([
  "created",
  "queued",
  "leased",
  "running",
  "retrying",
  "complete",
  "degraded",
  "failed",
  "cancelled",
]);
const MAX_CONVERSATION_EVENTS = 600;
const DISMISS_GESTURE_ACTIVATION_DISTANCE = 4;
const DISMISS_GESTURE_DISTANCE = 40;
const DISMISS_GESTURE_VELOCITY = 350;
const DISMISS_SPRING = {
  damping: 22,
  stiffness: 240,
  mass: 0.8,
};
const webGrabberStyle = Platform.OS === "web"
  ? ({ touchAction: "none" } as unknown as ViewStyle)
  : undefined;

interface ConversationToolCallPayload {
  id: string;
  name: string;
  arguments?: Record<string, unknown>;
}

function visibleConversationContent(value: unknown): string {
  if (typeof value !== "string") return "";
  const handoffStart = value.indexOf("<delegated_handoff>");
  const reportStart = value.indexOf("<explore_report>");
  const starts = [handoffStart, reportStart].filter((index) => index >= 0);
  const structuredStart = starts.length > 0 ? Math.min(...starts) : -1;
  return (
    structuredStart >= 0 ? value.slice(0, structuredStart) : value
  ).trimEnd();
}

function mergeEvents(
  current: DelegationEventResponse[],
  incoming: DelegationEventResponse[],
): DelegationEventResponse[] {
  const byId = new Map(current.map((event) => [event.event_id, event]));
  for (const event of incoming) byId.set(event.event_id, event);
  return [...byId.values()]
    .sort((a, b) => a.event_id - b.event_id)
    .slice(-MAX_CONVERSATION_EVENTS);
}

function conversationMessages(
  events: DelegationEventResponse[],
): ChatMessage[] {
  const messages = new Map<string, ChatMessage>();
  const order: string[] = [];

  for (const event of events) {
    if (event.event_type !== "task_conversation") continue;
    const payload = event.payload;
    const kind = typeof payload.kind === "string" ? payload.kind : "";
    const messageId =
      typeof payload.message_id === "string" ? payload.message_id : "";
    if (!messageId) continue;

    if (kind === "assistant_turn") {
      const existing = messages.get(messageId);
      const calls = Array.isArray(payload.tool_calls)
        ? payload.tool_calls.flatMap((value): ToolCall[] => {
            if (!value || typeof value !== "object") return [];
            const call = value as unknown as ConversationToolCallPayload;
            if (typeof call.id !== "string" || typeof call.name !== "string")
              return [];
            const previous = existing?.toolCalls?.find(
              (item) => item.id === call.id,
            );
            return [
              {
                id: call.id,
                name: call.name,
                arguments: call.arguments,
                output: previous?.output,
                status: previous?.status ?? "running",
              },
            ];
          })
        : [];
      if (!messages.has(messageId)) order.push(messageId);
      messages.set(messageId, {
        id: messageId,
        role: "assistant",
        content: visibleConversationContent(payload.content),
        toolCalls: calls,
      });
      continue;
    }

    if (kind === "tool_result") {
      const toolCallId =
        typeof payload.tool_call_id === "string" ? payload.tool_call_id : "";
      if (!toolCallId) continue;
      let message = messages.get(messageId);
      if (!message) {
        order.push(messageId);
        message = {
          id: messageId,
          role: "assistant",
          content: "",
          toolCalls: [],
        };
      }
      const calls = [...(message.toolCalls ?? [])];
      const index = calls.findIndex((call) => call.id === toolCallId);
      const nextCall: ToolCall = {
        ...(index >= 0
          ? calls[index]
          : {
              id: toolCallId,
              name: typeof payload.name === "string" ? payload.name : "tool",
              status: "running",
            }),
        output: typeof payload.output === "string" ? payload.output : "",
        status: payload.is_error === true ? "error" : "success",
      };
      if (index >= 0) calls[index] = nextCall;
      else calls.push(nextCall);
      messages.set(messageId, { ...message, toolCalls: calls });
    }
  }

  return order.flatMap((id) => {
    const message = messages.get(id);
    return message ? [message] : [];
  });
}

function taskStateFromEvent(
  event: DelegationEventResponse,
): DelegationTaskState | null {
  const state = event.payload.state;
  if (
    typeof state === "string" &&
    TASK_STATES.has(state as DelegationTaskState)
  ) {
    return state as DelegationTaskState;
  }
  if (event.event_type === "task_claimed") return "leased";
  if (event.event_type === "task_running") return "running";
  return null;
}

export function AgentConversationScreen({
  sessionId,
  groupId,
  taskId,
  fallbackName,
  openedFromParent = false,
}: AgentConversationScreenProps) {
  const router = useRouter();
  const insets = useSafeAreaInsets();
  const { height } = useWindowDimensions();
  const sheetHeight = Math.round(height * 0.92);
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [task, setTask] = useState<DelegationTaskStateResponse | null>(null);
  const [events, setEvents] = useState<DelegationEventResponse[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const slideY = useSharedValue(-sheetHeight);
  const closingRef = useRef(false);

  const restoreParentSession = useCallback(async () => {
    let focus: "chat" | "code" | "hive" | undefined;
    try {
      const parent = await client?.getSession(sessionId);
      const sessionType = parent?.session.session_type;
      if (
        sessionType === "chat" ||
        sessionType === "code" ||
        sessionType === "hive"
      ) {
        focus = sessionType;
      }
    } catch {
      // The explicit session ID still provides a safe fallback when the
      // lightweight metadata lookup is unavailable during reconnection.
    }
    router.replace({
      pathname: "/",
      params: { sessionId, ...(focus ? { focus } : {}) },
    });
  }, [client, router, sessionId]);

  useEffect(() => {
    slideY.value = -sheetHeight;
    slideY.value = withTiming(0, {
      duration: 240,
      easing: Easing.out(Easing.cubic),
    });
  }, [sheetHeight, slideY]);

  const drawerAnimatedStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: slideY.value }],
  }));

  const finishClose = useCallback(() => {
    // Normal card navigation leaves the parent transcript mounted beneath
    // this transparent modal. Pop that history entry so its exact scroll
    // position survives. Deep links and restored routes have no trustworthy
    // parent entry, so they still use the explicit session fallback.
    if (openedFromParent && router.canGoBack()) {
      router.back();
    } else {
      void restoreParentSession();
    }
  }, [openedFromParent, restoreParentSession, router]);

  const close = useCallback(() => {
    if (closingRef.current) return;
    closingRef.current = true;
    slideY.value = withTiming(-sheetHeight, {
      duration: 200,
      easing: Easing.in(Easing.cubic),
    }, (finished) => {
      if (finished) {
        runOnJS(finishClose)();
      }
    });
  }, [finishClose, sheetHeight, slideY]);

  const dismissGesture = Gesture.Pan()
    .activeOffsetY(-DISMISS_GESTURE_ACTIVATION_DISTANCE)
    .failOffsetX([-24, 24])
    .onUpdate((event) => {
      slideY.value = Math.min(0, event.translationY);
    })
    .onEnd((event) => {
      if (
        event.translationY < -DISMISS_GESTURE_DISTANCE ||
        event.velocityY < -DISMISS_GESTURE_VELOCITY
      ) {
        runOnJS(close)();
      } else {
        slideY.value = withSpring(0, DISMISS_SPRING);
      }
    })
    .onFinalize((_event, success) => {
      if (!success) slideY.value = withSpring(0, DISMISS_SPRING);
    });

  useFocusEffect(
    useCallback(() => {
      let cancelled = false;
      const controller = new AbortController();

      if (!client) {
        setError("Unable to connect to this Agent conversation");
        setIsLoading(false);
        return () => controller.abort();
      }

      const acceptEvent = (event: DelegationEventResponse) => {
        if (
          cancelled ||
          event.parent_session_id !== sessionId ||
          event.delegation_group_id !== groupId ||
          event.delegation_task_id !== taskId
        )
          return;
        setEvents((current) => mergeEvents(current, [event]));
        const nextState = taskStateFromEvent(event);
        if (nextState) {
          setTask((current) =>
            current
              ? {
                  ...current,
                  state: nextState,
                  updated_at: event.created_at,
                  completed_at: ACTIVE_TASK_STATES.has(nextState)
                    ? current.completed_at
                    : event.created_at,
                }
              : current,
          );
        }
      };
      const unsubscribe = client.subscribeDelegationEvents(acceptEvent);

      const load = async () => {
        try {
          const snapshot = await client.getSessionState(sessionId, {
            signal: controller.signal,
          });
          if (cancelled) return;
          const nextGroup =
            snapshot.delegation_groups?.find(
              (candidate) => candidate.delegation_group_id === groupId,
            ) ?? null;
          const nextTask =
            nextGroup?.tasks.find(
              (candidate) => candidate.delegation_task_id === taskId,
            ) ?? null;
          const incoming = (snapshot.delegation_events ?? []).filter(
            (event) =>
              event.delegation_group_id === groupId &&
              event.delegation_task_id === taskId,
          );
          setTask(nextTask);
          setEvents((current) => mergeEvents(current, incoming));
          setError(null);
          setIsLoading(false);
        } catch (loadError) {
          if (cancelled || controller.signal.aborted) return;
          setError(
            loadError instanceof Error
              ? loadError.message
              : "Unable to load Agent conversation",
          );
          setIsLoading(false);
        }
      };

      void load();
      return () => {
        cancelled = true;
        controller.abort();
        unsubscribe();
      };
    }, [client, groupId, sessionId, taskId]),
  );

  const messages = useMemo(() => conversationMessages(events), [events]);
  const name = task?.task_key || fallbackName || "Hive Worker";
  const state = task?.state || (isLoading ? "loading" : "unknown");
  const active = ACTIVE_TASK_STATES.has(state);
  const activeToolCallId =
    messages
      .flatMap((message) => message.toolCalls ?? [])
      .findLast((call) => call.status === "running")?.id ?? null;

  return (
    <View style={styles.overlay}>
      <Animated.View
        style={[
          styles.root,
          {
            backgroundColor: t.background,
            height: sheetHeight,
            paddingTop: Math.max(insets.top, 8),
          },
          drawerAnimatedStyle,
        ]}
      >
        <View
          style={[
            styles.header,
            { top: Math.max(insets.top, 8), pointerEvents: "none" },
          ]}
        >
          <Text
            selectable
            numberOfLines={1}
            style={[styles.name, { color: t.foreground }]}
          >
            {name}
          </Text>
        </View>

        <View style={styles.transcript}>
          <ChatTranscript
            messages={messages}
            sessionId={sessionId}
            scrollStateKey={`agent:${groupId}:${taskId}`}
            isStreaming={active}
            isThinking={active && messages.length === 0}
            isLoading={isLoading}
            activeToolCallId={activeToolCallId}
            bottomPadding={Math.max(28, insets.bottom + 20)}
            topFadeHeight={96}
            topFadeOffset={0}
            topContentPadding={58}
            emptyState={
              <Text
                selectable
                style={[styles.empty, { color: t.mutedForeground }]}
              >
                {isLoading
                  ? "Opening child conversation…"
                  : error ||
                    "This run predates retained child conversations. New Agent runs will stream here."}
              </Text>
            }
          />
        </View>
        <GestureDetector gesture={dismissGesture}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Dismiss Agent conversation"
            accessibilityHint="Tap or swipe up"
            hitSlop={12}
            onPress={close}
            style={[styles.grabberRow, webGrabberStyle]}
          >
            <View
              style={[styles.grabber, { backgroundColor: t.mutedForeground }]}
            />
          </Pressable>
        </GestureDetector>
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  overlay: { flex: 1, backgroundColor: "rgba(0, 0, 0, 0.28)" },
  root: {
    position: "absolute",
    left: 0,
    right: 0,
    overflow: "hidden",
    borderBottomLeftRadius: 24,
    borderBottomRightRadius: 24,
    borderCurve: "continuous",
  },
  grabberRow: {
    height: 48,
    alignItems: "center",
    justifyContent: "center",
  },
  grabber: { width: 42, height: 4, borderRadius: 999, opacity: 0.42 },
  header: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    zIndex: 3,
    paddingHorizontal: 16,
  },
  name: {
    minWidth: 0,
    fontSize: 17,
    lineHeight: 21,
    fontWeight: "700",
  },
  transcript: { flex: 1 },
  empty: {
    paddingHorizontal: 28,
    textAlign: "center",
    fontSize: 13,
    lineHeight: 19,
  },
});
