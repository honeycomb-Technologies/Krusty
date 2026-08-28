import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { HiveWorker, HiveWorkerDetail } from "@mitsuro/api";
import type { HiveWorkersState } from "./useHiveWorkers";
import {
  canAdoptWorkerSessionBinding,
  isCurrentWorkerSessionLookup,
} from "./workerSessionBindingFence";

export type ActiveHiveConversationKind =
  | "none"
  | "resolving"
  | "primary_hive"
  | "worker_dm";

export interface ActiveHiveWorkerBinding {
  kind: ActiveHiveConversationKind;
  worker: HiveWorker | null;
  detail: HiveWorkerDetail | null;
  isResolving: boolean;
  error: string | null;
  /** Re-run the exact active-session lookup after a surfaced failure. */
  retry: () => void;
}

interface ResolvedBinding {
  sessionId: string;
  kind: "invalid" | "primary_hive" | "worker_dm";
  worker: HiveWorkerDetail | null;
}

const BINDING_AUTO_RETRY_DELAYS_MS = [200, 600] as const;
const BINDING_UNAVAILABLE_ERROR =
  "Unable to resolve this Hive conversation. Check the connection and try again.";

function lookupErrorMessage(error: unknown): string {
  return error instanceof Error
    ? error.message
    : "Failed to resolve the Hive conversation";
}

function isTransientLookupError(error: unknown): boolean {
  if (!error || typeof error !== "object" || !("status" in error)) {
    // Fetch/network/decode failures do not carry an HTTP status.
    return true;
  }
  const status = (error as { status?: unknown }).status;
  return typeof status !== "number" || status <= 0 || status === 408 ||
    status === 425 || status === 429 || (status >= 500 && status <= 599);
}

function waitForRetryDelay(
  delayMs: number,
  signal: AbortSignal,
): Promise<boolean> {
  return new Promise((resolve) => {
    if (signal.aborted) {
      resolve(false);
      return;
    }

    let settled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const finish = (completed: boolean) => {
      if (settled) return;
      settled = true;
      if (timer !== null) clearTimeout(timer);
      signal.removeEventListener("abort", handleAbort);
      resolve(completed);
    };
    const handleAbort = () => finish(false);
    signal.addEventListener("abort", handleAbort, { once: true });
    timer = setTimeout(() => finish(true), delayMs);
  });
}

/**
 * Resolves the already-mounted Hive transcript to its exact durable surface.
 * The shared roster provides an immediate active/paused Worker candidate;
 * the direct lookup then handles archived Workers and explicitly proves the
 * primary Hive case. Requests are abortable and fenced to the session
 * generation, so a late Worker A lookup can never bind Worker B's transcript.
 */
