import type { ChatMessage, ChatRenderPart, ToolCall } from "@krusty/api";

/** Keep in sync with plan/mode tools filtered from UI in core tool policy. */
const INTERNAL_TOOL_NAMES = new Set([
  "enter_plan_mode",
  "set_work_mode",
  "set_workspace_context",
  "task_start",
  "task_complete",
  "add_subtask",
  "set_dependency",
]);

const EXPLORATION_TOOL_NAMES = new Set([
  "glob",
  "grep",
  "ls",
  "list",
  "list_files",
  "read",
  "search",
]);

export type AssistantVisualSegment =
  | {
      type: "thinking";
      id: string;
      content: string;
    }
  | {
      type: "text";
      id: string;
      content: string;
    }
  | {
      type: "attachments";
      id: string;
    }
  | {
      type: "tool";
      id: string;
      toolCall: ToolCall;
    }
  | {
      type: "exploration";
      id: string;
      tools: ToolCall[];
    };

export function isDelegatedTool(toolCall: ToolCall): boolean {
  return (
    toolCall.name === "agent" ||
    toolCall.name === "explore" ||
    toolCall.name === "plan" ||
    toolCall.name === "verify" ||
    toolCall.name === "build"
  );
}

export function isQuestionTool(toolCall: ToolCall): boolean {
  return toolCall.name === "AskUserQuestion";
}

export function isPlanConfirmTool(toolCall: ToolCall): boolean {
  return toolCall.name === "PlanConfirm";
}

function isInternalTool(toolCall: ToolCall): boolean {
  return INTERNAL_TOOL_NAMES.has(toolCall.name);
}

function isExplorationTool(toolCall: ToolCall): boolean {
  return EXPLORATION_TOOL_NAMES.has(toolCall.name.toLowerCase());
}

function isSoftInterruption(segment: AssistantVisualSegment): boolean {
  return (
    segment.type === "exploration" ||
    segment.type === "thinking" ||
    segment.type === "tool"
  );
}

export function startsLikeNewBlock(content: string): boolean {
  return /^(#{1,6}\s|[-*+]\s|\d+[.)]\s|```|>|<\w)/.test(content);
}

/** Exported for unit tests — mid-stream tool interruptions rejoin prose. */
export function shouldMergeContinuationText(
  previousContent: string,
  nextContent: string,
  interveningSegments: AssistantVisualSegment[],
): boolean {
  if (interveningSegments.length === 0) return false;
  if (!interveningSegments.every(isSoftInterruption)) return false;

  const previous = previousContent.trimEnd();
  const next = nextContent.trimStart();
  if (!previous || !next || startsLikeNewBlock(next)) return false;

  // Do not glue across closed code fences / list boundaries left unfinished.
  if (/```\s*$/.test(previous) || /^```/.test(next)) return false;

  const previousLooksUnfinished = /[A-Za-z0-9_'")\]]$/.test(previous);
  const nextLooksContinued = /^[a-z,.;:!?'"()\]}]/.test(next);

  return previousLooksUnfinished && nextLooksContinued;
}

export function appendContinuationText(previous: string, next: string): string {
  if (!previous) return next;
  if (!next) return previous;
  if (/\s$/.test(previous) || /^\s/.test(next)) return previous + next;
  return previous + next.trimStart();
}

/** Exported for unit tests. */
export function smoothInterruptedText(
  segments: AssistantVisualSegment[],
): AssistantVisualSegment[] {
  const smoothed: AssistantVisualSegment[] = [];

  for (const segment of segments) {
    if (segment.type !== "text") {
      smoothed.push(segment);
      continue;
    }

    let previousTextIndex = -1;
    for (let index = smoothed.length - 1; index >= 0; index -= 1) {
      if (smoothed[index]?.type === "text") {
        previousTextIndex = index;
        break;
      }
    }
    const previousText = smoothed[previousTextIndex];
    const interveningSegments =
      previousTextIndex >= 0 ? smoothed.slice(previousTextIndex + 1) : [];

    if (
      previousText?.type === "text" &&
      shouldMergeContinuationText(
        previousText.content,
        segment.content,
        interveningSegments,
      )
    ) {
      smoothed[previousTextIndex] = {
        ...previousText,
        content: appendContinuationText(previousText.content, segment.content),
      };
      continue;
    }

    smoothed.push(segment);
  }

  return smoothed;
}

function legacyRenderParts(
  message: ChatMessage,
  isLast: boolean,
  isThinking?: boolean,
): ChatRenderPart[] {
  const parts: ChatRenderPart[] = [];

  if (message.thinking || (isLast && isThinking)) {
    parts.push({
      type: "thinking",
      id: "legacy-thinking",
      content: message.thinking ?? "",
    });
  }

  for (const toolCall of message.toolCalls ?? []) {
    parts.push({
      type: "tool",
      id: `legacy-tool-${toolCall.id}`,
      toolCallId: toolCall.id,
    });
  }

  if ((message.attachments?.length ?? 0) > 0) {
    parts.push({ type: "attachments", id: "legacy-attachments" });
  }

  if (message.content.length > 0) {
    parts.push({
      type: "text",
      id: "legacy-text",
      content: message.content,
    });
  }

  return parts;
}

export function assistantVisualSegments(
  message: ChatMessage,
  isLast: boolean,
  isThinking?: boolean,
): AssistantVisualSegment[] {
  const toolById = new Map(
    (message.toolCalls ?? []).map((toolCall) => [toolCall.id, toolCall]),
  );
  const renderParts =
    message.renderParts && message.renderParts.length > 0
      ? message.renderParts
      : legacyRenderParts(message, isLast, isThinking);
  const segments: AssistantVisualSegment[] = [];
  let explorationBuffer: ToolCall[] = [];

  const flushExplorationBuffer = () => {
    if (explorationBuffer.length === 0) return;
    const first = explorationBuffer[0];
    segments.push({
      type: "exploration",
      id: `exploration-${first?.id ?? segments.length}`,
      tools: explorationBuffer,
    });
    explorationBuffer = [];
  };

  for (const part of renderParts) {
    switch (part.type) {
      case "thinking": {
        if (!part.content && !(isLast && isThinking)) break;
        flushExplorationBuffer();
        segments.push({
          type: "thinking",
          id: part.id,
          content: part.content,
        });
        break;
      }
      case "tool": {
        const toolCall = toolById.get(part.toolCallId);
        if (!toolCall || isInternalTool(toolCall)) break;
        if (isExplorationTool(toolCall)) {
          explorationBuffer.push(toolCall);
        } else {
          flushExplorationBuffer();
          segments.push({
            type: "tool",
            id: part.id,
            toolCall,
          });
        }
        break;
      }
      case "attachments": {
        if ((message.attachments?.length ?? 0) === 0) break;
        flushExplorationBuffer();
        segments.push({
          type: "attachments",
          id: part.id,
        });
        break;
      }
      case "text": {
        if (part.content.length === 0) break;
        flushExplorationBuffer();
        segments.push({
          type: "text",
          id: part.id,
          content: part.content,
        });
        break;
      }
    }
  }

  flushExplorationBuffer();

  const smoothedSegments = smoothInterruptedText(segments);

  if (
    smoothedSegments.length === 0 &&
    (message.thinking || (isLast && isThinking))
  ) {
    smoothedSegments.push({
      type: "thinking",
      id: "fallback-thinking",
      content: message.thinking ?? "",
    });
  }

  return smoothedSegments;
}
