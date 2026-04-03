import { create } from 'zustand';
import type { KrustyClient, GitBranch, GitStatusResponse, GitWorktree } from '@krusty/api';

const POLL_INTERVAL_MS = 5000;

export interface GitStoreState {
  status: GitStatusResponse | null;
  branches: GitBranch[];
  worktrees: GitWorktree[];
  isLoading: boolean;
  error: string | null;
  refresh: (forceLoading?: boolean) => Promise<void>;
  checkoutBranch: (branch: string) => Promise<void>;
  switchWorktree: (path: string, sessionId?: string | null) => Promise<void>;
  startPolling: () => void;
  stopPolling: () => void;
  cleanup: () => void;
}

export function createGitStore(
  client: KrustyClient,
  getDirectory: () => string | null,
  onDirectoryChange?: (path: string) => void,
  onSessionUpdate?: (sessionId: string, data: { project_dir: string; working_dir: string; workspace_mode: string }) => Promise<void>,
  onSessionsReload?: () => void,
) {
  let pollTimer: ReturnType<typeof setInterval> | null = null;

  const store = create<GitStoreState>((set, get) => ({
    status: null,
    branches: [],
    worktrees: [],
    isLoading: false,
    error: null,

    async refresh(forceLoading = false) {
      const directory = getDirectory();
      if (!directory) {
        set({ status: null, branches: [], worktrees: [], isLoading: false, error: null });
        return;
      }

      set((s) => ({
        isLoading: forceLoading || s.status === null,
        error: null,
      }));

      try {
        const status = await client.getGitStatus(directory);

        if (!status.in_repo) {
          set({ status, branches: [], worktrees: [], isLoading: false, error: null });
          return;
        }

        const [branchesRes, worktreesRes] = await Promise.all([
          client.getGitBranches(directory),
          client.getGitWorktrees(directory),
        ]);

        set({
          status,
          branches: branchesRes.branches,
          worktrees: worktreesRes.worktrees,
          isLoading: false,
          error: null,
        });
      } catch (err) {
        set((s) => ({
          ...s,
          isLoading: false,
          error: err instanceof Error ? err.message : 'Failed to load git status',
        }));
      }
    },

    async checkoutBranch(branch: string) {
      const directory = getDirectory();
      if (!directory) return;

      await client.checkoutGitBranch(branch, directory);
      await get().refresh(false);
    },

    async switchWorktree(path: string, sessionId?: string | null) {
      onDirectoryChange?.(path);

      if (sessionId && onSessionUpdate) {
        await onSessionUpdate(sessionId, {
          project_dir: path,
          working_dir: path,
          workspace_mode: 'selected',
        });
        onSessionsReload?.();
      }

      await get().refresh(true);
    },

    startPolling() {
      if (pollTimer) return;
      pollTimer = setInterval(() => {
        const state = get();
        if (state.isLoading) return;
        void state.refresh(false);
      }, POLL_INTERVAL_MS);
    },

    stopPolling() {
      if (!pollTimer) return;
      clearInterval(pollTimer);
      pollTimer = null;
    },

    cleanup() {
      get().stopPolling();
    },
  }));

  return store;
}