export function useActiveHiveWorkerBinding(
  workers: HiveWorkersState,
  sessionId: string | null,
): ActiveHiveWorkerBinding {
  const rosterWorker = useMemo(
    () =>
      sessionId
        ? workers.workers.find((worker) =>
          worker.dm_session_id === sessionId
        ) ??
          null
        : null,
    [sessionId, workers.workers],
  );
  const [resolved, setResolved] = useState<ResolvedBinding | null>(null);
  const [scopedError, setScopedError] = useState<
    {
      sessionId: string;
      message: string;
    } | null
  >(null);
  const [retryRevision, setRetryRevision] = useState(0);
  const [directVerificationSessionId, setDirectVerificationSessionId] =
    useState<string | null>(null);
  const generationRef = useRef(0);
  const sessionIdRef = useRef(sessionId);
  const abortRef = useRef<AbortController | null>(null);
  sessionIdRef.current = sessionId;
  const error = scopedError?.sessionId === sessionId
    ? scopedError.message
    : null;

  const retry = useCallback(() => {
    setDirectVerificationSessionId(sessionIdRef.current);
    setResolved(null);
    setScopedError(null);
    setRetryRevision((current) => current + 1);
  }, []);

  useEffect(() => {
    const generation = ++generationRef.current;
    abortRef.current?.abort();
    abortRef.current = null;
    setScopedError(null);

    if (!sessionId) {
      setResolved(null);
      return;
    }
    if (workers.isLoading && !rosterWorker) {
      setResolved(null);
      return;
    }

    const controller = new AbortController();
    abortRef.current = controller;
    void (async () => {
      for (
        let attempt = 0;
        attempt <= BINDING_AUTO_RETRY_DELAYS_MS.length;
        attempt += 1
      ) {
        try {
          const response = await workers.loadWorkerBySession(sessionId, {
            signal: controller.signal,
          });
          if (
            controller.signal.aborted ||
            !isCurrentWorkerSessionLookup(
              generation,
              sessionId,
              generationRef.current,
              sessionIdRef.current,
            )
          ) {
            return;
          }
          if (!response) {
            throw new Error(BINDING_UNAVAILABLE_ERROR);
          }
          if (
            !canAdoptWorkerSessionBinding(
              generation,
              sessionId,
              generationRef.current,
              sessionIdRef.current,
              response.session_id,
            )
          ) {
            setResolved({ sessionId, kind: "invalid", worker: null });
            setScopedError({
              sessionId,
              message: "Hive conversation lookup returned a different session",
            });
            return;
          }
          if (response.kind === "worker_dm") {
            if (response.worker.dm_session_id !== sessionId) {
              setResolved({ sessionId, kind: "invalid", worker: null });
              setScopedError({
                sessionId,
                message: "Hive Worker lookup returned a mismatched DM binding",
              });
              return;
            }
            setResolved({
              sessionId,
              kind: "worker_dm",
              worker: response.worker,
            });
            setDirectVerificationSessionId(null);
            return;
          }
          if (response.kind !== "primary_hive") {
            setResolved({ sessionId, kind: "invalid", worker: null });
            setScopedError({
              sessionId,
              message: "Hive conversation lookup returned an unknown binding",
            });
            return;
          }
          if (rosterWorker) {
            setResolved({ sessionId, kind: "invalid", worker: null });
            setScopedError({
              sessionId,
              message:
                "Hive conversation binding conflicts with the Worker roster",
            });
            return;
          }
          setResolved({ sessionId, kind: "primary_hive", worker: null });
          setDirectVerificationSessionId(null);
          return;
        } catch (lookupError: unknown) {
          if (
            controller.signal.aborted ||
            !isCurrentWorkerSessionLookup(
              generation,
              sessionId,
              generationRef.current,
              sessionIdRef.current,
            )
          ) {
            return;
          }

          const retryDelay = BINDING_AUTO_RETRY_DELAYS_MS[attempt];
          if (
            retryDelay !== undefined && isTransientLookupError(lookupError)
          ) {
            const shouldContinue = await waitForRetryDelay(
              retryDelay,
              controller.signal,
            );
            if (shouldContinue) continue;
            return;
          }

          setResolved({ sessionId, kind: "invalid", worker: null });
          setScopedError({
            sessionId,
            message: lookupErrorMessage(lookupError),
          });
          return;
        }
      }
    })();

    return () => {
      if (generation === generationRef.current) {
        controller.abort();
        abortRef.current = null;
      }
    };
  }, [
    retryRevision,
    rosterWorker,
    sessionId,
    workers.isLoading,
    workers.loadWorkerBySession,
  ]);

  if (!sessionId) {
    return {
      kind: "none",
      worker: null,
      detail: null,
      isResolving: false,
      error: null,
      retry,
    };
  }
  if (resolved?.sessionId === sessionId && resolved.kind === "worker_dm") {
    return {
      kind: "worker_dm",
      worker: resolved.worker,
      detail: resolved.worker,
      isResolving: false,
      error,
      retry,
    };
  }
  if (resolved?.sessionId === sessionId && resolved.kind === "primary_hive") {
    return {
      kind: "primary_hive",
      worker: null,
      detail: null,
      isResolving: false,
      error,
      retry,
    };
  }
  if (resolved?.sessionId === sessionId && resolved.kind === "invalid") {
    return {
      kind: "resolving",
      worker: null,
      detail: null,
      isResolving: false,
      error,
      retry,
    };
  }
  if (rosterWorker && directVerificationSessionId !== sessionId) {
    return {
      kind: "worker_dm",
      worker: rosterWorker,
      detail: null,
      isResolving: false,
      error,
      retry,
    };
  }
  return {
    kind: "resolving",
    worker: null,
    detail: null,
    isResolving: true,
    error,
    retry,
  };
}
