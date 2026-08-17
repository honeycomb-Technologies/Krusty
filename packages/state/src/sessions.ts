import { create } from 'zustand';
import type { MitsuroClient, SessionResponse } from '@mitsuro/api';
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
  session_type?: 'chat' | 'code' | 'hive';
  target_branch?: string | null;
  permission_mode?: 'supervised' | 'autonomous';
  pinned_at?: string | null;
  archived_at?: string | null;
}

export interface SessionsStoreState {
  sessions: SessionListItem[];
  directories: string[];
  isLoading: boolean;
  error: string | null;
  loadSessions: () => Promise<void>;
  loadDirectories: () => Promise<void>;
  /** Local list patch used by optimistic create/bootstrap paths. */
  upsertSession: (session: SessionListItem) => void;
  createSession: (title?: string, workingDir?: string, targetBranch?: string | null) => Promise<SessionListItem | null>;
  deleteSession: (id: string) => Promise<boolean>;
  selectSession: (id: string) => Promise<void>;
}

function sessionsListSignature(sessions: SessionListItem[]): string {
  // Cheap structural fingerprint so soft polls do not replace identical arrays
  // and re-render every closed drawer / shell consumer.
  return sessions
    .map((session) =>
      [
        session.id,
        session.updated_at,
        session.title ?? '',
        session.token_count ?? '',
        session.session_type ?? '',
        session.project_dir ?? session.working_dir ?? '',
        session.pinned_at ?? '',
        session.archived_at ?? '',
      ].join('\u001f'),
    )
    .join('\u001e');
}

export function createSessionsStore(
  client: MitsuroClient,
  workspace: ReturnType<typeof createWorkspaceStore>,
) {
  let loadSessionsInFlight: Promise<void> | null = null;

  return create<SessionsStoreState>((set, get) => ({
    sessions: [],
    directories: [],
    isLoading: false,
    error: null,

    async loadSessions() {
      if (loadSessionsInFlight) {
        return loadSessionsInFlight;
      }

      // Only show list loading chrome on the first fill. Soft refreshes should
      // not flash/disable the drawer while chat is active.
      set((s) => ({
        ...s,
        isLoading: s.sessions.length === 0,
        error: null,
      }));

      loadSessionsInFlight = (async () => {
        try {
          const data = (await client.getSessions()) as SessionListItem[];
          set((s) => {
            const nextSignature = sessionsListSignature(data);
            const prevSignature = sessionsListSignature(s.sessions);
            if (nextSignature === prevSignature) {
              return {
                ...s,
                isLoading: false,
                error: null,
              };
            }
            return {
              ...s,
              sessions: data,
              isLoading: false,
              error: null,
            };
          });
        } catch (err) {
          set((s) => ({
            ...s,
            isLoading: false,
            error: err instanceof Error ? err.message : 'Failed to load sessions',
          }));
        } finally {
          loadSessionsInFlight = null;
        }
      })();

      return loadSessionsInFlight;
    },

    async loadDirectories() {
      try {
        const dirs = await client.getDirectories();
        set((s) => ({ ...s, directories: dirs }));
      } catch {
        // Silently fail
      }
    },

    upsertSession(session: SessionListItem) {
      set((s) => {
        const existingIndex = s.sessions.findIndex((item) => item.id === session.id);
        if (existingIndex === -1) {
          return {
            ...s,
            sessions: [session, ...s.sessions],
          };
        }
        const next = s.sessions.slice();
        next[existingIndex] = { ...next[existingIndex], ...session };
        return { ...s, sessions: next };
      });
    },

    async createSession(title?: string, workingDir?: string, targetBranch?: string | null) {
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
          data.target_branch ?? targetBranch ?? null,
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
        session?.target_branch ?? null,
      );
    },
  }));
}
