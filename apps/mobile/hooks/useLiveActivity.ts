import { useRef, useCallback, useEffect } from 'react';
import { Platform } from 'react-native';
import { addUserInteractionListener } from 'expo-widgets';
import type { LiveActivity } from 'expo-widgets';

// Import the Live Activity factory
import ChatStreamActivityFactory from '../widgets/ChatStreamActivity';

interface StreamState {
  chatTitle: string;
  model: string;
  status: 'streaming' | 'thinking' | 'tool_call' | 'awaiting_approval' | 'complete';
  currentText: string;
  currentTool: string;
  tokenCount: number;
  progress: number;
  toolApprovalId?: string;
  toolApprovalName?: string;
}

interface UseLiveActivityOptions {
  onToolApproval?: (id: string, approved: boolean) => void;
}

export function useLiveActivity(options?: UseLiveActivityOptions) {
  const activityRef = useRef<LiveActivity | null>(null);
  const startTimeRef = useRef<number>(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const stateRef = useRef<StreamState | null>(null);

  // Listen for button interactions from the Live Activity (approve/deny)
  useEffect(() => {
    if (Platform.OS !== 'ios') return;

    const sub = addUserInteractionListener((event) => {
      const target = event.target;
      if (target.startsWith('approve:')) {
        const id = target.slice('approve:'.length);
        options?.onToolApproval?.(id, true);
      } else if (target.startsWith('deny:')) {
        const id = target.slice('deny:'.length);
        options?.onToolApproval?.(id, false);
      }
    });

    return () => sub.remove();
  }, [options?.onToolApproval]);

  const startActivity = useCallback((chatTitle: string, model: string) => {
    if (Platform.OS !== 'ios') return;

    startTimeRef.current = Date.now();
    stateRef.current = {
      chatTitle,
      model,
      status: 'thinking',
      currentText: '',
      currentTool: '',
      tokenCount: 0,
      progress: 0,
    };

    try {
      activityRef.current = ChatStreamActivityFactory.start({
        ...stateRef.current,
        elapsedSeconds: 0,
      }, 'krusty://chat');
    } catch {
      // Live Activities may not be available (simulator, unsupported device)
      activityRef.current = null;
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
  }, []);

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
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }

    if (!activityRef.current || !stateRef.current) return;

    const elapsed = Math.floor((Date.now() - startTimeRef.current) / 1000);

    activityRef.current.end('default', {
      ...stateRef.current,
      status: 'complete',
      progress: 1,
      elapsedSeconds: elapsed,
    }).catch(() => {});

    activityRef.current = null;
    stateRef.current = null;
  }, []);

  // Cleanup on unmount
  useEffect(() => {
    return () => {
      if (timerRef.current) clearInterval(timerRef.current);
    };
  }, []);

  return { startActivity, updateActivity, endActivity };
}
