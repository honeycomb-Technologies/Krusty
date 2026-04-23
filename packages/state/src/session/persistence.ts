import type { KrustyClient } from '@krusty/api';
import type { createSessionsStore } from '../sessions';
import type { SessionMode, SessionStoreState } from './types';

export async function syncSessionPresence(
  client: KrustyClient,
  sessionId: string,
  clientId: string | null,
  getState: () => SessionStoreState,
) {
  if (!clientId) return;

  const state = getState();
  try {
    await client.heartbeatSessionPresence(sessionId, {
      client_id: clientId,
      surface: 'mobile',
      capability: 'controller',
      last_event_sequence: state.lastEventSequence,
    });
  } catch {
    // Presence heartbeat failed silently
  }
}

export async function persistSessionMode(
  client: KrustyClient,
  sessionsStore: ReturnType<typeof createSessionsStore>,
  getState: () => SessionStoreState,
  mode: SessionMode,
) {
  const state = getState();
  if (!state.sessionId) return;

  try {
    await client.updateSession(state.sessionId, { mode });
    sessionsStore.getState().loadSessions();
  } catch {
    // Failed to persist
  }
}

export async function persistSessionModel(
  client: KrustyClient,
  sessionsStore: ReturnType<typeof createSessionsStore>,
  getState: () => SessionStoreState,
  model: string | null,
) {
  const state = getState();
  if (!state.sessionId) return;

  try {
    await client.updateSession(state.sessionId, { model });
    sessionsStore.getState().loadSessions();
  } catch {
    // Failed to persist
  }
}

export async function persistCurrentModel(
  client: KrustyClient,
  model: string | null,
) {
  try {
    await client.setCurrentModel(model);
  } catch {
    // Failed to persist
  }
}
