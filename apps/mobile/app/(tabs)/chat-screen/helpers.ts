import { Dimensions } from "react-native";

import type {
  ChatMessage,
  ModelInfo,
  SessionType,
} from "@krusty/api";
import type { ToolCall } from "@krusty/state";

const SCREEN_HEIGHT = Dimensions.get("window").height;

export const SPLIT_PANEL_HEIGHT = SCREEN_HEIGHT * 0.42;
export const CHAT_BAR_ZONE = 130;
export const SELECTED_MODEL_KEY = "krusty_selected_model";
export const TAB_TYPES: SessionType[] = ["chat", "code", "mako"];

export type WorkspaceMode = "neutral" | "selected" | "created";

export function normalizeProviderId(provider: string | null | undefined): string {
  return (provider ?? "").trim().toLowerCase();
}

export function isModelUsable(
  modelId: string | null | undefined,
  catalog: ModelInfo[],
  configuredProviders: string[],
): boolean {
  if (!modelId) {
    return false;
  }

  const match = catalog.find((candidate) => candidate.id === modelId);
  if (!match) {
    return false;
  }

  if (configuredProviders.length === 0) {
    return true;
  }

  return configuredProviders.includes(normalizeProviderId(match.provider));
}

export function sessionTypeForTab(index: number): SessionType {
  return TAB_TYPES[index] ?? "code";
}

export function tabForSessionType(type: SessionType): number {
  switch (type) {
    case "chat":
      return 0;
    case "mako":
      return 2;
    default:
      return 1;
  }
}

export function getLastAssistantMessage(messages: ChatMessage[]): ChatMessage | null {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message?.role === "assistant") {
      return message;
    }
  }
  return null;
}

export function flattenToolCalls(messages: ChatMessage[]): ToolCall[] {
  const toolCalls: ToolCall[] = [];
  for (const message of messages) {
    if (message.toolCalls?.length) {
      toolCalls.push(...message.toolCalls);
    }
  }
  return toolCalls;
}

export function getActiveToolCall(toolCalls: ToolCall[]): ToolCall | null {
  for (let index = toolCalls.length - 1; index >= 0; index -= 1) {
    const toolCall = toolCalls[index];
    if (
      toolCall &&
      (toolCall.status === "awaiting_approval" ||
        toolCall.status === "running" ||
        toolCall.status === "pending")
    ) {
      return toolCall;
    }
  }
  return null;
}

export function getWorkspaceMode(path: string | null): WorkspaceMode {
  return path ? "selected" : "neutral";
}
