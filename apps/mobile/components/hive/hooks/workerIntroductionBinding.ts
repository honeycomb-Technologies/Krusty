export interface HiveWorkerIntroductionActionBinding {
  generation: number;
  workerId: string | null;
  sessionId: string | null;
}

export interface HiveWorkerIntroductionActionResultBinding {
  workerId: string;
  sessionId: string | null;
}

/**
 * Accept an action response only for the exact active Worker conversation
 * that initiated it. This is intentionally dependency-free so the A-to-B
 * navigation race can be tested without mounting another transcript.
 */
export function canAdoptHiveWorkerIntroductionAction(
  current: HiveWorkerIntroductionActionBinding,
  expected: HiveWorkerIntroductionActionBinding,
  result: HiveWorkerIntroductionActionResultBinding,
): boolean {
  return (
    current.generation === expected.generation &&
    current.workerId === expected.workerId &&
    current.sessionId === expected.sessionId &&
    result.workerId === expected.workerId &&
    result.sessionId === expected.sessionId
  );
}
