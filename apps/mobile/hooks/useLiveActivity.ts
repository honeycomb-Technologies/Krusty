import { useRef, useCallback, useEffect } from 'react';
import { Platform } from 'react-native';
import {
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

const MIN_ACTIVITY_UPDATE_INTERVAL_MS = 2_000;

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

  const runPendingUpdateRef = useRef<(generation: number) => void>(() => {});

  const schedulePendingUpdate = useCallback((force = false) => {
    if (!activityRef.current || !pendingUpdateRef.current) return;
    if (force) pendingUpdateUrgentRef.current = true;

    const generation = updateGenerationRef.current;
    const elapsed = Date.now() - lastUpdateStartedAtRef.current;
    const delay = pendingUpdateUrgentRef.current
      ? 0
      : Math.max(0, MIN_ACTIVITY_UPDATE_INTERVAL_MS - elapsed);

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

  const clearPendingEnd = useCallback(() => {
    if (pendingEndTimerRef.current) {
      clearTimeout(pendingEndTimerRef.current);
      pendingEndTimerRef.current = null;
    }
  }, []);

  const startActivity = useCallback((sessionId: string, chatTitle: string) => {
    if (Platform.OS !== 'ios' || !sessionId || !ChatStreamActivityFactory) return;

    // Same session: keep existing activity.
    if (activityRef.current && sessionIdRef.current === sessionId) {
      clearPendingEnd();
      return;
    }

    clearPendingEnd();
    clearUpdateDelay();
    removePushTokenSubscription();
    updateGenerationRef.current += 1;
    pendingUpdateRef.current = null;
    pendingUpdateUrgentRef.current = false;

    // Session switch: end previous activity, but prefer a short delayed end over
    // immediate destroy/recreate thrash when the user is flipping sessions.
    if (activityRef.current) {
      const previousActivity = activityRef.current;
      const previousSessionId = sessionIdRef.current;
      const previousPushToken = pushTokenRef.current;
      activityRef.current = null;
      pendingEndTimerRef.current = setTimeout(() => {
        pendingEndTimerRef.current = null;
        void previousActivity.end('immediate').catch(() => {});
        if (previousSessionId && previousPushToken) {
          void clientRef.current
            ?.unregisterLiveActivity(previousSessionId, previousPushToken)
            .catch(() => {});
        }
      }, 250);
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
  ]);

  const updateActivity = useCallback((partial: Partial<StreamState>) => {
    if (!activityRef.current || !stateRef.current) return;

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
  }, [schedulePendingUpdate]);

  const endActivity = useCallback(() => {
    clearPendingEnd();
    clearUpdateDelay();

    if (!activityRef.current || !stateRef.current) return;

    const activity = activityRef.current;
    const sessionId = sessionIdRef.current;
    const pushToken = pushTokenRef.current;
    const finalState = stateRef.current;
    const elapsed = Math.floor((Date.now() - startTimeRef.current) / 1000);
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
  }, [clearUpdateDelay, removePushTokenSubscription]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      clearUpdateDelay();
      removePushTokenSubscription();
      updateGenerationRef.current += 1;
      pendingUpdateRef.current = null;
      pendingUpdateUrgentRef.current = false;
    };
  }, [clearUpdateDelay, removePushTokenSubscription]);

  return { startActivity, updateActivity, endActivity };
}
