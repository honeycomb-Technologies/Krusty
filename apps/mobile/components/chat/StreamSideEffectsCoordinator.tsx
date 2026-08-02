import { memo, useEffect, useMemo, useRef, type MutableRefObject } from "react";
import type { SessionType } from "@mitsuro/api";
import { beginMitsuroPerformanceSpan } from "@mitsuro/state";
import { useShallow } from "zustand/react/shallow";

import * as Haptics from "../../platform/haptics";
import { useConnection } from "../../hooks/useConnection";
import { useLiveActivity } from "../../hooks/useLiveActivity";
import { useNotifications } from "../../hooks/useNotifications";
import { resolveLiveActivityTransition } from "../../hooks/presentationCadence";
import { useSessionStore } from "../../hooks/useStores";
import { useWidgetSync } from "../../hooks/useWidgetSync";
import { getToolDiffStats } from "./toolDiffModel";
import { createStreamEffectSelector } from "./streamEffectsModel";

interface StreamSideEffectsCoordinatorProps {
  activeMode: SessionType;
  suppressCompletionRef: MutableRefObject<boolean>;
}

function StreamSideEffectsCoordinatorComponent({
  activeMode,
  suppressCompletionRef,
}: StreamSideEffectsCoordinatorProps) {
  const { isConnected } = useConnection();
  const selector = useMemo(createStreamEffectSelector, [activeMode]);
  const view = useSessionStore(useShallow(selector), activeMode);
  const previousStreamingRef = useRef(false);
  const currentStreamSessionIdRef = useRef<string | null>(null);
  const streamStartedAtRef = useRef<number | null>(null);
  const finishStreamSpanRef = useRef<(() => number | null) | null>(null);
  const liveActivitySessionIdRef = useRef<string | null>(null);
  const notifiedApprovalIdsRef = useRef<Set<string>>(new Set());

  const {
    notificationLevel,
    notifyToolApproval,
    notifyStreamComplete,
    submitToolApprovalAction,
  } = useNotifications();
  const { startActivity, updateActivity, endActivity } = useLiveActivity({
    onToolApproval: (sessionId, toolCallId, approved) => {
      void submitToolApprovalAction(sessionId, toolCallId, approved);
    },
  });

  const toolActivity = useMemo(() => {
    const awaitingApprovalCalls = view.currentTurnToolCalls.filter(
      (toolCall) => toolCall.status === "awaiting_approval",
    );
    const activeToolCall = [...view.currentTurnToolCalls]
      .reverse()
      .find((toolCall) =>
        toolCall.status === "running" ||
        toolCall.status === "pending" ||
        toolCall.status === "awaiting_approval",
      ) ?? null;
    const activityDiff = view.currentTurnToolCalls.reduce(
      (total, toolCall) => {
        const stats = getToolDiffStats(toolCall);
        if (stats) {
          total.additions += stats.additions;
          total.deletions += stats.deletions;
        }
        return total;
      },
      { additions: 0, deletions: 0 },
    );
    return { awaitingApprovalCalls, activeToolCall, activityDiff };
  }, [view.currentTurnToolCalls]);

  useWidgetSync({
    sessionId: view.sessionId,
    hasActiveSession: Boolean(view.sessionId),
    sessionTitle: view.title || "Untitled",
    lastMessage: view.settledAssistantSnippet,
    model: view.model || "",
    isStreaming: view.isStreaming,
    tokenCount: view.tokenCount,
    serverConnected: isConnected,
  });

  useEffect(() => {
    const nextNotifiedIds = new Set<string>();
    if (!view.sessionId) {
      notifiedApprovalIdsRef.current = nextNotifiedIds;
      return;
    }

    for (const toolCall of toolActivity.awaitingApprovalCalls) {
      nextNotifiedIds.add(toolCall.id);
      if (notifiedApprovalIdsRef.current.has(toolCall.id)) continue;
      void notifyToolApproval(toolCall.id, toolCall.name, view.sessionId);
      if (notificationLevel !== "silent") {
        void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Warning);
      }
    }
    notifiedApprovalIdsRef.current = nextNotifiedIds;
  }, [
    notificationLevel,
    notifyToolApproval,
    toolActivity.awaitingApprovalCalls,
    view.sessionId,
  ]);

  useEffect(() => {
    const awaitingApproval =
      toolActivity.awaitingApprovalCalls[
        toolActivity.awaitingApprovalCalls.length - 1
      ] ?? null;
    const shouldKeepActivity =
      Boolean(view.sessionId) && (view.isStreaming || Boolean(awaitingApproval));

    if (view.isStreaming && !previousStreamingRef.current) {
      suppressCompletionRef.current = false;
      currentStreamSessionIdRef.current = view.sessionId;
      streamStartedAtRef.current = Date.now();
      finishStreamSpanRef.current?.();
      finishStreamSpanRef.current = beginMitsuroPerformanceSpan(
        "stream.finish",
        view.sessionId ?? undefined,
      );
    }

    const transition = resolveLiveActivityTransition({
      trackedSessionId: liveActivitySessionIdRef.current,
      focusedSessionId: view.sessionId,
      shouldKeepFocused: shouldKeepActivity,
    });

    if (transition.action === "start") {
      startActivity(transition.sessionId, view.title || "Chat");
      liveActivitySessionIdRef.current = transition.sessionId;
    } else if (transition.action === "end") {
      endActivity();
      liveActivitySessionIdRef.current = null;
    }

    if (transition.action === "start" || transition.action === "update") {
      updateActivity({
        chatTitle: view.title || "Chat",
        status: awaitingApproval ? "needs_input" : "working",
        toolCount:
          toolActivity.awaitingApprovalCalls.length +
          (toolActivity.activeToolCall ? 1 : 0),
        filesAdded: toolActivity.activityDiff.additions,
        filesRemoved: toolActivity.activityDiff.deletions,
        toolApprovalId: awaitingApproval?.id,
        toolApprovalName: awaitingApproval?.name,
        toolApprovalSessionId: awaitingApproval
          ? view.sessionId ?? undefined
          : undefined,
      });
    }

    if (
      previousStreamingRef.current &&
      !view.isStreaming &&
      !awaitingApproval &&
      !suppressCompletionRef.current &&
      currentStreamSessionIdRef.current &&
      currentStreamSessionIdRef.current === view.sessionId
    ) {
      const startedAt = streamStartedAtRef.current ?? Date.now();
      void notifyStreamComplete(
        currentStreamSessionIdRef.current,
        view.title || "Chat",
        view.tokenCount,
        Math.max(0, Math.floor((Date.now() - startedAt) / 1000)),
      );
    }

    if (previousStreamingRef.current && !view.isStreaming) {
      finishStreamSpanRef.current?.();
      finishStreamSpanRef.current = null;
    }

    if (!shouldKeepActivity && !view.isStreaming) {
      suppressCompletionRef.current = false;
      currentStreamSessionIdRef.current = null;
      streamStartedAtRef.current = null;
    }
    previousStreamingRef.current = view.isStreaming;
  }, [
    endActivity,
    notifyStreamComplete,
    startActivity,
    suppressCompletionRef,
    toolActivity,
    updateActivity,
    view.isStreaming,
    view.sessionId,
    view.title,
    view.tokenCount,
  ]);

  useEffect(
    () => () => {
      finishStreamSpanRef.current?.();
      finishStreamSpanRef.current = null;
    },
    [],
  );

  return null;
}

export const StreamSideEffectsCoordinator = memo(
  StreamSideEffectsCoordinatorComponent,
);
