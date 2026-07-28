export interface ComposerAttachmentLike {
  uri: string;
}

export function resolveComposerSendPayload<T extends ComposerAttachmentLike>(
  text: string,
  attachments: T[],
): { content: string; attachments?: T[] } | null {
  const content = text.trim();
  if (!content && attachments.length === 0) return null;
  return {
    content,
    attachments: attachments.length > 0 ? attachments : undefined,
  };
}
