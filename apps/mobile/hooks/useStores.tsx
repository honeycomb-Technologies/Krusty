import {
  createContext,
  useContext,
  useEffect,
  useState,
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

const STORAGE_HYDRATION_KEYS = [
  "krusty:workspace",
  "krusty-permission-mode",
  "krusty:presence-client-id",
] as const;

export function StoresProvider({ client, children }: StoresProviderProps) {
  const [stores, setStores] = useState<StoresContextValue | null>(null);

  useEffect(() => {
    let cancelled = false;

    if (!client) {
      setStores(null);
      return () => {
        cancelled = true;
      };
    }

    setStores(null);

    void (async () => {
      const storage = createStorage();
      if (typeof storage.hydrate === "function") {
        await storage.hydrate([...STORAGE_HYDRATION_KEYS]);
      }
      if (cancelled) {
        return;
      }

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

      setStores({ sessions, session, workspace, git, plan });
    })();

    return () => {
      cancelled = true;
    };
  }, [client]);

  useEffect(() => {
    if (!stores) return;
    void stores.sessions.getState().loadSessions();
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

function useRequiredStoresContext(): StoresContextValue {
  const ctx = useStoresContext();
  if (!ctx) {
    throw new Error("Stores are not ready");
  }
  return ctx;
}

export function useSessionsStore<T>(
  selector: (state: SessionsStoreState) => T,
): T {
  const store = useRequiredStoresContext().sessions;
  return useStore(store, selector);
}

export function useSessionStore<T>(
  selector: (state: SessionStoreState) => T,
): T {
  const store = useRequiredStoresContext().session;
  return useStore(store, selector);
}

export function useWorkspaceStore<T>(
  selector: (state: WorkspaceStoreState) => T,
): T {
  const store = useRequiredStoresContext().workspace;
  return useStore(store, selector);
}

export function useGitStore<T>(selector: (state: GitStoreState) => T): T {
  const store = useRequiredStoresContext().git;
  return useStore(store, selector);
}

export function usePlanStore<T>(selector: (state: PlanStoreState) => T): T {
  const store = useRequiredStoresContext().plan;
  return useStore(store, selector);
}

export function useStores() {
  return useStoresContext();
}
