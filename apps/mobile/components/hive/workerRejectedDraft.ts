/** Preserve both the rejected message and text typed while it was in flight. */
export function mergeRejectedWorkerDraft(
  rejectedContent: string,
  currentDraft: string,
): string {
  if (!currentDraft) return rejectedContent;
  if (currentDraft === rejectedContent) return currentDraft;
  return `${rejectedContent}\n\n${currentDraft}`;
}
