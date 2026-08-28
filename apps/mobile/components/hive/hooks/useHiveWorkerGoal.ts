import { useCallback, useEffect, useRef, useState } from "react";
import {
  type HiveWorker,
  type HiveWorkerGoalProjection,
  MitsuroApiError,
  type MitsuroClient,
} from "@mitsuro/api";

import { useConnection } from "../../../hooks/useConnection";
import {
  isCurrentWorkerGoalMutation,
  type WorkerGoalMutationBinding,
} from "./workerGoalBindingFence";

const ACTIVE_GOAL_POLL_MS = 5_000;

interface UseHiveWorkerGoalOptions {
  worker: HiveWorker | null;
  sessionId: string | null;
  transcriptTailKey: string;
  isStreaming: boolean;
}

interface MutationAttempt {
  fingerprint: string;
  idempotencyKey: string;
}

export interface HiveWorkerGoalDraft {
  title: string;
  objective: string;
  successCriterion: string;
  planSteps: string;
}

export interface HiveWorkerGoalCriterionReview {
  criterionId: string;
  decision: "passed" | "waived";
  evidence: string;
}

export interface HiveWorkerGoalAcceptanceReview {
  reason: string;
  criteria: HiveWorkerGoalCriterionReview[];
}

export interface HiveWorkerGoalState {
  projection: HiveWorkerGoalProjection | null;
  isLoading: boolean;
  isSaving: boolean;
  error: string | null;
  refresh: () => void;
  create: (draft: HiveWorkerGoalDraft) => Promise<void>;
  approve: () => Promise<void>;
  activate: () => Promise<void>;
  pause: () => Promise<void>;
  cancel: () => Promise<void>;
  accept: (review: HiveWorkerGoalAcceptanceReview) => Promise<void>;
  reject: (reason: string) => Promise<void>;
  setWorkspace: (path: string) => Promise<void>;
}

export function selectCurrentHiveWorkerGoalProjection(
  projection: HiveWorkerGoalProjection | null,
  workerId: string | null,
  sessionId: string | null,
): HiveWorkerGoalProjection | null {
  if (
    !projection || !workerId || !sessionId ||
    projection.worker_id !== workerId || projection.session_id !== sessionId
  ) {
    return null;
  }
  return projection;
}

function mutationKey(workerId: string, action: string): string {
  const nonce = globalThis.crypto?.randomUUID?.() ??
    `${Date.now()}:${Math.random().toString(36).slice(2)}`;
  return `worker-goal:${workerId}:${action}:${nonce}`;
}

/**
 * Owns the active Worker's Goal projection without subscribing to transcript
 * state itself. Every read and mutation is fenced to the Worker/DM generation;
 * a lost mutation response retains its exact key and body for safe replay.
 */
