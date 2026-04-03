import { create } from 'zustand';
import type { KrustyStorage } from './storage';

const STORAGE_KEY = 'krusty:workspace';

export interface WorkspaceStoreState {
  directory: string | null;
  mode: 'neutral' | 'selected' | 'created';
  sessionId: string | null;
  initialized: boolean;
  setWorkspace: (directory: string | null, sessionId: string | null, mode?: 'neutral' | 'selected' | 'created') => void;
  setSession: (sessionId: string | null) => void;
  setDirectory: (directory: string | null) => void;
  clear: () => void;
  initFromSession: (sessionId: string, directory: string | null, mode?: 'neutral' | 'selected' | 'created') => void;
}

function loadState(storage: KrustyStorage): Omit<WorkspaceStoreState, 'setWorkspace' | 'setSession' | 'setDirectory' | 'clear' | 'initFromSession'> {
  try {
    const stored = storage.get(STORAGE_KEY);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<WorkspaceStoreState>;
      return {
        directory: parsed.directory ?? null,
        mode: parsed.mode ?? 'neutral',
        sessionId: parsed.sessionId ?? null,
        initialized: true,
      };
    }
  } catch {
    // Ignore parse errors
  }
  return { directory: null, mode: 'neutral', sessionId: null, initialized: true };
}

function saveState(storage: KrustyStorage, state: { directory: string | null; mode: string; sessionId: string | null }) {
  try {
    storage.set(STORAGE_KEY, JSON.stringify({
      directory: state.directory,
      mode: state.mode,
      sessionId: state.sessionId,
    }));
  } catch {
    // Ignore storage errors
  }
}

export function createWorkspaceStore(storage: KrustyStorage) {
  const initial = loadState(storage);

  return create<WorkspaceStoreState>((set, get) => ({
    ...initial,

    setWorkspace(
      directory: string | null,
      sessionId: string | null,
      mode: 'neutral' | 'selected' | 'created' = directory ? 'selected' : 'neutral',
    ) {
      const newState = { directory, mode, sessionId, initialized: true };
      set(newState);
      saveState(storage, newState);
    },

    setSession(sessionId: string | null) {
      const prev = get();
      const newState = { ...prev, sessionId };
      set({ sessionId });
      saveState(storage, newState);
    },

    setDirectory(directory: string | null) {
      const prev = get();
      const mode = directory
        ? (prev.mode === 'created' ? 'created' : 'selected')
        : 'neutral';
      const newState = { ...prev, directory, mode };
      set({ directory, mode });
      saveState(storage, newState);
    },

    clear() {
      const newState = { directory: null, mode: 'neutral' as const, sessionId: null, initialized: true };
      set(newState);
      saveState(storage, newState);
    },

    initFromSession(
      sessionId: string,
      directory: string | null,
      mode: 'neutral' | 'selected' | 'created' = directory ? 'selected' : 'neutral',
    ) {
      const newState = { directory, mode, sessionId, initialized: true };
      set(newState);
      saveState(storage, newState);
    },
  }));
}

export function validateWorkspace(
  store: ReturnType<typeof createWorkspaceStore>,
  apiClient: { getSession: (id: string) => Promise<unknown> },
) {
  const state = store.getState();
  if (!state.sessionId) return Promise.resolve();

  return apiClient.getSession(state.sessionId)
    .then(() => {
      // Session exists, workspace is valid
    })
    .catch(() => {
      // Session was deleted, clear workspace
      state.clear();
    });
}
