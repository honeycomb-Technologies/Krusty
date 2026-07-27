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
import type { SessionType } from "@krusty/api";
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
  modes: Record<SessionType, ModeStores>;
  /** Compatibility aliases for shared/non-mode-aware surfaces. */
  session: SessionStore;
  workspace: WorkspaceStore;
  git: GitStore;
  plan: PlanStore;
}

interface ModeStores {
  session: SessionStore;
  workspace: WorkspaceStore;
  plan: PlanStore;
}

const StoresContext = createContext<StoresContextValue | null>(null);

interface StoresProviderProps {
  client: KrustyClient | null;
  children: ReactNode;
}

const STORAGE_HYDRATION_KEYS = [
  "krusty:workspace",
  "krusty:workspace:chat",
  "krusty:workspace:mako",
  "krusty-permission-mode",
  "krusty:presence-client-id",
] as const;

function buildStores(client: KrustyClient, storage: ReturnType<typeof createStorage>) {
  // Keep the legacy workspace key attached to Code so existing project and
  // branch selection survives the migration to independent mode slots.
  const workspaces: Record<SessionType, WorkspaceStore> = {
    chat: createWorkspaceStore(storage, "krusty:workspace:chat"),
    code: createWorkspaceStore(storage),
    mako: createWorkspaceStore(storage, "krusty:workspace:mako"),
  };
  const sessions = createSessionsStore(client, workspaces.code);
  const modes = (["chat", "code", "mako"] as const).reduce(
    (result, mode) => {
      const plan = createPlanStore();
      result[mode] = {
        workspace: workspaces[mode],
        plan,
        session: createSessionStore(
          client,
          storage,
          workspaces[mode],
          sessions,
          plan,
        ),
      };
      return result;
    },
    {} as Record<SessionType, ModeStores>,
  );
  const getDirectory = () => workspaces.code.getState().directory;
  const git = createGitStore(client, getDirectory);

  return {
    sessions,
    modes,
    // These aliases keep older shared components working. Mode-aware chat
    // surfaces use the hooks below with an explicit mode.
    session: modes.chat.session,
    workspace: modes.code.workspace,
    plan: modes.chat.plan,
    git,
  };
}

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
        try {
          await storage.hydrate([...STORAGE_HYDRATION_KEYS]);
        } catch {}
      }
      if (cancelled) {
        return;
      }

      setStores(buildStores(client, storage));
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
      if (!stores) return;
      for (const mode of ["chat", "code", "mako"] as const) {
        stores.modes[mode].session.getState().cleanup();
      }
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
  mode: SessionType = "chat",
): T {
  const store = useRequiredStoresContext().modes[mode].session;
  return useStore(store, selector);
}

export function useWorkspaceStore<T>(
  selector: (state: WorkspaceStoreState) => T,
  mode: SessionType = "code",
): T {
  const store = useRequiredStoresContext().modes[mode].workspace;
  return useStore(store, selector);
}

export function useGitStore<T>(selector: (state: GitStoreState) => T): T {
  const store = useRequiredStoresContext().git;
  return useStore(store, selector);
}

export function usePlanStore<T>(
  selector: (state: PlanStoreState) => T,
  mode: SessionType = "chat",
): T {
  const store = useRequiredStoresContext().modes[mode].plan;
  return useStore(store, selector);
}

export function useStores() {
  return useStoresContext();
}
