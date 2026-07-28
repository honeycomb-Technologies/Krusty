import { useCallback, useEffect, useRef, useState, type MutableRefObject } from "react";
import { AppState } from "react-native";

import type { ModelInfo, ModelKey, SessionResponse, SessionType } from "@krusty/api";
import {
  resolveUsableModel,
} from "@krusty/state";

import type { useConnection } from "../../../hooks/useConnection";
import type { useStores } from "../../../hooks/useStores";
import * as SecureStore from "../../../platform/secure-store";
import { normalizeProviderId, SELECTED_MODEL_KEY } from "./helpers";

type LoadedStores = NonNullable<ReturnType<typeof useStores>>;
type ConnectionClient = ReturnType<typeof useConnection>["client"];
type SessionStoreApi = LoadedStores["session"];
type SessionsStoreApi = LoadedStores["sessions"];
type ModeStores = LoadedStores["modes"];

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
  const attemptedWorkspaceSessionHydrationRef = useRef<
    Record<SessionType, string | null>
  >({
    chat: null,
    code: null,
    mako: null,
  });
  const ownedLastSessionIdByTypeRef = useRef<Record<SessionType, string | null>>({
    chat: null,
    code: null,
    mako: null,
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
    setModels(response.models);
    setDefaultModelId(response.default_model ?? null);
    setDefaultModelKey(response.default_model_key ?? null);
    setConfiguredProviders(nextConfiguredProviders);
    return {
      response,
      configuredProviders: nextConfiguredProviders,
    };
  }, [client, isConnected]);

  const ensureModelReady = useCallback(async (
    targetStore: SessionStoreApi = sessionStore,
  ) => {
    const existingModel = targetStore.getState().model;
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
      targetStore
        .getState()
        .setModel(selectedModel.id, selectedModel.provider ?? null, selectedModel);
      await SecureStore.setItemAsync(SELECTED_MODEL_KEY, selectedModel.id);
      return selectedModel.id;
    }

    targetStore.getState().setModel(null);
    await SecureStore.deleteItemAsync(SELECTED_MODEL_KEY).catch(() => {});
    return null;
  }, [
    configuredProviders,
    defaultModelId,
    defaultModelKey,
    loadModelCatalog,
    models,
    sessionStore,
  ]);

  // Connect warmup: list sessions + resolve the visible mode model path.
  useEffect(() => {
    if (!client || !isConnected) {
      return;
    }

    void sessionsStore.getState().loadSessions();
    // Warm only the active mode model path on connect. Background modes can
    // resolve models when first focused / first used.
    void ensureModelReady(modeStores[activeMode].session);
  }, [activeMode, client, ensureModelReady, isConnected, modeStores, sessionsStore]);

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

    // Parallel chat/code/mako loads were a major source of resume thrash and
    // made mode switches feel crashy under load. Background modes warm on
    // first focus instead of all at once.
    const type = activeMode;
    const slot = modeStores[type];
    if (slot.session.getState().sessionId) {
      return;
    }
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
    const targetId = persisted?.id ?? recent?.id ?? null;
    if (
      !targetId ||
      attemptedWorkspaceSessionHydrationRef.current[type] === targetId
    ) {
      return;
    }

    attemptedWorkspaceSessionHydrationRef.current[type] = targetId;
    lastSessionIdByTypeRef.current[type] = targetId;
    void slot.session.getState().loadSession(targetId, true).catch(() => {
      void sessionsStore.getState().loadSessions();
    });
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
        return;
      }

      const activeSlot = modeStores[activeMode];
      const currentSessionId =
        activeSlot.session.getState().sessionId ??
        activeSlot.workspace.getState().sessionId;
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
