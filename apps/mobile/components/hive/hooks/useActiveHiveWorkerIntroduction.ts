import { useCallback, useEffect, useRef, useState } from "react";
import type {
  HiveWorker,
  HiveWorkerDetail,
  HiveWorkerIntroduction,
  HiveWorkerIntroductionSelectedFact,
} from "@mitsuro/api";
import type { HiveWorkersState } from "./useHiveWorkers";
import {
  canAdoptHiveWorkerIntroductionAction,
  type HiveWorkerIntroductionActionBinding,
} from "./workerIntroductionBinding";

const REVIEW_POLL_MS = 1_200;

type ExactHiveWorkerIntroductionActionBinding =
  & HiveWorkerIntroductionActionBinding
  & { workerId: string; sessionId: string };

interface ActiveHiveWorkerIntroductionOptions {
  workers: HiveWorkersState;
  worker: HiveWorker | null;
  sessionId: string | null;
  transcriptTailKey: string;
  isStreaming: boolean;
}

export interface ActiveHiveWorkerIntroductionState {
  worker: HiveWorker | null;
  detail: HiveWorkerDetail | null;
  introduction: HiveWorkerIntroduction | null;
  isLoading: boolean;
  isSaving: boolean;
  error: string | null;
  refresh: () => void;
  resume: () => Promise<void>;
  retry: () => Promise<void>;
  skip: () => Promise<void>;
  confirm: (
    selectedFacts: HiveWorkerIntroductionSelectedFact[],
  ) => Promise<void>;
  keepTalking: () => Promise<void>;
}

export function selectCurrentHiveWorkerIntroductionDetail(
  detail: HiveWorkerDetail | null,
  workerId: string | null,
  sessionId: string | null,
): HiveWorkerDetail | null {
  if (
    !detail || !workerId || !sessionId || detail.id !== workerId ||
    detail.dm_session_id !== sessionId
  ) {
    return null;
  }
  return detail;
}

export function canShowHiveWorkerGoalForIntroduction(
  detail: HiveWorkerDetail | null,
): boolean {
  if (!detail) return false;
  const status = detail.introduction?.status ?? null;
  return status === null || status === "confirmed" || status === "skipped";
}

/**
 * Projects Introduction state for the already-active Hive transcript.
 *
 * This hook deliberately consumes the session/tail/stream values supplied by
 * HiveThreadSurface. It never subscribes to the transcript itself. The active
 * Worker is resolved against the single roster owned by HiveScreen, and only
 * core-authored `should_poll` can schedule another detail read.
 */
