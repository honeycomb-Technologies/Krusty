import { useRef, useCallback, useEffect } from 'react';
import { Platform } from 'react-native';
import {
  LIVE_ACTIVITY_TRANSITION_GRACE_MS,
  MIN_LIVE_ACTIVITY_UPDATE_INTERVAL_MS,
  liveActivityStateEqual,
  type LiveActivitySemanticState,
} from './presentationCadence';
import { useConnection } from './useConnection';

// Native-only imports — loaded dynamically to avoid crash on web
let addUserInteractionListener: any = () => ({ remove: () => {} });
let ChatStreamActivityFactory: any = null;

if (Platform.OS === 'ios') {
  try {
    addUserInteractionListener = require('expo-widgets').addUserInteractionListener;
    ChatStreamActivityFactory = require('../widgets/ChatStreamActivity').default;
  } catch {
    // Not available (web, simulator without widgets)
  }
}

type LiveActivity = any;
type LiveActivityInteractionEvent = {
  target?: unknown;
};

type StreamState = LiveActivitySemanticState;

interface ActivityContentState extends StreamState {
  startedAtMs: number;
  elapsedSeconds: number;
}

interface UseLiveActivityOptions {
  onToolApproval?: (
    sessionId: string,
    id: string,
    approved: boolean,
  ) => void;
}

function parseApprovalTarget(
  target: string,
  prefix: 'approve:' | 'deny:',
): { sessionId: string; toolCallId: string } | null {
  if (!target.startsWith(prefix)) {
    return null;
  }

  const payload = target.slice(prefix.length);
  const separatorIndex = payload.indexOf(':');
  if (separatorIndex === -1) {
    return null;
  }

  return {
    sessionId: decodeURIComponent(payload.slice(0, separatorIndex)),
    toolCallId: decodeURIComponent(payload.slice(separatorIndex + 1)),
  };
}

