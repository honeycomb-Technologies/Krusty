import { create } from 'zustand';
import type { KrustyStorage } from './storage';

export const DEFAULT_WORKSPACE_STORAGE_KEY = 'krusty:workspace';

type WorkspaceMode = 'neutral' | 'selected' | 'created';

export interface WorkspaceStoreState {
  directory: string | null;
  targetBranch: string | null;
  mode: WorkspaceMode;
  sessionId: string | null;
  initialized: boolean;
  setWorkspace: (
    directory: string | null,
    sessionId: string | null,
    mode?: WorkspaceMode,
    targetBranch?: string | null,
  ) => void;
  setSession: (sessionId: string | null) => void;
  setDirectory: (directory: string | null, targetBranch?: string | null) => void;
  setTargetBranch: (targetBranch: string | null) => void;
  clear: () => void;
  initFromSession: (
    sessionId: string,
    directory: string | null,
    mode?: WorkspaceMode,
    targetBranch?: string | null,
  ) => void;
}

type PersistedWorkspaceState = Pick<
  WorkspaceStoreState,
  'directory' | 'targetBranch' | 'mode' | 'sessionId' | 'initialized'
>;

function normalizeTargetBranch(targetBranch: string | null | undefined): string | null {
  const trimmed = targetBranch?.trim();
  return trimmed ? trimmed : null;
}

function loadState(
  storage: KrustyStorage,
  storageKey: string,
): Omit<WorkspaceStoreState, 'setWorkspace' | 'setSession' | 'setDirectory' | 'setTargetBranch' | 'clear' | 'initFromSession'> {
  try {
    const stored = storage.get(storageKey);
    if (stored) {
      const parsed = JSON.parse(stored) as Partial<PersistedWorkspaceState>;
      return {
        directory: parsed.directory ?? null,
        targetBranch: normalizeTargetBranch(parsed.targetBranch),
        mode: parsed.mode ?? 'neutral',
        sessionId: parsed.sessionId ?? null,
        initialized: true,
      };
    }
  } catch {
    // Ignore parse errors
  }
  return { directory: null, targetBranch: null, mode: 'neutral', sessionId: null, initialized: true };
}

function saveState(
  storage: KrustyStorage,
  storageKey: string,
  state: {
    directory: string | null;
    targetBranch: string | null;
    mode: string;
    sessionId: string | null;
  },
) {
  try {
    storage.set(storageKey, JSON.stringify({
      directory: state.directory,
      targetBranch: state.targetBranch,
      mode: state.mode,
      sessionId: state.sessionId,
    }));
  } catch {
    // Ignore storage errors
  }
}

export function createWorkspaceStore(
  storage: KrustyStorage,
  storageKey = DEFAULT_WORKSPACE_STORAGE_KEY,
) {
  const initial = loadState(storage, storageKey);

  return create<WorkspaceStoreState>((set, get) => ({
    ...initial,

    setWorkspace(
      directory: string | null,
      sessionId: string | null,
      mode: WorkspaceMode = directory ? 'selected' : 'neutral',
      targetBranch: string | null = null,
    ) {
      const newState = {
        directory,
        targetBranch: normalizeTargetBranch(targetBranch),
        mode,
        sessionId,
        initialized: true,
      };
      set(newState);
      saveState(storage, storageKey, newState);
    },

    setSession(sessionId: string | null) {
      const prev = get();
      const newState = { ...prev, sessionId };
      set({ sessionId });
      saveState(storage, storageKey, newState);
    },

    setDirectory(directory: string | null, targetBranch: string | null = null) {
      const prev = get();
      const mode = directory
        ? (prev.mode === 'created' ? 'created' : 'selected')
        : 'neutral';
      const newState = {
        ...prev,
        directory,
        targetBranch: normalizeTargetBranch(targetBranch),
        mode,
      };
      set({ directory, targetBranch: newState.targetBranch, mode });
      saveState(storage, storageKey, newState);
    },

    setTargetBranch(targetBranch: string | null) {
      const prev = get();
      const newState = {
        ...prev,
        targetBranch: normalizeTargetBranch(targetBranch),
      };
      set({ targetBranch: newState.targetBranch });
      saveState(storage, storageKey, newState);
    },

    clear() {
      const newState = {
        directory: null,
        targetBranch: null,
        mode: 'neutral' as const,
        sessionId: null,
        initialized: true,
      };
      set(newState);
      saveState(storage, storageKey, newState);
    },

    initFromSession(
      sessionId: string,
      directory: string | null,
      mode: WorkspaceMode = directory ? 'selected' : 'neutral',
      targetBranch: string | null = null,
    ) {
      const newState = {
        directory,
        targetBranch: normalizeTargetBranch(targetBranch),
        mode,
        sessionId,
        initialized: true,
      };
      set(newState);
      saveState(storage, storageKey, newState);
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
