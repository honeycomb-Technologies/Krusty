import type { ChatMessage } from "@krusty/api";
import { assistantMessageRevision } from "./assistantSegments";
import {
  compactHistoricalMessage,
  isTurnInRichWindow,
} from "./presentationRetention";

export interface TranscriptTurn {
  id: string;
  messages: ChatMessage[];
  isLive: boolean;
  renderSignature: string;
}

export interface SplitTranscriptTurns {
  /** Completed/previous turns rendered as stable FlatList rows. */
  historicalTurns: TranscriptTurn[];
  /** Active/latest turn isolated from historical row updates. */
  liveTurn: TranscriptTurn | null;
  /** Full turn list (historical + live) for callers that need both. */
  turns: TranscriptTurn[];
}

export function buildTranscriptTurns(
  messages: ChatMessage[],
  isStreaming: boolean,
): TranscriptTurn[] {
  return splitTranscriptTurns(messages, isStreaming).turns;
}

/**
 * Split the transcript so streaming updates only invalidate the live turn.
 * Historical turns keep stable identities/signatures while the tail grows.
 */
export function splitTranscriptTurns(
  messages: ChatMessage[],
  isStreaming: boolean,
): SplitTranscriptTurns {
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
  const turnCount = groupedMessages.length;
  const turns = groupedMessages.map((turnMessages, index) => {
    const firstMessage = turnMessages[0];
    const id = firstMessage ? `turn-${firstMessage.id}` : `turn-${index}`;
    // Only the isolated tail row is live; historical rows never flip live state
    // during stream ticks.
    const isLive = isStreaming && index === lastIndex;
    // Litter-style retention: only recent turns keep full tool/thinking detail.
    const rich = isTurnInRichWindow(index, turnCount) || isLive;
    const displayMessages = rich
      ? turnMessages
      : turnMessages.map(compactHistoricalMessage);

    return {
      id,
      messages: displayMessages,
      isLive,
      renderSignature: [
        id,
        isLive ? "live" : "steady",
        rich ? "rich" : "compact",
        ...displayMessages.map(messageRenderSignature),
      ].join("||"),
    };
  });

  if (turns.length === 0) {
    return {
      historicalTurns: [],
      liveTurn: null,
      turns,
    };
  }

  // Always isolate the latest turn from FlatList row recycling so stream
  // deltas do not rebuild historical cells.
  return {
    historicalTurns: turns.slice(0, -1),
    liveTurn: turns[turns.length - 1] ?? null,
    turns,
  };
}

export function findTurnIndexForMessage(
  turns: TranscriptTurn[],
  messageId: string,
): number {
  return turns.findIndex((turn) =>
    turn.messages.some((message) => message.id === messageId),
  );
}

export function turnContainsMessage(
  turn: TranscriptTurn | null | undefined,
  messageId: string,
): boolean {
  if (!turn) return false;
  return turn.messages.some((message) => message.id === messageId);
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
