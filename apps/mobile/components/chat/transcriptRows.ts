import type { ChatMessage } from "@krusty/api";

import type { TranscriptTurn } from "./transcriptTurns";

export interface TranscriptMessageRow {
  id: string;
  turnId: string;
  message: ChatMessage;
  isLive: boolean;
  isLastMessageInTurn: boolean;
  /** Stable identity used by the memoized native row. */
  renderSignature: ChatMessage;
}

export interface SplitTranscriptRows {
  rows: TranscriptMessageRow[];
  liveFooterRow: TranscriptMessageRow | null;
}

export interface TranscriptRowsCache {
  historicalTurns: TranscriptTurn[];
  liveTurnId: string | null;
  liveMessages: ChatMessage[] | null;
  liveIsLive: boolean;
  historicalRows: TranscriptMessageRow[];
  livePrefixRows: TranscriptMessageRow[];
  split: SplitTranscriptRows;
}

export interface CachedSplitTranscriptRows extends SplitTranscriptRows {
  cache: TranscriptRowsCache;
}

/**
 * Flatten visible turns into message-sized native list cells.
 *
 * The newest message remains outside the virtualized list so token deltas only
 * invalidate one footer subtree. Every completed message, including completed
 * messages from the newest turn, is a bounded FlatList cell instead of one
 * monolithic turn/footer mount.
 */
export function splitTranscriptRows(
  historicalTurns: TranscriptTurn[],
  liveTurn: TranscriptTurn | null,
): SplitTranscriptRows {
  const { rows, liveFooterRow } = splitTranscriptRowsCached(
    historicalTurns,
    liveTurn,
  );
  return { rows, liveFooterRow };
}

/**
 * Preserve the exact FlatList data identity across live-token replacements.
 * Only a new completed message changes the virtualized row array.
 */
export function splitTranscriptRowsCached(
  historicalTurns: TranscriptTurn[],
  liveTurn: TranscriptTurn | null,
  previous?: TranscriptRowsCache | null,
): CachedSplitTranscriptRows {
  const liveMessages = liveTurn?.messages ?? null;
  const liveTurnId = liveTurn?.id ?? null;
  const liveIsLive = liveTurn?.isLive ?? false;
  if (
    previous
    && previous.historicalTurns === historicalTurns
    && previous.liveTurnId === liveTurnId
    && previous.liveMessages === liveMessages
    && previous.liveIsLive === liveIsLive
  ) {
    return { ...previous.split, cache: previous };
  }

  const historicalRows = previous?.historicalTurns === historicalTurns
    ? previous.historicalRows
    : historicalTurns.flatMap(buildTurnRows);
  const livePrefixRows = buildLivePrefixRows(liveTurn, previous);
  const rows = previous
      && historicalRows === previous.historicalRows
      && livePrefixRows === previous.livePrefixRows
    ? previous.split.rows
    : [...historicalRows, ...livePrefixRows];
  const liveFooterRow = buildLiveFooterRow(liveTurn, previous);
  const split = { rows, liveFooterRow };
  return {
    ...split,
    cache: {
      historicalTurns,
      liveTurnId,
      liveMessages,
      liveIsLive,
      historicalRows,
      livePrefixRows,
      split,
    },
  };
}

export function findTranscriptRowIndex(
  rows: TranscriptMessageRow[],
  messageId: string,
): number {
  return rows.findIndex((row) => row.message.id === messageId);
}

function buildTurnRows(turn: TranscriptTurn): TranscriptMessageRow[] {
  return turn.messages.map((message, index) =>
    buildMessageRow(
      turn,
      message,
      index,
      index === turn.messages.length - 1,
    )
  );
}

function buildLivePrefixRows(
  liveTurn: TranscriptTurn | null,
  previous?: TranscriptRowsCache | null,
): TranscriptMessageRow[] {
  if (!liveTurn || liveTurn.messages.length <= 1) return [];
  const prefixLength = liveTurn.messages.length - 1;
  const previousPrefix = previous?.liveTurnId === liveTurn.id
    ? previous.livePrefixRows
    : [];
  const next = Array.from({ length: prefixLength }, (_, index) => {
    const message = liveTurn.messages[index]!;
    const cached = previousPrefix[index];
    return cached?.message === message
      ? cached
      : buildMessageRow(liveTurn, message, index, false);
  });
  return next.length === previousPrefix.length
      && next.every((row, index) => row === previousPrefix[index])
    ? previousPrefix
    : next;
}

function buildLiveFooterRow(
  liveTurn: TranscriptTurn | null,
  previous?: TranscriptRowsCache | null,
): TranscriptMessageRow | null {
  if (!liveTurn || liveTurn.messages.length === 0) return null;
  const index = liveTurn.messages.length - 1;
  const message = liveTurn.messages[index]!;
  const cached = previous?.liveTurnId === liveTurn.id
    ? previous.split.liveFooterRow
    : null;
  if (
    cached?.message === message
    && cached.isLive === liveTurn.isLive
    && cached.isLastMessageInTurn
  ) {
    return cached;
  }
  return buildMessageRow(liveTurn, message, index, true);
}

function buildMessageRow(
  turn: TranscriptTurn,
  message: ChatMessage,
  index: number,
  isLastMessageInTurn: boolean,
): TranscriptMessageRow {
  return {
    id: `${turn.id}:message:${message.id}:${index}`,
    turnId: turn.id,
    message,
    isLive: turn.isLive && isLastMessageInTurn,
    isLastMessageInTurn,
    renderSignature: message,
  };
}
