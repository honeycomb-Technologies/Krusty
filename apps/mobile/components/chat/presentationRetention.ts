import type { ChatMessage, ToolCall } from "@krusty/api";

/** Keep newest N turns fully rich; older turns are presentation-light. */
export const RICH_RECENT_TURN_COUNT = 3;

/** Cap retained tool output in historical (non-rich) turns. */
export const HISTORICAL_TOOL_OUTPUT_PREVIEW = 800;

export function isTurnInRichWindow(
  turnIndex: number,
  turnCount: number,
  richRecentCount = RICH_RECENT_TURN_COUNT,
): boolean {
  return turnIndex >= Math.max(0, turnCount - richRecentCount);
}

export function compactHistoricalToolCall(toolCall: ToolCall): ToolCall {
  const output = toolCall.output;
  if (!output || output.length <= HISTORICAL_TOOL_OUTPUT_PREVIEW) {
    return toolCall;
  }
  return {
    ...toolCall,
    output: `${output.slice(0, HISTORICAL_TOOL_OUTPUT_PREVIEW)}
…[older tool output collapsed]`,
    delegated: toolCall.delegated
      ? {
          ...toolCall.delegated,
          thinking: toolCall.delegated.thinking
            ? toolCall.delegated.thinking.slice(0, HISTORICAL_TOOL_OUTPUT_PREVIEW)
            : toolCall.delegated.thinking,
          agents: toolCall.delegated.agents.slice(0, 4),
          filesExamined: toolCall.delegated.filesExamined.slice(0, 8),
          errors: toolCall.delegated.errors.slice(0, 4),
        }
      : undefined,
  };
}

export function compactHistoricalMessage(message: ChatMessage): ChatMessage {
  if (message.role !== "assistant") {
    return {
      ...message,
      // Historical attachments should not keep base64 blobs hot in render path.
      attachments: message.attachments?.map((attachment) => ({
        type: attachment.type,
        name: attachment.name,
        mimeType: attachment.mimeType,
        uri: attachment.uri,
      })),
    };
  }

  return {
    ...message,
    thinking:
      message.thinking && message.thinking.length > HISTORICAL_TOOL_OUTPUT_PREVIEW
        ? `${message.thinking.slice(0, HISTORICAL_TOOL_OUTPUT_PREVIEW)}
…[older thinking collapsed]`
        : message.thinking,
    toolCalls: message.toolCalls?.map(compactHistoricalToolCall),
    attachments: message.attachments?.map((attachment) => ({
      type: attachment.type,
      name: attachment.name,
      mimeType: attachment.mimeType,
      uri: attachment.uri,
    })),
  };
}