export function useActiveHiveWorkerIntroduction({
  workers,
  worker: activeWorker,
  sessionId,
  transcriptTailKey,
  isStreaming,
}: ActiveHiveWorkerIntroductionOptions): ActiveHiveWorkerIntroductionState {
  const { loadWorkerDetail } = workers;
  const [detail, setDetail] = useState<HiveWorkerDetail | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [localError, setLocalError] = useState<
    {
      workerId: string;
      sessionId: string;
      message: string;
    } | null
  >(null);
  const [refreshRevision, setRefreshRevision] = useState(0);
  const generationRef = useRef(0);
  const bindingRef = useRef<
    { workerId: string | null; sessionId: string | null }
  >({
    workerId: null,
    sessionId: null,
  });
  bindingRef.current = { workerId: activeWorker?.id ?? null, sessionId };
  const abortRef = useRef<AbortController | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const currentDetail = selectCurrentHiveWorkerIntroductionDetail(
    detail,
    activeWorker?.id ?? null,
    sessionId,
  );
  const currentError = activeWorker && sessionId &&
      localError?.workerId === activeWorker.id &&
      localError.sessionId === sessionId
    ? localError.message
    : null;
  const currentDetailRef = useRef<HiveWorkerDetail | null>(currentDetail);
  currentDetailRef.current = currentDetail;

  useEffect(() => {
    const generation = ++generationRef.current;
    abortRef.current?.abort();
    abortRef.current = null;
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }

    if (!activeWorker || !sessionId) {
      setDetail(null);
      setIsLoading(false);
      setLocalError(null);
      return;
    }

    let disposed = false;
    const load = async (showLoading: boolean): Promise<void> => {
      if (disposed || generation !== generationRef.current) return;
      const controller = new AbortController();
      abortRef.current?.abort();
      abortRef.current = controller;
      if (showLoading) {
        setIsLoading(true);
        setLocalError(null);
      }
      let next: HiveWorkerDetail | null;
      try {
        next = await loadWorkerDetail(activeWorker.id, {
          signal: controller.signal,
          throwOnError: true,
        });
      } catch (loadError: unknown) {
        if (
          disposed || controller.signal.aborted ||
          generation !== generationRef.current ||
          bindingRef.current.workerId !== activeWorker.id ||
          bindingRef.current.sessionId !== sessionId
        ) {
          return;
        }
        setDetail(null);
        setIsLoading(false);
        setLocalError({
          workerId: activeWorker.id,
          sessionId,
          message: loadError instanceof Error
            ? loadError.message
            : "Failed to load the Worker",
        });
        return;
      }
      if (
        disposed ||
        controller.signal.aborted ||
        generation !== generationRef.current
      ) {
        return;
      }
      setIsLoading(false);
      if (
        !next ||
        next.id !== activeWorker.id ||
        next.dm_session_id !== sessionId
      ) {
        setDetail(null);
        setLocalError({
          workerId: activeWorker.id,
          sessionId,
          message: "Worker detail did not match this conversation",
        });
        return;
      }
      setLocalError(null);
      setDetail(next);
      if (next.introduction?.review_projection.should_poll) {
        timerRef.current = setTimeout(() => {
          timerRef.current = null;
          void load(false);
        }, REVIEW_POLL_MS);
      }
    };

    void load(true);
    return () => {
      disposed = true;
      if (generation === generationRef.current) {
        abortRef.current?.abort();
        abortRef.current = null;
      }
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [
    activeWorker,
    isStreaming,
    loadWorkerDetail,
    refreshRevision,
    sessionId,
    transcriptTailKey,
  ]);

  const adopt = useCallback(
    (
      next: HiveWorkerDetail,
      expected: HiveWorkerIntroductionActionBinding,
    ) => {
      const current = bindingRef.current;
      if (
        !canAdoptHiveWorkerIntroductionAction(
          { generation: generationRef.current, ...current },
          expected,
          { workerId: next.id, sessionId: next.dm_session_id ?? null },
        )
      ) {
        return;
      }
      setDetail(next);
    },
    [],
  );

  const captureActionBinding = useCallback(
    (): ExactHiveWorkerIntroductionActionBinding | null => {
      const current = bindingRef.current;
      if (!current.workerId || !current.sessionId) return null;
      return {
        generation: generationRef.current,
        workerId: current.workerId,
        sessionId: current.sessionId,
      };
    },
    [],
  );

  const captureCurrentDetailAction = useCallback(() => {
    const expected = captureActionBinding();
    if (!expected) return null;
    const exactDetail = selectCurrentHiveWorkerIntroductionDetail(
      currentDetailRef.current,
      expected.workerId,
      expected.sessionId,
    );
    return exactDetail ? { detail: exactDetail, expected } : null;
  }, [captureActionBinding]);

  const isCurrentActionBinding = useCallback(
    (expected: HiveWorkerIntroductionActionBinding) => {
      const current = bindingRef.current;
      return generationRef.current === expected.generation &&
        current.workerId === expected.workerId &&
        current.sessionId === expected.sessionId;
    },
    [],
  );

  const runCurrentAction = useCallback(
    async (
      expected: ExactHiveWorkerIntroductionActionBinding,
      action: () => Promise<HiveWorkerDetail>,
    ) => {
      if (isCurrentActionBinding(expected)) setLocalError(null);
      try {
        adopt(await action(), expected);
      } catch (actionError: unknown) {
        if (isCurrentActionBinding(expected)) {
          setLocalError({
            workerId: expected.workerId,
            sessionId: expected.sessionId,
            message: actionError instanceof Error
              ? actionError.message
              : "Failed to update the Worker Introduction",
          });
        }
        throw actionError;
      }
    },
    [adopt, isCurrentActionBinding],
  );

  const retry = useCallback(async () => {
    const action = captureCurrentDetailAction();
    if (!action) return;
    await runCurrentAction(
      action.expected,
      () => workers.retryIntroduction(action.detail.id),
    );
  }, [captureCurrentDetailAction, runCurrentAction, workers.retryIntroduction]);

  const resume = useCallback(async () => {
    const action = captureCurrentDetailAction();
    if (!action || action.detail.status !== "paused") return;
    await runCurrentAction(
      action.expected,
      () => workers.resumeWorker(action.detail.id),
    );
  }, [captureCurrentDetailAction, runCurrentAction, workers.resumeWorker]);

  const skip = useCallback(async () => {
    const action = captureCurrentDetailAction();
    if (!action) return;
    await runCurrentAction(
      action.expected,
      () => workers.skipIntroduction(action.detail.id),
    );
  }, [captureCurrentDetailAction, runCurrentAction, workers.skipIntroduction]);

  const confirm = useCallback(
    async (selectedFacts: HiveWorkerIntroductionSelectedFact[]) => {
      const action = captureCurrentDetailAction();
      const introduction = action?.detail.introduction;
      const proposal = introduction?.proposal;
      const projection = introduction?.review_projection;
      if (
        !action ||
        !proposal ||
        selectedFacts.length === 0 ||
        introduction.status !== "review_ready" ||
        projection?.state !== "review_ready" ||
        !projection.is_current_through ||
        proposal.worker_id !== action.expected.workerId ||
        proposal.session_id !== action.expected.sessionId
      ) {
        return;
      }
      await runCurrentAction(
        action.expected,
        () =>
          workers.confirmIntroduction(action.detail.id, {
            proposal_id: proposal.proposal_id,
            proposal_revision: proposal.revision,
            selected_facts: selectedFacts,
          }),
      );
    },
    [
      captureCurrentDetailAction,
      runCurrentAction,
      workers.confirmIntroduction,
    ],
  );

  const keepTalking = useCallback(async () => {
    const action = captureCurrentDetailAction();
    const proposal = action?.detail.introduction?.proposal;
    if (
      !action ||
      !proposal ||
      proposal.worker_id !== action.expected.workerId ||
      proposal.session_id !== action.expected.sessionId
    ) {
      return;
    }
    await runCurrentAction(
      action.expected,
      () =>
        workers.keepTalkingIntroduction(action.detail.id, {
          proposal_id: proposal.proposal_id,
          proposal_revision: proposal.revision,
        }),
    );
  }, [
    captureCurrentDetailAction,
    runCurrentAction,
    workers.keepTalkingIntroduction,
  ]);

  return {
    worker: activeWorker,
    detail: currentDetail,
    introduction: currentDetail?.introduction ?? null,
    isLoading,
    isSaving: workers.isSaving,
    error: currentError,
    refresh: () => {
      setLocalError(null);
      setRefreshRevision((current) => current + 1);
    },
    resume,
    retry,
    skip,
    confirm,
    keepTalking,
  };
}
