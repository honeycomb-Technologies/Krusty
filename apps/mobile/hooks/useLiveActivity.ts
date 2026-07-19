import { useRef, useCallback, useEffect } from 'react';
import { Platform } from 'react-native';

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

interface StreamState {
  chatTitle: string;
  status: 'working' | 'needs_input' | 'completed';
  toolCount: number;
  filesAdded: number;
  filesRemoved: number;
  toolApprovalId?: string;
  toolApprovalName?: string;
  toolApprovalSessionId?: string;
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
  const activityRef = useRef<LiveActivity | null>(null);
  const startTimeRef = useRef<number>(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const stateRef = useRef<StreamState | null>(null);
  const sessionIdRef = useRef<string | null>(null);

  const clearTimer = useCallback(() => {
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
  }, []);

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

    if (activityRef.current && sessionIdRef.current === sessionId) {
      return;
    }

    clearTimer();
    if (activityRef.current) {
      void activityRef.current.end('immediate').catch(() => {});
      activityRef.current = null;
    }
    closeExistingActivities();

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
      activityRef.current = ChatStreamActivityFactory.start({
        ...stateRef.current,
        elapsedSeconds: 0,
      }, `krusty://?sessionId=${encodeURIComponent(sessionId)}`);
    } catch {
      // Live Activities may not be available (simulator, unsupported device)
      activityRef.current = null;
      stateRef.current = null;
      sessionIdRef.current = null;
      return;
    }

    // Update elapsed time every second
    timerRef.current = setInterval(() => {
      if (!activityRef.current || !stateRef.current) return;
      const elapsed = Math.floor((Date.now() - startTimeRef.current) / 1000);
      activityRef.current.update({
        ...stateRef.current,
        elapsedSeconds: elapsed,
      }).catch(() => {});
    }, 1000);
  }, [clearTimer, closeExistingActivities]);

  const updateActivity = useCallback((partial: Partial<StreamState>) => {
    if (!activityRef.current || !stateRef.current) return;

    stateRef.current = { ...stateRef.current, ...partial };
    const elapsed = Math.floor((Date.now() - startTimeRef.current) / 1000);

    activityRef.current.update({
      ...stateRef.current,
      elapsedSeconds: elapsed,
    }).catch(() => {});
  }, []);

  const endActivity = useCallback(() => {
    clearTimer();

    if (!activityRef.current || !stateRef.current) return;

    const elapsed = Math.floor((Date.now() - startTimeRef.current) / 1000);

    activityRef.current.end({ after: new Date(Date.now() + 60_000) }, {
      ...stateRef.current,
      status: 'completed',
      elapsedSeconds: elapsed,
    }, new Date()).catch(() => {});

    activityRef.current = null;
    stateRef.current = null;
    sessionIdRef.current = null;
  }, [clearTimer]);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      clearTimer();
    };
  }, [clearTimer]);

  return { startActivity, updateActivity, endActivity };
}
