import type { ChatMessage, ChatRenderPart, ToolCall } from "@krusty/api";
import {
  isExplorationToolName,
  isHiddenToolName,
} from "./toolPresentation";

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
  return INTERNAL_TOOL_NAMES.has(toolCall.name) || isHiddenToolName(toolCall.name);
}

export function isExplorationTool(toolCall: ToolCall): boolean {
  return isExplorationToolName(toolCall.name);
}

function isSoftInterruption(segment: AssistantVisualSegment): boolean {
  // Only tool-like interruptions should rejoin split prose.
  // Thinking is a real phase boundary and must stay sticky.
  return segment.type === "exploration" || segment.type === "tool";
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

  const previousLooksUnfinished =
    /[A-Za-z0-9_/'")\]]$/.test(previous) || /[A-Za-z0-9]\s+$/u.test(previousContent);
  // Lowercase continuation, punctuation, or a short trailing fragment after a tool.
  const nextLooksContinued =
    /^[a-z,.;:!?'"()\]}]/.test(next) ||
    (/^[A-Za-z][A-Za-z0-9/_-]*[.!?]?$/.test(next) && previousLooksUnfinished);

  return previousLooksUnfinished && nextLooksContinued;
}

export function appendContinuationText(previous: string, next: string): string {
  if (!previous) return next;
  if (!next) return previous;
  if (/\s$/.test(previous) || /^\s/.test(next)) return previous + next;

  const left = previous[previous.length - 1] ?? "";
  const right = next.trimStart()[0] ?? "";
  // "we only" + "skimmed." / "server/mobile" + "edges." need a joining space.
  if (/[A-Za-z0-9_/'")\]]/.test(left) && /[A-Za-z0-9("'[]/.test(right)) {
    return `${previous} ${next.trimStart()}`;
  }
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
  // Sticky tool slots: one visual slot per recorded tool/thinking part.
  // Only rejoin prose fragments that were split mid-sentence by tools.
  const segments: AssistantVisualSegment[] = [];

  for (const part of renderParts) {
    switch (part.type) {
      case "thinking": {
        if (!part.content && !(isLast && isThinking)) break;
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
        segments.push({
          type: "tool",
          id: part.id,
          toolCall,
        });
        break;
      }
      case "attachments": {
        if ((message.attachments?.length ?? 0) === 0) break;
        segments.push({
          type: "attachments",
          id: part.id,
        });
        break;
      }
      case "text": {
        if (part.content.length === 0) break;
        segments.push({
          type: "text",
          id: part.id,
          content: part.content,
        });
        break;
      }
    }
  }

  if (
    segments.length === 0 &&
    (message.thinking || (isLast && isThinking))
  ) {
    segments.push({
      type: "thinking",
      id: "fallback-thinking",
      content: message.thinking ?? "",
    });
  }

  // Rejoin "we only" + tool + "skimmed." into one prose block, keeping the tool
  // after the completed sentence instead of splitting words across it.
  return smoothInterruptedText(segments);
}
