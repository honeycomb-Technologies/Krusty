import type { SessionStateResponse as ApiSessionStateResponse } from '@krusty/api';
import type { createPlanStore } from '../plan';
import { applyDelegatedSessionState } from './delegated';
import { applyLivePartialAssistant, applyRecoveryParity } from './transient';
import type { SessionMode, SessionStoreState } from './types';

type SessionStateSetter = (
  partial:
    | Partial<SessionStoreState>
    | ((state: SessionStoreState) => Partial<SessionStoreState>),
) => void;

export function applySessionSnapshot(
  sessionId: string,
  serverState: ApiSessionStateResponse | null,
  isRefresh: boolean,
  set: SessionStateSetter,
  get: () => SessionStoreState,
  planStore: ReturnType<typeof createPlanStore>,
) {
  if (!serverState) return;

  const nextMode: SessionMode = serverState.mode ?? 'build';
  set((state) => ({
    mode: nextMode,
    isStreaming:
      serverState.agent_state === 'streaming' ||
      serverState.agent_state === 'tool_executing',
    isThinking:
      serverState.agent_state === 'streaming'
        ? Boolean(serverState.live_partial_assistant?.thinking?.trim()) ||
          state.isThinking
        : false,
    thinkingContent: serverState.live_partial_assistant?.thinking || '',
    lastEventSequence: serverState.last_event_sequence ?? null,
    messages: applyDelegatedSessionState(
      applyLivePartialAssistant(
        applyRecoveryParity(
          state.messages,
          serverState.recovery,
          serverState.agent_state,
        ),
        serverState.live_partial_assistant,
        serverState.agent_state,
      ),
      serverState.delegated_tools,
      serverState.recent_delegated_runs,
    ),
  }));
  planStore.getState().setVisible(nextMode === 'plan');

  if (
    (serverState.agent_state === 'streaming' ||
      serverState.agent_state === 'tool_executing') &&
    !isRefresh
  ) {
    get().startStatePolling(sessionId);
  }
}
