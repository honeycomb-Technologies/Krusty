import type { ChatMessage, ToolCall } from "@mitsuro/api";

export interface StreamEffectView {
  sessionId: string | null;
  title: string;
  isStreaming: boolean;
  tokenCount: number;
  model: string | null;
  settledAssistantSnippet: string;
  currentTurnToolCalls: ToolCall[];
}

interface StreamEffectSource {
  sessionId: string | null;
  title: string;
  isStreaming: boolean;
  tokenCount: number;
  model: string | null;
  messages: ChatMessage[];
}

function sameToolCallReferences(left: ToolCall[], right: ToolCall[]): boolean {
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

/**
 * Selects only lifecycle/tool transitions needed by notifications and native
 * presentation. Text deltas keep every selected field referentially stable,
 * allowing Zustand's shallow selector to skip the coordinator render.
 */
export function createStreamEffectSelector() {
  let previousToolCalls: ToolCall[] = [];

  return (state: StreamEffectSource): StreamEffectView => {
    const nextToolCalls: ToolCall[] = [];
    let settledAssistantSnippet = "";

    for (let index = state.messages.length - 1; index >= 0; index -= 1) {
      const message = state.messages[index];
      if (!message) continue;
      // Queued steering can sit after an assistant that is still awaiting
      // approval. It is not a durable turn boundary yet, so keep scanning.
      if (message.role === "user" && !message.isQueued) break;
      if (!state.isStreaming && !settledAssistantSnippet) {
        const content = message.content.trim();
        if (content) {
          settledAssistantSnippet =
            content.length > 180 ? `${content.slice(0, 177)}...` : content;
        }
      }
      if (message.toolCalls?.length) {
        nextToolCalls.unshift(...message.toolCalls);
      }
    }

    if (!sameToolCallReferences(previousToolCalls, nextToolCalls)) {
      previousToolCalls = nextToolCalls;
    }

    return {
      sessionId: state.sessionId,
      title: state.title,
      isStreaming: state.isStreaming,
      tokenCount: state.tokenCount,
      model: state.model,
      settledAssistantSnippet,
      currentTurnToolCalls: previousToolCalls,
    };
  };
}
