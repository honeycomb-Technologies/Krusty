import { useRef, useCallback, useEffect } from 'react';
import { Platform } from 'react-native';
import {
  LIVE_ACTIVITY_TRANSITION_GRACE_MS,
  MIN_LIVE_ACTIVITY_UPDATE_INTERVAL_MS,
  liveActivityStateEqual,
  type LiveActivitySemanticState,
} from './presentationCadence';
import { useConnection } from './useConnection';
import {
  beginKrustyPerformanceSpan,
  trackKrustyPerformanceResource,
} from '@krusty/state';
import { recordLiveActivityDiagnostic } from '../diagnostics/mobileDiagnostics';

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

const LIVE_ACTIVITY_RECREATE_COOLDOWN_MS = 750;

interface PendingActivityStart {
  sessionId: string;
  chatTitle: string;
  partial: Partial<StreamState>;
}

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
  const pendingStartTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingStartRef = useRef<PendingActivityStart | null>(null);
  const lastActivityStartAtRef = useRef(0);
  const didReconcileExistingActivitiesRef = useRef(false);
  const endInFlightRef = useRef<Promise<void> | null>(null);
  const endWaitScheduledRef = useRef(false);
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

  const clearPendingStart = useCallback(() => {
    if (pendingStartTimerRef.current) {
      clearTimeout(pendingStartTimerRef.current);
      pendingStartTimerRef.current = null;
    }
    pendingStartRef.current = null;
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
    const activitySessionId = sessionIdRef.current;
    const activityPushToken = pushTokenRef.current;
    const content = pendingUpdateRef.current;
    pendingUpdateRef.current = null;
    pendingUpdateUrgentRef.current = false;
    updateInFlightRef.current = true;
    lastUpdateStartedAtRef.current = Date.now();
    const finishUpdateSpan = beginKrustyPerformanceSpan(
      'live_activity.update',
      sessionIdRef.current ?? undefined,
    );
    const releaseUpdateResource = trackKrustyPerformanceResource(
      'live_activity_updates',
    );

    void activity.update(content).then(() => {
      recordLiveActivityDiagnostic('update', Date.now() - lastUpdateStartedAtRef.current);
      if (
        activityRef.current === activity &&
        sessionIdRef.current === activitySessionId &&
        pushTokenRef.current === activityPushToken &&
        activityPushToken &&
        activitySessionId
      ) {
        void clientRef.current?.updateLiveActivityState(
          activitySessionId,
          activityPushToken,
          content as unknown as Record<string, unknown>,
        );
      }
    }).catch(() => {
      recordLiveActivityDiagnostic('error', Date.now() - lastUpdateStartedAtRef.current);
    }).finally(() => {
      finishUpdateSpan();
      releaseUpdateResource();
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

  const releaseCurrentActivityImmediately = useCallback(() => {
    clearPendingEnd();
    clearUpdateDelay();
    const activity = activityRef.current;
    const sessionId = sessionIdRef.current;
    const pushToken = pushTokenRef.current;
    if (!activity) return;

    // Drop JS ownership before invoking native end so another transition cannot
    // capture or end this handle a second time.
    activityRef.current = null;
    sessionIdRef.current = null;
    stateRef.current = null;
    pushTokenRef.current = null;
    removePushTokenSubscription();
    updateGenerationRef.current += 1;
    pendingUpdateRef.current = null;
    pendingUpdateUrgentRef.current = false;

    let ending: Promise<void>;
    try {
      ending = Promise.resolve(activity.end('immediate')).catch(() => {});
    } catch {
      ending = Promise.resolve();
    }
    endInFlightRef.current = ending;
    void ending.finally(() => {
      if (endInFlightRef.current === ending) {
        endInFlightRef.current = null;
      }
    });
    if (sessionId && pushToken) {
      void clientRef.current
        ?.unregisterLiveActivity(sessionId, pushToken)
        .catch(() => {});
    }
  }, [clearPendingEnd, clearUpdateDelay, removePushTokenSubscription]);

  const startActivityRef = useRef<(sessionId: string, chatTitle: string) => void>(
    () => {},
  );

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

    const queued = pendingStartRef.current;
    const queuedPartial = queued?.sessionId === sessionId ? queued.partial : {};
    pendingStartRef.current = { sessionId, chatTitle, partial: queuedPartial };

    // A different session never shares ownership with the current handle. End
    // it now while the handle is still reachable, then coalesce rapid requests
    // into one latest-session start.
    if (activityRef.current) {
      releaseCurrentActivityImmediately();
    }

    if (endInFlightRef.current) {
      if (!endWaitScheduledRef.current) {
        endWaitScheduledRef.current = true;
        void endInFlightRef.current.finally(() => {
          endWaitScheduledRef.current = false;
          const pending = pendingStartRef.current;
          if (pending) {
            startActivityRef.current(pending.sessionId, pending.chatTitle);
          }
        });
      }
      return;
    }

    const remainingCooldown = Math.max(
      0,
      LIVE_ACTIVITY_RECREATE_COOLDOWN_MS -
        (Date.now() - lastActivityStartAtRef.current),
    );
    if (remainingCooldown > 0) {
      if (!pendingStartTimerRef.current) {
        pendingStartTimerRef.current = setTimeout(() => {
          pendingStartTimerRef.current = null;
          const pending = pendingStartRef.current;
          if (!pending) return;
          startActivityRef.current(pending.sessionId, pending.chatTitle);
        }, remainingCooldown);
      }
      return;
    }

    if (pendingStartTimerRef.current) {
      clearTimeout(pendingStartTimerRef.current);
      pendingStartTimerRef.current = null;
    }
    const initialPartial = pendingStartRef.current?.sessionId === sessionId
      ? pendingStartRef.current.partial
      : {};
    pendingStartRef.current = null;
    clearPendingEnd();
    clearUpdateDelay();
    updateGenerationRef.current += 1;
    pendingUpdateRef.current = null;
    pendingUpdateUrgentRef.current = false;

    if (!didReconcileExistingActivitiesRef.current) {
      didReconcileExistingActivitiesRef.current = true;
      closeExistingActivities();
    }
    pushTokenRef.current = null;

    startTimeRef.current = Date.now();
    sessionIdRef.current = sessionId;
    stateRef.current = {
      status: 'working',
      toolCount: 0,
      filesAdded: 0,
      filesRemoved: 0,
      ...initialPartial,
      chatTitle,
    };

    try {
      const initialContent: ActivityContentState = {
        ...stateRef.current,
        startedAtMs: startTimeRef.current,
        elapsedSeconds: 0,
      };
      activityRef.current = ChatStreamActivityFactory.start(
        initialContent,
        `mitsuro://?sessionId=${encodeURIComponent(sessionId)}`,
      );
      recordLiveActivityDiagnostic('start');
      lastActivityStartAtRef.current = Date.now();
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
      recordLiveActivityDiagnostic('error');
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
    releaseCurrentActivityImmediately,
    registerActivityPushToken,
    removePushTokenSubscription,
    schedulePendingUpdate,
  ]);
  startActivityRef.current = startActivity;

  const updateActivity = useCallback((partial: Partial<StreamState>) => {
    if (!activityRef.current || !stateRef.current) {
      if (pendingStartRef.current) {
        pendingStartRef.current = {
          ...pendingStartRef.current,
          partial: { ...pendingStartRef.current.partial, ...partial },
        };
      }
      return;
    }

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
    clearPendingStart();
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
      let ending: Promise<void>;
      try {
        ending = Promise.resolve(activity.end(
          { after: new Date(Date.now() + 60_000) },
          {
            ...finalState,
            status: 'completed',
            startedAtMs: startTimeRef.current,
            elapsedSeconds: elapsed,
          },
          new Date(),
        )).catch(() => {});
      } catch {
        ending = Promise.resolve();
      }
      endInFlightRef.current = ending;
      void ending.finally(() => {
        recordLiveActivityDiagnostic('end', Date.now() - startTimeRef.current);
        if (endInFlightRef.current === ending) {
          endInFlightRef.current = null;
        }
      });
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
  }, [clearPendingEnd, clearPendingStart, clearUpdateDelay, removePushTokenSubscription]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      clearPendingEnd();
      clearPendingStart();
      clearUpdateDelay();
      removePushTokenSubscription();
      updateGenerationRef.current += 1;
      pendingUpdateRef.current = null;
      pendingUpdateUrgentRef.current = false;
    };
  }, [clearPendingEnd, clearPendingStart, clearUpdateDelay, removePushTokenSubscription]);

  return { startActivity, updateActivity, endActivity };
}
