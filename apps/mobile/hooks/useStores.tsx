import { createContext, useContext, useMemo, useEffect, type ReactNode } from 'react';
import { createSessionsStore, createWorkspaceStore, createGitStore, createPlanStore } from '@krusty/state';
import type { SessionsStoreState, WorkspaceStoreState, PlanStoreState } from '@krusty/state';
import type { KrustyClient } from '@krusty/api';
import { createStorage } from '../platform/krusty-storage';
import { useStore } from 'zustand';

// Store types
type SessionsStore = ReturnType<typeof createSessionsStore>;
type WorkspaceStore = ReturnType<typeof createWorkspaceStore>;
type GitStore = ReturnType<typeof createGitStore>;
type PlanStore = ReturnType<typeof createPlanStore>;

interface StoresContextValue {
  sessions: SessionsStore;
  workspace: WorkspaceStore;
  git: GitStore;
  plan: PlanStore;
}

const StoresContext = createContext<StoresContextValue | null>(null);

interface StoresProviderProps {
  client: KrustyClient;
  children: ReactNode;
}

export function StoresProvider({ client, children }: StoresProviderProps) {
  const stores = useMemo(() => {
    const storage = createStorage();
    const workspace = createWorkspaceStore(storage);
    const sessions = createSessionsStore(client, workspace);
    const plan = createPlanStore();

    const getDirectory = () => workspace.getState().directory;
    const git = createGitStore(client, getDirectory);

    return { sessions, workspace, git, plan };
  }, [client]);

  // Load sessions on mount
  useEffect(() => {
    stores.sessions.getState().loadSessions();
  }, [stores]);

  return (
    <StoresContext.Provider value={stores}>
      {children}
    </StoresContext.Provider>
  );
}

function useStoresContext() {
  const ctx = useContext(StoresContext);
  if (!ctx) throw new Error('useStores must be used within StoresProvider');
  return ctx;
}

// Typed selector hooks
export function useSessionsStore<T>(selector: (state: SessionsStoreState) => T): T {
  const { sessions } = useStoresContext();
  return useStore(sessions, selector);
}

export function useWorkspaceStore<T>(selector: (state: WorkspaceStoreState) => T): T {
  const { workspace } = useStoresContext();
  return useStore(workspace, selector);
}

export function usePlanStore<T>(selector: (state: PlanStoreState) => T): T {
  const { plan } = useStoresContext();
  return useStore(plan, selector);
}

// Direct store access for imperative calls
export function useStores() {
  return useStoresContext();
}
