import type { SessionStateResponse as ApiSessionStateResponse } from '@krusty/api';
import type { createPlanStore } from '../plan';
import { applyDelegatedSessionState } from './delegated';
import { applyLivePartialAssistant, applyRecoveryParity } from './transient';
import type { PermissionMode, SessionMode, SessionStoreState } from './types';

type SessionStateSetter = (
  partial:
    | Partial<SessionStoreState>
    | ((state: SessionStoreState) => Partial<SessionStoreState>),
) => void;

export function isActiveSessionAgentState(agentState: string | null | undefined) {
  return agentState === 'streaming' || agentState === 'tool_executing';
}

export function isActionableSessionAgentState(agentState: string | null | undefined) {
  return agentState === 'awaiting_input';
}

export function isTerminalSessionAgentState(agentState: string | null | undefined) {
  return agentState === 'idle' || agentState === 'failed' || agentState === 'error';
}

export function sessionAgentErrorMessage(
  serverState: ApiSessionStateResponse | null | undefined,
): string | null {
  if (
    serverState?.agent_state !== 'error'
    && serverState?.agent_state !== 'failed'
  ) {
    return null;
  }

  return (
    serverState.recovery?.last_error?.trim()
    || 'The assistant could not complete this response.'
  );
}

export function shouldStopSessionStatePolling(
  agentState: string | null | undefined,
) {
  return isTerminalSessionAgentState(agentState) || isActionableSessionAgentState(agentState);
}

export function pendingInteractionsFromSnapshot(
  serverState: ApiSessionStateResponse | null | undefined,
) {
  return (
    serverState?.pending_interactions ??
    serverState?.recovery?.pending_interactions ??
    []
  );
}

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
  const nextPermissionMode: PermissionMode = serverState.permission_mode ?? get().permissionMode;
  const pendingInteractions = pendingInteractionsFromSnapshot(serverState);
  set((state) => ({
    mode: nextMode,
    permissionMode: nextPermissionMode,
    isStreaming: isActiveSessionAgentState(serverState.agent_state),
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
        pendingInteractions,
      ),
      serverState.delegated_tools,
      serverState.recent_delegated_runs,
    ),
  }));
  planStore.getState().setWorkflow(serverState.workflow ?? null);
  planStore
    .getState()
    .setVisible(Boolean(serverState.workflow) || nextMode === 'plan');

  if (isActiveSessionAgentState(serverState.agent_state) && !isRefresh) {
    get().startStatePolling(sessionId);
  }
}