export function useHiveWorkerGoal({
  worker,
  sessionId,
  transcriptTailKey,
  isStreaming,
}: UseHiveWorkerGoalOptions): HiveWorkerGoalState {
  const { client, isConnected } = useConnection();
  const workerId = worker?.id ?? null;
  const workerRevision = worker?.revision ?? null;
  const [projection, setProjection] = useState<HiveWorkerGoalProjection | null>(
    null,
  );
  const currentProjection = selectCurrentHiveWorkerGoalProjection(
    projection,
    workerId,
    sessionId,
  );
  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshRevision, setRefreshRevision] = useState(0);
  const loadGenerationRef = useRef(0);
  const bindingGenerationRef = useRef(1);
  const bindingRef = useRef<{
    workerId: string | null;
    sessionId: string | null;
  }>({
    workerId,
    sessionId,
  });
  const nextBinding = { workerId, sessionId };
  if (
    bindingRef.current.workerId !== nextBinding.workerId ||
    bindingRef.current.sessionId !== nextBinding.sessionId
  ) {
    bindingGenerationRef.current += 1;
    bindingRef.current = nextBinding;
  }
  const abortRef = useRef<AbortController | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const attemptsRef = useRef<Map<string, MutationAttempt>>(new Map());
  const savingAttemptRef = useRef<string | null>(null);
  const mutationSerialRef = useRef(0);

  const refresh = useCallback(() => {
    setRefreshRevision((revision) => revision + 1);
  }, []);

  useEffect(() => {
    attemptsRef.current.clear();
    savingAttemptRef.current = null;
    setIsSaving(false);
    setError(null);
  }, [sessionId, workerId]);

  useEffect(() => {
    const generation = ++loadGenerationRef.current;
    abortRef.current?.abort();
    abortRef.current = null;
    if (timerRef.current) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    setError(null);

    if (!client || !isConnected || !workerId || !sessionId) {
      setProjection(null);
      setIsLoading(false);
      return;
    }

    const controller = new AbortController();
    abortRef.current = controller;
    setIsLoading(true);
    void client
      .getHiveWorkerGoal(workerId, { signal: controller.signal })
      .then((next) => {
        if (
          controller.signal.aborted ||
          generation !== loadGenerationRef.current ||
          bindingRef.current.workerId !== workerId ||
          bindingRef.current.sessionId !== sessionId
        ) {
          return;
        }
        if (next.worker_id !== workerId || next.session_id !== sessionId) {
          setProjection(null);
          setError("Worker Goal lookup returned a different conversation");
          return;
        }
        setProjection(next);
        if (
          next.active_run || next.pending_acceptance ||
          next.workflow?.goal.status === "active"
        ) {
          timerRef.current = setTimeout(() => {
            timerRef.current = null;
            setRefreshRevision((revision) => revision + 1);
          }, ACTIVE_GOAL_POLL_MS);
        }
      })
      .catch((loadError: unknown) => {
        if (
          controller.signal.aborted ||
          generation !== loadGenerationRef.current ||
          bindingRef.current.workerId !== workerId ||
          bindingRef.current.sessionId !== sessionId ||
          (loadError instanceof Error && loadError.name === "AbortError")
        ) {
          return;
        }
        setProjection(null);
        setError(
          loadError instanceof Error
            ? loadError.message
            : "Failed to load the Worker Goal",
        );
      })
      .finally(() => {
        if (
          generation === loadGenerationRef.current &&
          !controller.signal.aborted
        ) {
          setIsLoading(false);
        }
      });

    return () => {
      if (generation === loadGenerationRef.current) {
        controller.abort();
        abortRef.current = null;
      }
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [
    client,
    isConnected,
    isStreaming,
    refreshRevision,
    sessionId,
    transcriptTailKey,
    workerId,
    workerRevision,
  ]);

  const captureBinding = useCallback((): WorkerGoalMutationBinding | null => {
    const current = bindingRef.current;
    if (!current.workerId || !current.sessionId) return null;
    return {
      generation: bindingGenerationRef.current,
      workerId: current.workerId,
      sessionId: current.sessionId,
    };
  }, []);

  const adopt = useCallback(
    (
      next: HiveWorkerGoalProjection,
      expected: WorkerGoalMutationBinding,
    ): boolean => {
      const current = bindingRef.current;
      if (
        !isCurrentWorkerGoalMutation(
          expected,
          bindingGenerationRef.current,
          current.workerId,
          current.sessionId,
        ) ||
        next.worker_id !== expected.workerId ||
        next.session_id !== expected.sessionId
      ) {
        return false;
      }
      setProjection(next);
      return true;
    },
    [],
  );

  const mutate = useCallback(
    async (
      action: string,
      body: object,
      run: (
        api: MitsuroClient,
        workerId: string,
        idempotencyKey: string,
      ) => Promise<HiveWorkerGoalProjection>,
    ): Promise<void> => {
      const expected = captureBinding();
      if (!client || !isConnected || !expected) {
        throw new Error("Worker Goal is no longer connected");
      }
      const attemptId = `${expected.workerId}:${action}`;
      const fingerprint = JSON.stringify(body);
      let attempt = attemptsRef.current.get(attemptId);
      if (!attempt || attempt.fingerprint !== fingerprint) {
        attempt = {
          fingerprint,
          idempotencyKey: mutationKey(expected.workerId, action),
        };
        attemptsRef.current.set(attemptId, attempt);
      }

      const savingAttempt = `${attemptId}:${++mutationSerialRef.current}`;
      savingAttemptRef.current = savingAttempt;
      setIsSaving(true);
      setError(null);
      try {
        const next = await run(
          client,
          expected.workerId,
          attempt.idempotencyKey,
        );
        if (adopt(next, expected)) {
          attemptsRef.current.delete(attemptId);
        }
      } catch (mutationError) {
        const current = bindingRef.current;
        const isCurrent = isCurrentWorkerGoalMutation(
          expected,
          bindingGenerationRef.current,
          current.workerId,
          current.sessionId,
        );
        if (
          isCurrent &&
          mutationError instanceof MitsuroApiError &&
          mutationError.status === 409
        ) {
          attemptsRef.current.delete(attemptId);
          refresh();
        }
        if (isCurrent) {
          setError(
            mutationError instanceof Error
              ? mutationError.message
              : "Worker Goal update failed",
          );
        }
        throw mutationError;
      } finally {
        const current = bindingRef.current;
        if (
          savingAttemptRef.current === savingAttempt &&
          isCurrentWorkerGoalMutation(
            expected,
            bindingGenerationRef.current,
            current.workerId,
            current.sessionId,
          )
        ) {
          savingAttemptRef.current = null;
          setIsSaving(false);
        }
      }
    },
    [adopt, captureBinding, client, isConnected, refresh],
  );

  const requireCurrentProjection = useCallback(() => {
    const current = bindingRef.current;
    if (
      !currentProjection ||
      currentProjection.worker_id !== current.workerId ||
      currentProjection.session_id !== current.sessionId
    ) {
      const message = "Worker Goal is no longer available";
      setError(message);
      throw new Error(message);
    }
    return currentProjection;
  }, [currentProjection]);

  const currentFence = useCallback(() => {
    const currentProjection = requireCurrentProjection();
    const workflow = currentProjection.workflow;
    if (!workflow) {
      const message = "Worker Goal is no longer available";
      setError(message);
      throw new Error(message);
    }
    return {
      goal_id: workflow.goal.id,
      expected_worker_revision: currentProjection.worker_revision,
      expected_goal_revision: workflow.aggregate_revision,
    };
  }, [requireCurrentProjection]);

  const create = useCallback(async (draft: HiveWorkerGoalDraft) => {
    const currentProjection = requireCurrentProjection();
    if (!currentProjection.allowed_actions.includes("create_goal")) {
      const message = "This Worker cannot create a Goal right now";
      setError(message);
      throw new Error(message);
    }
    const title = draft.title.trim();
    const objective = draft.objective.trim();
    const successCriterion = draft.successCriterion.trim();
    const planSteps = draft.planSteps.split("\n")
      .map((step) => step.trim())
      .filter(Boolean);
    if (!title || !objective || !successCriterion || planSteps.length === 0) {
      const message =
        "Title, objective, success criterion, and plan steps are required";
      setError(message);
      throw new Error(message);
    }
    if (planSteps.length > 12) {
      const message = "A Worker Goal plan can contain at most 12 steps";
      setError(message);
      throw new Error(message);
    }
    const body = {
      expected_worker_revision: currentProjection.worker_revision,
      goal: {
        title,
        objective,
        constraints: [],
        criteria: [{ description: successCriterion, required: true }],
        token_budget: null,
      },
      plan: {
        title: `${title} plan`,
        rationale: "User-authored Worker Goal plan",
        steps: planSteps.map((description, index) => ({
          display_key: String(index + 1),
          description,
          dependencies: [],
          acceptance_criteria: [`Complete and verify: ${description}`],
          required: true,
        })),
      },
    };
    await mutate(
      "create",
      body,
      (api, workerId, idempotencyKey) =>
        api.createHiveWorkerGoal(workerId, body, { idempotencyKey }),
    );
  }, [mutate, requireCurrentProjection]);

  const approve = useCallback(async () => {
    const fence = currentFence();
    const plan = currentProjection?.workflow?.plan_revision;
    if (!plan) throw new Error("No proposed Worker Goal plan is available");
    const body = { ...fence, plan_revision_id: plan.id };
    await mutate(
      "approve",
      body,
      (api, workerId, idempotencyKey) =>
        api.approveHiveWorkerGoal(workerId, body, { idempotencyKey }),
    );
  }, [currentFence, currentProjection?.workflow?.plan_revision, mutate]);

  const activate = useCallback(async () => {
    const body = currentFence();
    await mutate(
      "activate",
      body,
      (api, workerId, idempotencyKey) =>
        api.activateHiveWorkerGoal(workerId, body, { idempotencyKey }),
    );
  }, [currentFence, mutate]);

  const pause = useCallback(async () => {
    const body = { ...currentFence(), reason: "paused_from_mobile" };
    await mutate(
      "pause",
      body,
      (api, workerId, idempotencyKey) =>
        api.pauseHiveWorkerGoal(workerId, body, { idempotencyKey }),
    );
  }, [currentFence, mutate]);

  const cancel = useCallback(async () => {
    const body = { ...currentFence(), reason: "cancelled_from_mobile" };
    await mutate(
      "cancel",
      body,
      (api, workerId, idempotencyKey) =>
        api.cancelHiveWorkerGoal(workerId, body, { idempotencyKey }),
    );
  }, [currentFence, mutate]);

  const resolveAcceptance = useCallback(async (
    decision: "accept" | "reject",
    review: HiveWorkerGoalAcceptanceReview,
  ) => {
    const currentProjection = requireCurrentProjection();
    const pending = currentProjection.pending_acceptance;
    if (
      !pending ||
      !currentProjection.allowed_actions.includes("resolve_acceptance") ||
      pending.expected_worker_revision !== currentProjection.worker_revision ||
      pending.goal_id !== currentProjection.workflow?.goal.id ||
      pending.expected_goal_revision !==
        currentProjection.workflow?.aggregate_revision
    ) {
      const message = "Worker Goal acceptance is no longer available";
      setError(message);
      throw new Error(message);
    }
    const reason = review.reason.trim();
    if (!reason) {
      const message = "A review reason is required";
      setError(message);
      throw new Error(message);
    }
    const criteria = decision === "reject"
      ? []
      : review.criteria.map((criterion) => ({
        criterion_id: criterion.criterionId,
        decision: criterion.decision,
        evidence: criterion.evidence.trim() ? [criterion.evidence.trim()] : [],
      }));
    if (!pending.is_final_step && criteria.length > 0) {
      throw new Error("Only the final required step can decide Goal criteria");
    }
    if (decision === "accept" && pending.is_final_step) {
      const expected = new Set(
        pending.required_goal_criteria.map((criterion) =>
          criterion.criterion_id
        ),
      );
      const supplied = new Set(
        criteria.map((criterion) => criterion.criterion_id),
      );
      if (
        criteria.length !== expected.size || supplied.size !== expected.size ||
        [...expected].some((criterionId) => !supplied.has(criterionId)) ||
        criteria.some((criterion) => criterion.evidence.length !== 1)
      ) {
        const message =
          "Pass or waive every required Goal criterion with concrete evidence";
        setError(message);
        throw new Error(message);
      }
    }
    const body = {
      expected_worker_revision: pending.expected_worker_revision,
      acceptance_run_id: pending.acceptance_run_id,
      expected_goal_revision: pending.expected_goal_revision,
      decision,
      reason,
      criteria,
    };
    await mutate(
      "resolve_acceptance",
      body,
      (api, workerId, idempotencyKey) =>
        api.resolveHiveWorkerGoalAcceptance(workerId, body, { idempotencyKey }),
    );
  }, [mutate, requireCurrentProjection]);

  const accept = useCallback(
    async (review: HiveWorkerGoalAcceptanceReview) => {
      await resolveAcceptance("accept", review);
    },
    [resolveAcceptance],
  );

  const reject = useCallback(async (reason: string) => {
    await resolveAcceptance("reject", { reason, criteria: [] });
  }, [resolveAcceptance]);

  const setWorkspace = useCallback(async (path: string) => {
    const currentProjection = requireCurrentProjection();
    const body = {
      expected_worker_revision: currentProjection.worker_revision,
      workspace_mode: "selected" as const,
      working_dir: path,
      project_dir: path,
    };
    await mutate(
      "workspace",
      body,
      (api, workerId, idempotencyKey) =>
        api.setHiveWorkerWorkspace(workerId, body, { idempotencyKey }),
    );
  }, [mutate, requireCurrentProjection]);

  return {
    projection: currentProjection,
    isLoading,
    isSaving,
    error,
    refresh,
    create,
    approve,
    activate,
    pause,
    cancel,
    accept,
    reject,
    setWorkspace,
  };
}
