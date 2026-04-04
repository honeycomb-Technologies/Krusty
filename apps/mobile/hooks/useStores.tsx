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
  client: KrustyClient;
  children: ReactNode;
}

export function StoresProvider({ client, children }: StoresProviderProps) {
  const stores = useMemo(() => {
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
    stores.sessions.getState().loadSessions();
  }, [stores]);

  useEffect(() => {
    return () => {
      stores.session.getState().cleanup();
    };
  }, [stores]);

  return (
    <StoresContext.Provider value={stores}>{children}</StoresContext.Provider>
  );
}

function useStoresContext() {
  const ctx = useContext(StoresContext);
  if (!ctx) throw new Error("useStores must be used within StoresProvider");
  return ctx;
}

export function useSessionsStore<T>(
  selector: (state: SessionsStoreState) => T,
): T {
  const { sessions } = useStoresContext();
  return useStore(sessions, selector);
}

export function useSessionStore<T>(
  selector: (state: SessionStoreState) => T,
): T {
  const { session } = useStoresContext();
  return useStore(session, selector);
}

export function useWorkspaceStore<T>(
  selector: (state: WorkspaceStoreState) => T,
): T {
  const { workspace } = useStoresContext();
  return useStore(workspace, selector);
}

export function usePlanStore<T>(selector: (state: PlanStoreState) => T): T {
  const { plan } = useStoresContext();
  return useStore(plan, selector);
}

export function useStores() {
  return useStoresContext();
}
