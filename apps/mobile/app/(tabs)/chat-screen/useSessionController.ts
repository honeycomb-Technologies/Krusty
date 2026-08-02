import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import { AppState } from "react-native";

import type { ModelInfo, ModelKey, SessionResponse, SessionType } from "@mitsuro/api";
import {
  modelKeysEqual,
  resolveUsableModel,
} from "@mitsuro/state";

import type { useConnection } from "../../../hooks/useConnection";
import type { useStores } from "../../../hooks/useStores";
import * as SecureStore from "../../../platform/secure-store";
import { normalizeProviderId } from "./helpers";
import {
  deleteMigratedAsyncValue,
  IDENTITY_STORAGE_KEYS,
  readMigratedAsyncValue,
  writeCanonicalAsyncValue,
} from "../../../platform/identity-storage";
import { resolveModeLifecyclePolicy } from "./modeLifecyclePolicy";

type LoadedStores = NonNullable<ReturnType<typeof useStores>>;
type ConnectionClient = ReturnType<typeof useConnection>["client"];
type SessionStoreApi = LoadedStores["session"];
type SessionsStoreApi = LoadedStores["sessions"];
type ModeStores = LoadedStores["modes"];

const VISIBLE_MODE_HYDRATION_DELAY_MS = 80;
const PRESENCE_SETTLE_DELAY_MS = 250;

function jsonEqual(left: unknown, right: unknown): boolean {
  if (left === right) return true;
  return JSON.stringify(left) === JSON.stringify(right);
}

