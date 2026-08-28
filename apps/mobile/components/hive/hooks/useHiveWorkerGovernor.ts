import { useCallback, useEffect, useRef, useState } from "react";
import type {
  HiveWorker,
  HiveWorkerGovernorProjection,
  HiveWorkerGovernorRecoveryResponse,
} from "@mitsuro/api";

import { useConnection } from "../../../hooks/useConnection";

const GOVERNOR_POLL_MS = 15_000;

interface UseHiveWorkerGovernorOptions {
  worker: HiveWorker | null;
  sessionId: string | null;
  enabled: boolean;
  poll?: boolean;
}

export interface HiveWorkerGovernorState {
  projection: HiveWorkerGovernorProjection | null;
  recoveryGrant: HiveWorkerGovernorRecoveryResponse | null;
  isLoading: boolean;
  isGrantingRecovery: boolean;
  error: string | null;
  recoveryError: string | null;
  refresh: () => void;
  grantRecovery: () => Promise<void>;
}

interface RecoveryAttempt {
  workerId: string;
  sessionId: string;
  idempotencyKey: string;
}

interface ScopedGovernorValue<T> {
  bindingKey: string;
  value: T;
}

function governorBindingKey(
  workerId: string | null,
  workerRevision: number | null,
  sessionId: string | null,
): string | null {
  return workerId && workerRevision !== null && sessionId
    ? JSON.stringify([workerId, workerRevision, sessionId])
    : null;
}

function recoveryKey(workerId: string): string {
  const nonce = globalThis.crypto?.randomUUID?.() ??
    `${Date.now()}:${Math.random().toString(36).slice(2)}`;
  return `worker-governor-recovery:${workerId}:${nonce}`;
}

/**
 * Reads one aggregate-only Worker-DM governor projection. Binding generation
 * and AbortSignal fences prevent a late Worker A response from appearing in
 * Worker B after navigation or a profile revision change.
 */
