import type { ChatMessage } from "@krusty/api";
import { assistantMessageRevision } from "./assistantSegments";

export interface TranscriptTurn {
  id: string;
  messages: ChatMessage[];
  isLive: boolean;
  renderSignature: string;
}

export function buildTranscriptTurns(
  messages: ChatMessage[],
  isStreaming: boolean,
): TranscriptTurn[] {
  const groupedMessages: ChatMessage[][] = [];
  let currentGroup: ChatMessage[] = [];

  for (const message of messages) {
    const startsNewTurn = message.role === "user" && currentGroup.length > 0;
    if (startsNewTurn) {
      groupedMessages.push(currentGroup);
      currentGroup = [message];
      continue;
    }

    currentGroup.push(message);
  }

  if (currentGroup.length > 0) {
    groupedMessages.push(currentGroup);
  }

  const lastIndex = groupedMessages.length - 1;
  return groupedMessages.map((turnMessages, index) => {
    const firstMessage = turnMessages[0];
    const id = firstMessage ? `turn-${firstMessage.id}` : `turn-${index}`;
    const isLive = isStreaming && index === lastIndex;

    return {
      id,
      messages: turnMessages,
      isLive,
      renderSignature: [
        id,
        isLive ? "live" : "steady",
        ...turnMessages.map(messageRenderSignature),
      ].join("||"),
    };
  });
}

export function findTurnIndexForMessage(
  turns: TranscriptTurn[],
  messageId: string,
): number {
  return turns.findIndex((turn) =>
    turn.messages.some((message) => message.id === messageId),
  );
}

function messageRenderSignature(message: ChatMessage): string {
  if (message.role === "assistant") {
    return assistantMessageRevision(message);
  }

  return [
    message.id,
    message.role,
    message.content.length,
    message.isQueued ? "queued" : "steady",
    message.attachments
      ?.map((attachment) =>
        [
          attachment.type,
          attachment.name ?? "",
          attachment.uri?.length ?? 0,
          attachment.base64?.length ?? 0,
        ].join(":"),
      )
      .join("|") ?? "",
  ].join("::");
}
