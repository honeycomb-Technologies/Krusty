export interface WorkerGoalMutationBinding {
  generation: number;
  workerId: string;
  sessionId: string;
}

export function isCurrentWorkerGoalMutation(
  expected: WorkerGoalMutationBinding,
  generation: number,
  workerId: string | null,
  sessionId: string | null,
): boolean {
  return expected.generation === generation &&
    expected.workerId === workerId &&
    expected.sessionId === sessionId;
}
