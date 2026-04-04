import {
  createContext,
  useContext,
  useMemo,
  useEffect,
  type ReactNode,
} from "react";
import {
  createSessionsStore,
  createSessionStore,
  createWorkspaceStore,
  createGitStore,
  createPlanStore,
} from "@krusty/state";
import type {
  SessionsStoreState,
  SessionStoreState,
  WorkspaceStoreState,
  GitStoreState,
  PlanStoreState,
} from "@krusty/state";
import type { KrustyClient } from "@krusty/api";
import { createStorage } from "../platform/krusty-storage";
import { useStore } from "zustand";

// Store types
type SessionsStore = ReturnType<typeof createSessionsStore>;
type SessionStore = ReturnType<typeof createSessionStore>;
type WorkspaceStore = ReturnType<typeof createWorkspaceStore>;
type GitStore = ReturnType<typeof createGitStore>;
type PlanStore = ReturnType<typeof createPlanStore>;

interface StoresContextValue {
  sessions: SessionsStore;
  session: SessionStore;
  workspace: WorkspaceStore;
  git: GitStore;
  plan: PlanStore;
}

const StoresContext = createContext<StoresContextValue | null>(null);

interface StoresProviderProps {
  client: KrustyClient | null;
  children: ReactNode;
}

export function StoresProvider({ client, children }: StoresProviderProps) {
  const stores = useMemo(() => {
    if (!client) return null;
    const storage = createStorage();
    const workspace = createWorkspaceStore(storage);
    const sessions = createSessionsStore(client, workspace);
    const plan = createPlanStore();
    const session = createSessionStore(
      client,
      storage,
      workspace,
      sessions,
      plan,
    );

    const getDirectory = () => workspace.getState().directory;
    const git = createGitStore(client, getDirectory);

    return { sessions, session, workspace, git, plan };
  }, [client]);

  useEffect(() => {
    stores?.sessions.getState().loadSessions();
  }, [stores]);

  useEffect(() => {
    return () => {
      stores?.session.getState().cleanup();
    };
  }, [stores]);

  return (
    <StoresContext.Provider value={stores}>{children}</StoresContext.Provider>
  );
}

function useStoresContext() {
  return useContext(StoresContext);
}

export function useSessionsStore<T>(
  selector: (state: SessionsStoreState) => T,
): T | undefined {
  const ctx = useStoresContext();
  const store = ctx?.sessions;
  return store ? useStore(store, selector) : undefined;
}

export function useSessionStore<T>(
  selector: (state: SessionStoreState) => T,
): T | undefined {
  const ctx = useStoresContext();
  const store = ctx?.session;
  return store ? useStore(store, selector) : undefined;
}

export function useWorkspaceStore<T>(
  selector: (state: WorkspaceStoreState) => T,
): T | undefined {
  const ctx = useStoresContext();
  const store = ctx?.workspace;
  return store ? useStore(store, selector) : undefined;
}

export function useGitStore<T>(selector: (state: GitStoreState) => T): T | undefined {
  const ctx = useStoresContext();
  const store = ctx?.git;
  return store ? useStore(store, selector) : undefined;
}

export function usePlanStore<T>(selector: (state: PlanStoreState) => T): T | undefined {
  const ctx = useStoresContext();
  const store = ctx?.plan;
  return store ? useStore(store, selector) : undefined;
}

export function useStores() {
  return useStoresContext();
}