export function useHiveWorkerGovernor({
  worker,
  sessionId,
  enabled,
  poll = true,
}: UseHiveWorkerGovernorOptions): HiveWorkerGovernorState {
  const { client, isConnected } = useConnection();
  const workerId = worker?.id ?? null;
  const workerRevision = worker?.revision ?? null;
  const workerDmSessionId = worker?.dm_session_id ?? null;
  const exactBindingKey = governorBindingKey(
    workerId,
    workerRevision,
    sessionId,
  );
  const [projectionState, setProjectionState] = useState<
    ScopedGovernorValue<HiveWorkerGovernorProjection> | null
  >(null);
  const projection = projectionState?.bindingKey === exactBindingKey
    ? projectionState.value
    : null;
  const [isLoading, setIsLoading] = useState(false);
  const [loadingBindingKey, setLoadingBindingKey] = useState<string | null>(
    null,
  );
  const [errorState, setErrorState] = useState<
    ScopedGovernorValue<string> | null
  >(null);
  const error = errorState?.bindingKey === exactBindingKey
    ? errorState.value
    : null;
  const [recoveryGrantState, setRecoveryGrantState] = useState<
    ScopedGovernorValue<HiveWorkerGovernorRecoveryResponse> | null
  >(null);
  const recoveryGrant = recoveryGrantState?.bindingKey === exactBindingKey
    ? recoveryGrantState.value
    : null;
  const [isGrantingRecovery, setIsGrantingRecovery] = useState(false);
  const [grantingBindingKey, setGrantingBindingKey] = useState<string | null>(
    null,
  );
  const [recoveryErrorState, setRecoveryErrorState] = useState<
    ScopedGovernorValue<string> | null
  >(null);
  const recoveryError = recoveryErrorState?.bindingKey === exactBindingKey
    ? recoveryErrorState.value
    : null;
  const [refreshRevision, setRefreshRevision] = useState(0);
  const loadGenerationRef = useRef(0);
  const bindingGenerationRef = useRef(1);
  const bindingRef = useRef({ workerId, workerRevision, sessionId });
  const nextBinding = { workerId, workerRevision, sessionId };
  if (
    bindingRef.current.workerId !== nextBinding.workerId ||
    bindingRef.current.workerRevision !== nextBinding.workerRevision ||
    bindingRef.current.sessionId !== nextBinding.sessionId
  ) {
    bindingRef.current = nextBinding;
    bindingGenerationRef.current += 1;
    loadGenerationRef.current += 1;
  }
  const abortRef = useRef<AbortController | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const recoveryAttemptRef = useRef<RecoveryAttempt | null>(null);
  const recoveryInFlightRef = useRef(false);

  const refresh = useCallback(() => {
    setRefreshRevision((revision) => revision + 1);
  }, []);

  useEffect(() => {
    recoveryAttemptRef.current = null;
    recoveryInFlightRef.current = false;
    setRecoveryGrantState(null);
    setRecoveryErrorState(null);
    setIsGrantingRecovery(false);
    setGrantingBindingKey(null);
  }, [sessionId, workerId, workerRevision]);

  useEffect(() => {
    if (!recoveryGrant) return;
    if (recoveryGrant.status === "response_loss_acknowledged") return;
    const expiresAt = Date.parse(recoveryGrant.expires_at);
    const clearConfirmedGrant = () => {
      setRecoveryGrantState((current) =>
        current?.value.grant_id === recoveryGrant.grant_id ? null : current
      );
    };
    const expireConfirmedGrant = () => {
      clearConfirmedGrant();
      if (exactBindingKey) {
        setRecoveryErrorState({
          bindingKey: exactBindingKey,
          value:
            "One-call recovery expired before provider admission. Prepare a fresh short-lived recovery",
        });
      }
    };
    if (!Number.isFinite(expiresAt)) {
      clearConfirmedGrant();
      if (exactBindingKey) {
        setRecoveryErrorState({
          bindingKey: exactBindingKey,
          value:
            "Worker recovery returned an invalid expiry. Prepare a fresh short-lived recovery",
        });
      }
      return;
    }
    const remainingMs = expiresAt - Date.now();
    if (remainingMs <= 0) {
      expireConfirmedGrant();
      return;
    }
    const expiryTimer = setTimeout(
      expireConfirmedGrant,
      Math.min(remainingMs, 2_147_483_647),
    );
    return () => clearTimeout(expiryTimer);
  }, [exactBindingKey, recoveryGrant]);

  useEffect(() => {
    if (
      projection?.unresolved_started_count !== 0 ||
      projection?.response_loss_recovery_required
    ) return;
    recoveryAttemptRef.current = null;
    setRecoveryGrantState(null);
    setRecoveryErrorState(null);
  }, [
    projection?.response_loss_recovery_required,
    projection?.unresolved_started_count,
  ]);

  useEffect(() => {
    const generation = ++loadGenerationRef.current;
    abortRef.current?.abort();
    abortRef.current = null;
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    setErrorState(null);

    const exactDmBinding = Boolean(
      enabled && workerId && sessionId && workerDmSessionId === sessionId,
    );
    if (
      !client || !isConnected || !exactDmBinding || !workerId || !sessionId ||
      !exactBindingKey
    ) {
      setProjectionState(null);
      setIsLoading(false);
      setLoadingBindingKey(null);
      return;
    }

    const controller = new AbortController();
    abortRef.current = controller;
    setIsLoading(true);
    setLoadingBindingKey(exactBindingKey);
    void client
      .getHiveWorkerGovernor(workerId, { signal: controller.signal })
      .then((next) => {
        const binding = bindingRef.current;
        if (
          controller.signal.aborted ||
          generation !== loadGenerationRef.current ||
          binding.workerId !== workerId ||
          binding.sessionId !== sessionId
        ) {
          return;
        }
        if (
          next.worker_id !== workerId || next.dm_session_id !== sessionId ||
          next.policy.worker_id !== workerId
        ) {
          setProjectionState(null);
          setErrorState({
            bindingKey: exactBindingKey,
            value: "Worker limits returned a different private conversation",
          });
          return;
        }
        setProjectionState({ bindingKey: exactBindingKey, value: next });
        if (poll) {
          timerRef.current = setTimeout(() => {
            timerRef.current = null;
            setRefreshRevision((revision) => revision + 1);
          }, GOVERNOR_POLL_MS);
        }
      })
      .catch((loadError: unknown) => {
        if (
          controller.signal.aborted ||
          generation !== loadGenerationRef.current ||
          (loadError instanceof Error && loadError.name === "AbortError")
        ) {
          return;
        }
        setProjectionState(null);
        setErrorState({
          bindingKey: exactBindingKey,
          value: loadError instanceof Error
            ? loadError.message
            : "Failed to load Worker limits",
        });
      })
      .finally(() => {
        if (
          generation === loadGenerationRef.current &&
          !controller.signal.aborted
        ) {
          setIsLoading(false);
          setLoadingBindingKey(null);
        }
      });

    return () => {
      controller.abort();
      if (abortRef.current === controller) abortRef.current = null;
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [
    client,
    enabled,
    exactBindingKey,
    isConnected,
    poll,
    refreshRevision,
    sessionId,
    workerDmSessionId,
    workerId,
    workerRevision,
  ]);

  const grantRecovery = useCallback(async (): Promise<void> => {
    const binding = bindingRef.current;
    const bindingGeneration = bindingGenerationRef.current;
    if (
      !client || !isConnected || recoveryInFlightRef.current || !workerId ||
      !sessionId || !exactBindingKey || binding.workerId !== workerId ||
      binding.sessionId !== sessionId || workerDmSessionId !== sessionId ||
      !projection || projection.worker_id !== workerId ||
      projection.dm_session_id !== sessionId ||
      (projection.unresolved_started_count <= 0 &&
        !projection.response_loss_recovery_required)
    ) {
      return;
    }

    let attempt = recoveryAttemptRef.current;
    if (
      !attempt || attempt.workerId !== workerId ||
      attempt.sessionId !== sessionId
    ) {
      attempt = {
        workerId,
        sessionId,
        idempotencyKey: recoveryKey(workerId),
      };
      recoveryAttemptRef.current = attempt;
    }
    recoveryInFlightRef.current = true;
    setIsGrantingRecovery(true);
    setGrantingBindingKey(exactBindingKey);
    setRecoveryErrorState(null);
    try {
      const response = await client.grantHiveWorkerGovernorRecovery(workerId, {
        idempotencyKey: attempt.idempotencyKey,
      });
      if (
        bindingGeneration !== bindingGenerationRef.current ||
        bindingRef.current.workerId !== workerId ||
        bindingRef.current.sessionId !== sessionId
      ) {
        return;
      }
      const safeGrant = (response.status === "granted" ||
        response.status === "already_available" ||
        response.status === "response_loss_acknowledged_with_grant") &&
        response.bypass_unresolved_provider_call === true &&
        Boolean(response.grant_id) && Boolean(response.expires_at);
      const responseLossAcknowledged =
        response.status === "response_loss_acknowledged" &&
        response.bypass_unresolved_provider_call === false &&
        response.grant_id === null && response.expires_at === null;
      if (
        response.worker_id !== workerId ||
        (!safeGrant && !responseLossAcknowledged)
      ) {
        throw new Error(
          "Worker recovery returned a different or unsafe result",
        );
      }
      recoveryAttemptRef.current = null;
      setRecoveryGrantState({ bindingKey: exactBindingKey, value: response });
      setRefreshRevision((revision) => revision + 1);
    } catch (grantError: unknown) {
      if (
        bindingGeneration !== bindingGenerationRef.current ||
        bindingRef.current.workerId !== workerId ||
        bindingRef.current.sessionId !== sessionId
      ) {
        return;
      }
      const message = grantError instanceof Error
        ? grantError.message
        : "Failed to prepare one-call Worker recovery";
      setRecoveryErrorState({
        bindingKey: exactBindingKey,
        value: `${message}. Retry replays the same recovery request safely`,
      });
    } finally {
      if (
        bindingGeneration === bindingGenerationRef.current &&
        bindingRef.current.workerId === workerId &&
        bindingRef.current.sessionId === sessionId
      ) {
        recoveryInFlightRef.current = false;
        setIsGrantingRecovery(false);
        setGrantingBindingKey(null);
      }
    }
  }, [
    client,
    exactBindingKey,
    isConnected,
    projection,
    sessionId,
    workerDmSessionId,
    workerId,
  ]);

  return {
    projection,
    recoveryGrant,
    isLoading: isLoading && loadingBindingKey === exactBindingKey,
    isGrantingRecovery: isGrantingRecovery &&
      grantingBindingKey === exactBindingKey,
    error,
    recoveryError,
    refresh,
    grantRecovery,
  };
}
