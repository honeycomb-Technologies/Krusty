import { create } from 'zustand';
import type { KrustyClient, SessionResponse } from '@krusty/api';
import type { createWorkspaceStore } from './workspace';

export interface SessionListItem {
  id: string;
  title: string;
  updated_at: string;
  token_count?: number | null;
  parent_session_id?: string | null;
  working_dir?: string | null;
  project_dir?: string | null;
  workspace_mode?: 'neutral' | 'selected' | 'created';
  session_type?: 'chat' | 'code' | 'mako';
  target_branch?: string | null;
}

export interface SessionsStoreState {
  sessions: SessionListItem[];
  directories: string[];
  isLoading: boolean;
  error: string | null;
  loadSessions: () => Promise<void>;
  loadDirectories: () => Promise<void>;
  createSession: (title?: string, workingDir?: string, targetBranch?: string) => Promise<SessionListItem | null>;
  deleteSession: (id: string) => Promise<boolean>;
  selectSession: (id: string) => Promise<void>;
}

export function createSessionsStore(
  client: KrustyClient,
  workspace: ReturnType<typeof createWorkspaceStore>,
) {
  return create<SessionsStoreState>((set, get) => ({
    sessions: [],
    directories: [],
    isLoading: false,
    error: null,

    async loadSessions() {
      set((s) => ({ ...s, isLoading: true }));

      try {
        const data = await client.getSessions();
        set((s) => ({
          ...s,
          sessions: data as SessionListItem[],
          isLoading: false,
        }));
      } catch (err) {
        set((s) => ({
          ...s,
          isLoading: false,
          error: err instanceof Error ? err.message : 'Failed to load sessions',
        }));
      }
    },

    async loadDirectories() {
      try {
        const dirs = await client.getDirectories();
        set((s) => ({ ...s, directories: dirs }));
      } catch {
        // Silently fail
      }
    },

    async createSession(title?: string, workingDir?: string, targetBranch?: string) {
      set((s) => ({ ...s, isLoading: true }));

      try {
        const workspaceMode = workingDir ? 'selected' : 'neutral';
        const data: SessionResponse = await client.createSession(title, workingDir, targetBranch, workspaceMode);
        const state = get();

        set((s) => ({
          ...s,
          sessions: [data as SessionListItem, ...s.sessions],
          isLoading: false,
        }));

        if (workingDir && !state.directories.includes(workingDir)) {
          get().loadDirectories();
        }

        workspace.getState().setWorkspace(
          data.project_dir ?? workingDir ?? null,
          data.id,
          (data.workspace_mode ?? workspaceMode) as 'neutral' | 'selected' | 'created',
        );

        return data as SessionListItem;
      } catch (err) {
        set((s) => ({
          ...s,
          isLoading: false,
          error: err instanceof Error ? err.message : 'Failed to create session',
        }));
        return null;
      }
    },

    async deleteSession(id: string) {
      try {
        await client.deleteSession(id);
        set((s) => ({
          ...s,
          sessions: s.sessions.filter((session) => session.id !== id),
        }));

        const wsState = workspace.getState();
        if (wsState.sessionId === id) {
          wsState.clear();
        }
        return true;
      } catch (err) {
        set((s) => ({
          ...s,
          error: err instanceof Error ? err.message : 'Failed to delete session',
        }));
        return false;
      }
    },

    async selectSession(id: string) {
      const state = get();
      let session = state.sessions.find((s) => s.id === id);

      if (!session) {
        try {
          const data = await client.getSession(id);
          session = data.session as SessionListItem;
        } catch {
          // Session doesn't exist, just set ID without directory
        }
      }

      workspace.getState().setWorkspace(
        session?.project_dir ?? session?.working_dir ?? null,
        id,
        (session?.workspace_mode ?? ((session?.project_dir ?? session?.working_dir) ? 'selected' : 'neutral')) as 'neutral' | 'selected' | 'created',
      );
    },
  }));
}
