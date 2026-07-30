interface RenderBudgetMessage {
  content: string;
  renderParts?: readonly unknown[] | null;
  toolCalls?: readonly unknown[] | null;
}

export interface TranscriptRenderBudget {
  messageCount: number;
  renderPartCount: number;
  toolCount: number;
  markdownCharacterCount: number;
}

export function summarizeTranscriptRenderBudget(
  messages: readonly RenderBudgetMessage[],
): TranscriptRenderBudget {
  let renderPartCount = 0;
  let toolCount = 0;
  let markdownCharacterCount = 0;
  for (const message of messages) {
    renderPartCount += message.renderParts?.length ?? 0;
    toolCount += message.toolCalls?.length ?? 0;
    markdownCharacterCount += message.content.length;
  }
  return {
    messageCount: messages.length,
    renderPartCount,
    toolCount,
    markdownCharacterCount,
  };
}