function stringArraysEqual(left: string[], right: string[]): boolean {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

interface UseSessionControllerArgs {
  client: ConnectionClient;
  isConnected: boolean;
  activeMode: SessionType;
  sessionStore: SessionStoreApi;
  sessionsStore: SessionsStoreApi;
  modeStores: ModeStores;
  sessions: SessionResponse[];
  /**
   * Optional external ref so action handlers can share the same
   * last-session-by-type map. When omitted, the controller owns one.
   */
  lastSessionIdByTypeRef?: MutableRefObject<Record<SessionType, string | null>>;
}

/**
 * Non-visual session lifecycle for the chat shell:
 * connect warmup, active-mode hydrate, soft sessions refresh,
 * AppState resume reload, and model catalog refresh.
 *
 * Message ownership stays in ActiveConversationSurface.
 */
export function useSessionController({
  client,
  isConnected,
  activeMode,
  sessionStore,
  sessionsStore,
  modeStores,
  sessions,
  lastSessionIdByTypeRef: externalLastSessionIdByTypeRef,
}: UseSessionControllerArgs) {
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [defaultModelId, setDefaultModelId] = useState<string | null>(null);
  const [defaultModelKey, setDefaultModelKey] = useState<ModelKey | null>(null);
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([]);

  const sessionsRefreshInFlightRef = useRef(false);
  const persistedResolvedModelRef = useRef<string | null | undefined>(undefined);
  const persistedModelCandidateRef = useRef<string | null | undefined>(undefined);
  const attemptedWorkspaceSessionHydrationRef = useRef<
    Record<SessionType, string | null>
  >({
    chat: null,
    code: null,
    hive: null,
  });
  const ownedLastSessionIdByTypeRef = useRef<Record<SessionType, string | null>>({
    chat: null,
    code: null,
    hive: null,
  });
  const lastSessionIdByTypeRef =
    externalLastSessionIdByTypeRef ?? ownedLastSessionIdByTypeRef;

  const loadModelCatalog = useCallback(async () => {
    if (!client || !isConnected) {
      return null;
    }

    const [response, credentials] = await Promise.all([
      client.getModels(),
      client.getCredentials().catch(() => []),
    ]);
    const nextConfiguredProviders = credentials
      .filter((provider) => provider.configured || provider.has_oauth)
      .map((provider) => normalizeProviderId(provider.name));
    setModels((current) => jsonEqual(current, response.models) ? current : response.models);
    setDefaultModelId((current) => current === (response.default_model ?? null)
      ? current
      : response.default_model ?? null);
    setDefaultModelKey((current) => modelKeysEqual(current, response.default_model_key ?? null)
      ? current
      : response.default_model_key ?? null);
    setConfiguredProviders((current) => stringArraysEqual(current, nextConfiguredProviders)
      ? current
      : nextConfiguredProviders);
    return {
      response,
      configuredProviders: nextConfiguredProviders,
    };
  }, [client, isConnected]);

  const ensureModelReady = useCallback(async (
    targetStore: SessionStoreApi = sessionStore,
  ) => {
    if (persistedModelCandidateRef.current === undefined) {
      persistedModelCandidateRef.current = await readMigratedAsyncValue(
        SecureStore,
        IDENTITY_STORAGE_KEYS.selectedModel,
      ).catch(() => null);
    }
    const existingModel =
      targetStore.getState().model ?? persistedModelCandidateRef.current;
    let catalog = models;
    let fallbackDefault = defaultModelId;
    let fallbackDefaultKey = defaultModelKey;
    let allowedProviders = configuredProviders;

    if (catalog.length === 0) {
      const result = await loadModelCatalog().catch(() => null);
      if (!result) {
        return null;
      }
      catalog = result.response.models;
      fallbackDefault = result.response.default_model ?? null;
      fallbackDefaultKey = result.response.default_model_key ?? null;
      allowedProviders = result.configuredProviders;
    }

    const selectedModel = resolveUsableModel(
      existingModel,
      fallbackDefault,
      catalog,
      allowedProviders,
      targetStore.getState().modelKey,
      fallbackDefaultKey,
    );

    if (selectedModel) {
      const state = targetStore.getState();
      const nextProvider = selectedModel.provider ?? null;
      if (
        state.model !== selectedModel.id
        || !modelKeysEqual(state.modelKey, selectedModel.key ?? null)
        || state.modelProvider !== nextProvider
        || !jsonEqual(state.modelInfo, selectedModel)
      ) {
        state.setModel(selectedModel.id, nextProvider, selectedModel);
      }
      if (persistedResolvedModelRef.current !== selectedModel.id) {
        await writeCanonicalAsyncValue(
          SecureStore,
          IDENTITY_STORAGE_KEYS.selectedModel,
          selectedModel.id,
        );
        persistedResolvedModelRef.current = selectedModel.id;
        persistedModelCandidateRef.current = selectedModel.id;
      }
      return selectedModel.id;
    }

    if (targetStore.getState().model !== null) {
      targetStore.getState().setModel(null);
    }
    if (persistedResolvedModelRef.current !== null) {
      await deleteMigratedAsyncValue(
        SecureStore,
        IDENTITY_STORAGE_KEYS.selectedModel,
      ).catch(() => {});
      persistedResolvedModelRef.current = null;
      persistedModelCandidateRef.current = null;
    }
    return null;
  }, [
    configuredProviders,
    defaultModelId,
    defaultModelKey,
    loadModelCatalog,
    models,
    sessionStore,
  ]);

  // Connect warmup: list sessions once per connection. Model catalog refreshes
  // must not turn into unrelated session-list reloads.
  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    void sessionsStore.getState().loadSessions();
  }, [client, isConnected, sessionsStore]);

  // Resolve only the visible mode model path. Background modes remain lazy.
  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }
    // Warm only the active mode model path on connect. Background modes can
    // resolve models when first focused / first used.
    void ensureModelReady(modeStores[activeMode].session);
  }, [activeMode, client, ensureModelReady, isConnected, modeStores]);

  // Exactly one settled visible mode advertises presence. Hidden streams keep
  // their recovery poll alive; hidden idle modes release it immediately.
  // Presence transport itself is delayed until navigation becomes quiet so
  // rapid mode switches do not emit a PUT/DELETE pair for every intermediate
  // surface.
  useEffect(() => {
    for (const mode of ["chat", "code", "hive"] as const) {
      const state = modeStores[mode].session.getState();
      if (mode !== activeMode && !state.isStreaming) {
        if (state.isLoading && state.messages.length === 0) {
          attemptedWorkspaceSessionHydrationRef.current[mode] = null;
        }
        state.cancelPendingSessionLoad();
      }
      const policy = resolveModeLifecyclePolicy(
        activeMode,
        mode,
        state.isStreaming,
      );
      if (!policy.keepPolling) {
        state.stopStatePolling();
      }
    }

    // Session identity can bind after the mode change effect (optimistic load,
    // creation, notification navigation). Only the visible store is observed,
    // so a late hidden hydration cannot re-enable its presence transport.
    const activeStore = modeStores[activeMode].session;
    let disposed = false;
    let presenceTimer: ReturnType<typeof setTimeout> | null = null;
    const reconcilePresence = () => {
      presenceTimer = null;
      if (disposed) return;
      for (const mode of ["chat", "code", "hive"] as const) {
        const state = modeStores[mode].session.getState();
        const policy = resolveModeLifecyclePolicy(
          activeMode,
          mode,
          state.isStreaming,
        );
        if (policy.keepPresence && state.sessionId) {
          state.startPresenceHeartbeat(state.sessionId);
        } else {
          // This reconciler is the current navigation authority, so clear any
          // older transport even if the hidden store changed session IDs while
          // the user was switching modes.
          state.stopPresenceHeartbeat();
        }
      }
    };
    const schedulePresenceReconcile = () => {
      if (presenceTimer !== null) clearTimeout(presenceTimer);
      presenceTimer = setTimeout(
        reconcilePresence,
        PRESENCE_SETTLE_DELAY_MS,
      );
    };

    let activeSessionId = activeStore.getState().sessionId;
    const unsubscribe = activeStore.subscribe((state) => {
      if (state.sessionId === activeSessionId) return;
      activeSessionId = state.sessionId;
      schedulePresenceReconcile();
    });
    schedulePresenceReconcile();
    return () => {
      disposed = true;
      if (presenceTimer !== null) clearTimeout(presenceTimer);
      unsubscribe();
    };
  }, [activeMode, modeStores]);

  // Periodic model catalog refresh while the app is foregrounded.
  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    const refreshHandle = setInterval(() => {
      if (AppState.currentState === "active") {
        void loadModelCatalog().catch(() => null);
      }
    }, 5 * 60 * 1000);

    return () => clearInterval(refreshHandle);
  }, [client, isConnected, loadModelCatalog]);

  // Eager-hydrate only the visible mode from the sessions list.
  useEffect(() => {
    if (!client || !isConnected || sessions.length === 0) {
      return;
    }

    // Parallel chat/code/hive loads were a major source of resume thrash and
    // made mode switches feel crashy under load. Background modes warm on
    // first focus instead of all at once.
    const type = activeMode;
    const slot = modeStores[type];
    const sessionState = slot.session.getState();
    let targetId = sessionState.sessionId;
    if (targetId && (!sessionState.isLoading || sessionState.messages.length > 0)) {
      return;
    }
    const rememberedId = lastSessionIdByTypeRef.current[type];
    const remembered = rememberedId
      ? sessions.find(
          (candidate) =>
            candidate.id === rememberedId && candidate.session_type === type,
        )
      : null;
    const persistedId = slot.workspace.getState().sessionId;
    const persisted = persistedId
      ? sessions.find(
          (candidate) =>
            candidate.id === persistedId && candidate.session_type === type,
        )
      : null;
    const recent = sessions
      .filter((candidate) => candidate.session_type === type)
      .sort(
        (left, right) =>
          new Date(right.updated_at).getTime() -
          new Date(left.updated_at).getTime(),
      )[0];
    targetId = targetId ?? remembered?.id ?? persisted?.id ?? recent?.id ?? null;
    const scheduledTargetId = targetId;
    if (
      !scheduledTargetId ||
      attemptedWorkspaceSessionHydrationRef.current[type] === scheduledTargetId
    ) {
      return;
    }

    lastSessionIdByTypeRef.current[type] = scheduledTargetId;
    const hydrationTimer = setTimeout(() => {
      attemptedWorkspaceSessionHydrationRef.current[type] = scheduledTargetId;
      void slot.session.getState().loadSession(scheduledTargetId, true).catch(() => {
        attemptedWorkspaceSessionHydrationRef.current[type] = null;
        void sessionsStore.getState().loadSessions();
      });
    }, VISIBLE_MODE_HYDRATION_DELAY_MS);

    return () => clearTimeout(hydrationTimer);
  }, [
    activeMode,
    client,
    isConnected,
    lastSessionIdByTypeRef,
    modeStores,
    sessions,
    sessionsStore,
  ]);

  // Soft sessions refresh while foregrounded and not streaming.
  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    const refreshHandle = setInterval(() => {
      if (
        AppState.currentState !== "active" ||
        sessionStore.getState().isStreaming ||
        sessionsRefreshInFlightRef.current
      ) {
        return;
      }

      sessionsRefreshInFlightRef.current = true;
      void sessionsStore.getState().loadSessions().finally(() => {
        sessionsRefreshInFlightRef.current = false;
      });
    }, 30_000);

    return () => clearInterval(refreshHandle);
  }, [client, isConnected, sessionStore, sessionsStore]);

  // Resume: reload the active mode session + refresh models. Background modes
  // stay lazy to avoid a resume network storm.
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (nextState) => {
      if (nextState !== "active") {
        for (const mode of ["chat", "code", "hive"] as const) {
          const state = modeStores[mode].session.getState();
          state.stopPresenceHeartbeat(state.sessionId);
        }
        return;
      }

      const activeSlot = modeStores[activeMode];
      const currentSessionId =
        activeSlot.session.getState().sessionId ??
        activeSlot.workspace.getState().sessionId;
      if (currentSessionId) {
        activeSlot.session.getState().startPresenceHeartbeat(currentSessionId);
      }
      if (
        currentSessionId &&
        !activeSlot.session.getState().isStreaming
      ) {
        void activeSlot.session.getState().loadSession(currentSessionId, true);
      }
      void loadModelCatalog().catch(() => null);
    });

    return () => subscription.remove();
  }, [activeMode, loadModelCatalog, modeStores]);

  return {
    models,
    defaultModelId,
    defaultModelKey,
    configuredProviders,
    loadModelCatalog,
    ensureModelReady,
    lastSessionIdByTypeRef,
  };
}