export function useLiveActivity(options?: UseLiveActivityOptions) {
  const { client } = useConnection();
  const clientRef = useRef(client);
  const activityRef = useRef<LiveActivity | null>(null);
  const startTimeRef = useRef<number>(0);
  const stateRef = useRef<StreamState | null>(null);
  const sessionIdRef = useRef<string | null>(null);
  const pendingEndTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingEndSessionIdRef = useRef<string | null>(null);
  const pendingUpdateRef = useRef<ActivityContentState | null>(null);
  const pendingUpdateUrgentRef = useRef(false);
  const updateInFlightRef = useRef(false);
  const updateGenerationRef = useRef(0);
  const lastUpdateStartedAtRef = useRef(0);
  const updateDelayRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pushTokenRef = useRef<string | null>(null);
  const pushTokenSubscriptionRef = useRef<{ remove?: () => void } | null>(null);

  useEffect(() => {
    clientRef.current = client;
  }, [client]);

  const removePushTokenSubscription = useCallback(() => {
    pushTokenSubscriptionRef.current?.remove?.();
    pushTokenSubscriptionRef.current = null;
  }, []);

  const registerActivityPushToken = useCallback(
    async (
      activity: LiveActivity,
      sessionId: string,
      pushToken: string,
      contentState: ActivityContentState,
    ) => {
      if (
        activityRef.current !== activity ||
        sessionIdRef.current !== sessionId ||
        !pushToken
      ) {
        return;
      }
      pushTokenRef.current = pushToken;
      await clientRef.current?.registerLiveActivity({
        sessionId,
        pushToken,
        contentState: contentState as unknown as Record<string, unknown>,
        startedAtMs: contentState.startedAtMs,
        environment: __DEV__ ? 'sandbox' : 'production',
      });
    },
    [],
  );

  useEffect(() => {
    const activity = activityRef.current;
    const sessionId = sessionIdRef.current;
    const pushToken = pushTokenRef.current;
    const state = stateRef.current;
    if (!client || !activity || !sessionId || !pushToken || !state) return;
    const contentState: ActivityContentState = {
      ...state,
      startedAtMs: startTimeRef.current,
      elapsedSeconds: Math.floor(
        (Date.now() - startTimeRef.current) / 1_000,
      ),
    };
    void registerActivityPushToken(
      activity,
      sessionId,
      pushToken,
      contentState,
    ).catch(() => {});
  }, [client, registerActivityPushToken]);

  const clearUpdateDelay = useCallback(() => {
    if (updateDelayRef.current) {
      clearTimeout(updateDelayRef.current);
      updateDelayRef.current = null;
    }
  }, []);

  const clearPendingEnd = useCallback(() => {
    if (pendingEndTimerRef.current) {
      clearTimeout(pendingEndTimerRef.current);
      pendingEndTimerRef.current = null;
    }
    pendingEndSessionIdRef.current = null;
  }, []);

  const runPendingUpdateRef = useRef<(generation: number) => void>(() => {});

  const schedulePendingUpdate = useCallback((force = false) => {
    if (!activityRef.current || !pendingUpdateRef.current) return;
    if (force) pendingUpdateUrgentRef.current = true;

    const generation = updateGenerationRef.current;
    const elapsed = Date.now() - lastUpdateStartedAtRef.current;
    const delay = pendingUpdateUrgentRef.current
      ? 0
      : Math.max(0, MIN_LIVE_ACTIVITY_UPDATE_INTERVAL_MS - elapsed);

    if (delay === 0 && !updateInFlightRef.current) {
      clearUpdateDelay();
      runPendingUpdateRef.current(generation);
      return;
    }

    if (!updateDelayRef.current && !updateInFlightRef.current) {
      updateDelayRef.current = setTimeout(() => {
        updateDelayRef.current = null;
        runPendingUpdateRef.current(generation);
      }, delay);
    }
  }, [clearUpdateDelay]);

  runPendingUpdateRef.current = (generation: number) => {
    if (
      generation !== updateGenerationRef.current ||
      updateInFlightRef.current ||
      !activityRef.current ||
      !pendingUpdateRef.current
    ) {
      return;
    }

    const activity = activityRef.current;
    const content = pendingUpdateRef.current;
    pendingUpdateRef.current = null;
    pendingUpdateUrgentRef.current = false;
    updateInFlightRef.current = true;
    lastUpdateStartedAtRef.current = Date.now();

    void activity.update(content).then(() => {
      const pushToken = pushTokenRef.current;
      const sessionId = sessionIdRef.current;
      if (pushToken && sessionId) {
        void clientRef.current?.updateLiveActivityState(
          sessionId,
          pushToken,
          content as unknown as Record<string, unknown>,
        );
      }
    }).catch(() => {}).finally(() => {
      updateInFlightRef.current = false;
      if (pendingUpdateRef.current && activityRef.current) {
        schedulePendingUpdate(pendingUpdateUrgentRef.current);
      }
    });
  };

  const closeExistingActivities = useCallback(() => {
    if (!ChatStreamActivityFactory) return;
    try {
      const instances = ChatStreamActivityFactory.getInstances?.() ?? [];
      for (const instance of instances) {
        void instance.end('immediate').catch(() => {});
      }
    } catch {
      // ActivityKit is unavailable on unsupported devices and simulators.
    }
  }, []);

  // Listen for button interactions from the Live Activity (approve/deny)
  useEffect(() => {
    if (Platform.OS !== 'ios') return;

    const sub = addUserInteractionListener((event: LiveActivityInteractionEvent) => {
      const target = event.target;
      if (typeof target !== 'string') {
        return;
      }

      const approvedAction = parseApprovalTarget(target, 'approve:');
      if (approvedAction) {
        options?.onToolApproval?.(
          approvedAction.sessionId,
          approvedAction.toolCallId,
          true,
        );
        return;
      }

      const deniedAction = parseApprovalTarget(target, 'deny:');
      if (deniedAction) {
        options?.onToolApproval?.(
          deniedAction.sessionId,
          deniedAction.toolCallId,
          false,
        );
      }
    });

    return () => sub.remove();
  }, [options?.onToolApproval]);

  const startActivity = useCallback((sessionId: string, chatTitle: string) => {
    if (Platform.OS !== 'ios' || !sessionId || !ChatStreamActivityFactory) return;

    // Same session still active: cancel any deferred end and keep the instance.
    if (activityRef.current && sessionIdRef.current === sessionId) {
      clearPendingEnd();
      if (stateRef.current && stateRef.current.chatTitle !== chatTitle) {
        stateRef.current = { ...stateRef.current, chatTitle };
        pendingUpdateRef.current = {
          ...stateRef.current,
          startedAtMs: startTimeRef.current,
          elapsedSeconds: Math.floor((Date.now() - startTimeRef.current) / 1000),
        };
        schedulePendingUpdate(true);
      }
      return;
    }

    // Re-enter during the transition grace window for the same session that was
    // about to end: cancel the deferred destroy instead of recreate thrash.
    if (
      !activityRef.current &&
      pendingEndTimerRef.current &&
      pendingEndSessionIdRef.current === sessionId &&
      stateRef.current
    ) {
      clearPendingEnd();
      // We no longer hold the previous activity handle once end was scheduled,
      // so create a fresh one only when the deferred end already released it.
    }

    clearPendingEnd();
    clearUpdateDelay();
    removePushTokenSubscription();
    updateGenerationRef.current += 1;
    pendingUpdateRef.current = null;
    pendingUpdateUrgentRef.current = false;

    // Session switch: delay previous destroy slightly so rapid flips batch into
    // one create, while still allowing the new session's activity to start now.
    if (activityRef.current) {
      const previousActivity = activityRef.current;
      const previousSessionId = sessionIdRef.current;
      const previousPushToken = pushTokenRef.current;
      const previousState = stateRef.current;
      activityRef.current = null;
      pendingEndSessionIdRef.current = previousSessionId;
      pendingEndTimerRef.current = setTimeout(() => {
        pendingEndTimerRef.current = null;
        pendingEndSessionIdRef.current = null;
        void previousActivity.end('immediate').catch(() => {});
        if (previousSessionId && previousPushToken) {
          void clientRef.current
            ?.unregisterLiveActivity(previousSessionId, previousPushToken)
            .catch(() => {});
        }
        // Drop only if no newer activity replaced this transition.
        if (sessionIdRef.current === previousSessionId && !activityRef.current) {
          stateRef.current = previousState;
        }
      }, LIVE_ACTIVITY_TRANSITION_GRACE_MS);
    } else {
      // No local activity handle, but OS may still have orphans.
      closeExistingActivities();
    }
    pushTokenRef.current = null;

    startTimeRef.current = Date.now();
    sessionIdRef.current = sessionId;
    stateRef.current = {
      chatTitle,
      status: 'working',
      toolCount: 0,
      filesAdded: 0,
      filesRemoved: 0,
    };

    try {
      const initialContent: ActivityContentState = {
        ...stateRef.current,
        startedAtMs: startTimeRef.current,
        elapsedSeconds: 0,
      };
      activityRef.current = ChatStreamActivityFactory.start(
        initialContent,
        `krusty://?sessionId=${encodeURIComponent(sessionId)}`,
      );
      const activity = activityRef.current;
      pushTokenSubscriptionRef.current = activity.addPushTokenListener?.(
        (event: { pushToken?: unknown }) => {
          if (typeof event.pushToken === 'string') {
            void registerActivityPushToken(
              activity,
              sessionId,
              event.pushToken,
              pendingUpdateRef.current ?? initialContent,
            ).catch(() => {});
          }
        },
      ) ?? null;
      void activity.getPushToken?.().then((pushToken: unknown) => {
        if (typeof pushToken === 'string') {
          return registerActivityPushToken(
            activity,
            sessionId,
            pushToken,
            pendingUpdateRef.current ?? initialContent,
          );
        }
      }).catch(() => {});
      lastUpdateStartedAtRef.current = Date.now();
    } catch {
      // Live Activities may not be available (simulator, unsupported device)
      activityRef.current = null;
      removePushTokenSubscription();
      pushTokenRef.current = null;
      stateRef.current = null;
      sessionIdRef.current = null;
      return;
    }
  }, [
    clearUpdateDelay,
    clearPendingEnd,
    closeExistingActivities,
    registerActivityPushToken,
    removePushTokenSubscription,
    schedulePendingUpdate,
  ]);

  const updateActivity = useCallback((partial: Partial<StreamState>) => {
    if (!activityRef.current || !stateRef.current) return;

    // A deferred end means the stream resumed for this session; keep it alive.
    clearPendingEnd();

    const previous = stateRef.current;
    const next = { ...previous, ...partial };
    if (liveActivityStateEqual(previous, next)) return;

    stateRef.current = next;
    const elapsed = Math.floor((Date.now() - startTimeRef.current) / 1000);
    pendingUpdateRef.current = {
      ...next,
      startedAtMs: startTimeRef.current,
      elapsedSeconds: elapsed,
    };

    const statusChanged = previous.status !== next.status;
    schedulePendingUpdate(statusChanged);
  }, [clearPendingEnd, schedulePendingUpdate]);

  const endActivity = useCallback((immediate = false) => {
    clearUpdateDelay();

    if (!activityRef.current || !stateRef.current) {
      // If only a deferred end is pending, either cancel-noop or force it.
      if (immediate) {
        clearPendingEnd();
      }
      return;
    }

    const activity = activityRef.current;
    const sessionId = sessionIdRef.current;
    const pushToken = pushTokenRef.current;
    const finalState = stateRef.current;
    const elapsed = Math.floor((Date.now() - startTimeRef.current) / 1000);

    const commitEnd = () => {
      updateGenerationRef.current += 1;
      pendingUpdateRef.current = null;
      pendingUpdateUrgentRef.current = false;
      activity.end({ after: new Date(Date.now() + 60_000) }, {
        ...finalState,
        status: 'completed',
        startedAtMs: startTimeRef.current,
        elapsedSeconds: elapsed,
      }, new Date()).catch(() => {});
      if (sessionId && pushToken) {
        void clientRef.current
          ?.unregisterLiveActivity(sessionId, pushToken)
          .catch(() => {});
      }

      removePushTokenSubscription();
      activityRef.current = null;
      pushTokenRef.current = null;
      stateRef.current = null;
      sessionIdRef.current = null;
      pendingEndSessionIdRef.current = null;
    };

    if (immediate) {
      clearPendingEnd();
      commitEnd();
      return;
    }

    // Batch short idle gaps / session-transition blips: defer the completed end
    // so a quick restart of the same session reuses the activity path.
    clearPendingEnd();
    pendingEndSessionIdRef.current = sessionId;
    pendingEndTimerRef.current = setTimeout(() => {
      pendingEndTimerRef.current = null;
      // Abort if a newer start/update reclaimed the activity.
      if (activityRef.current !== activity || sessionIdRef.current !== sessionId) {
        pendingEndSessionIdRef.current = null;
        return;
      }
      commitEnd();
    }, LIVE_ACTIVITY_TRANSITION_GRACE_MS);
  }, [clearPendingEnd, clearUpdateDelay, removePushTokenSubscription]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      clearPendingEnd();
      clearUpdateDelay();
      removePushTokenSubscription();
      updateGenerationRef.current += 1;
      pendingUpdateRef.current = null;
      pendingUpdateUrgentRef.current = false;
    };
  }, [clearPendingEnd, clearUpdateDelay, removePushTokenSubscription]);

  return { startActivity, updateActivity, endActivity };
}
