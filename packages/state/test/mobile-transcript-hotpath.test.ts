import type { ChatMessage } from '@mitsuro/api';
import {
  splitTranscriptTurnsCached,
} from '../../../apps/mobile/components/chat/transcriptTurns.ts';
import {
  findTranscriptRowIndex,
  splitTranscriptRows,
  splitTranscriptRowsCached,
} from '../../../apps/mobile/components/chat/transcriptRows.ts';
import {
  upsertTransientAssistantMessage,
} from '../src/session/transient.ts';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function message(
  id: string,
  role: ChatMessage['role'],
  content: string,
  kind?: ChatMessage['kind'],
): ChatMessage {
  return { id, role, content, kind };
}

Deno.test('live transcript deltas reuse the complete finalized turn prefix', () => {
  const prefix = [
    message('u1', 'user', 'first'),
    message('a1', 'assistant', 'settled'),
  ];
  const initialMessages = [
    ...prefix,
    message('u2', 'user', 'second'),
    message('a2', 'assistant', 'a', 'streaming'),
  ];
  const initial = splitTranscriptTurnsCached(initialMessages, true);
  const nextMessages = [
    ...prefix,
    initialMessages[2]!,
    message('a2', 'assistant', 'ab', 'streaming'),
  ];
  const next = splitTranscriptTurnsCached(nextMessages, true, initial.cache);

  assert(
    next.historicalTurns === initial.historicalTurns,
    'the finalized list must retain identity for a live-tail delta',
  );
  assert(
    next.historicalTurns[0] === initial.historicalTurns[0],
    'the finalized row must not be regrouped or recreated',
  );
  assert(next.liveTurn?.messages[1]?.content === 'ab', 'the live tail must update');
});

Deno.test('equal-length finalized replacements invalidate the transcript cache', () => {
  const initialMessages = [
    message('u1', 'user', 'first'),
    message('a1', 'assistant', 'alpha'),
    message('u2', 'user', 'second'),
    message('a2', 'assistant', 'tail', 'streaming'),
  ];
  const initial = splitTranscriptTurnsCached(initialMessages, true);
  const next = splitTranscriptTurnsCached(
    [
      initialMessages[0]!,
      message('a1', 'assistant', 'bravo'),
      initialMessages[2]!,
      initialMessages[3]!,
    ],
    true,
    initial.cache,
  );

  assert(
    next.historicalTurns !== initial.historicalTurns,
    'a replaced prefix object must invalidate even when content length matches',
  );
  assert(
    next.historicalTurns[0]?.renderSignature !==
      initial.historicalTurns[0]?.renderSignature,
    'the row revision must change with complete rendered content',
  );
});

Deno.test('returning to an unchanged transcript reuses exact turn identities', () => {
  const messages = [
    message('u1', 'user', 'first'),
    message('a1', 'assistant', 'answer'),
    message('u2', 'user', 'second'),
    message('a2', 'assistant', 'answer two'),
  ];
  const initial = splitTranscriptTurnsCached(messages, false);
  const restored = splitTranscriptTurnsCached(messages, false, initial.cache);

  assert(
    restored.historicalTurns === initial.historicalTurns,
    'unchanged history should keep the exact FlatList data identity',
  );
  assert(
    restored.liveTurn === initial.liveTurn,
    'unchanged latest turn should keep its render signature and identity',
  );
  assert(restored.cache === initial.cache, 'exact reuse should not allocate a replacement cache');
});

Deno.test('a new user boundary rebuilds turn grouping and retention', () => {
  const initialMessages = [
    message('u1', 'user', 'first'),
    message('a1', 'assistant', 'answer'),
  ];
  const initial = splitTranscriptTurnsCached(initialMessages, false);
  const next = splitTranscriptTurnsCached(
    [...initialMessages, message('u2', 'user', 'next')],
    false,
    initial.cache,
  );

  assert(next.turns.length === 2, 'a new user starts a distinct turn');
  assert(next.historicalTurns.length === 1, 'the prior turn becomes historical');
  assert(next.liveTurn?.messages[0]?.id === 'u2', 'the new turn owns the footer');
});

Deno.test('large latest turns are split into bounded message cells', () => {
  const messages = Array.from({ length: 50 }, (_, index) =>
    message(
      `message-${index}`,
      index === 0 ? 'user' : 'assistant',
      `content-${index}`,
    )
  );
  const split = splitTranscriptTurnsCached(messages, false);
  const { rows, liveFooterRow } = splitTranscriptRows(
    split.historicalTurns,
    split.liveTurn,
  );

  assert(rows.length === 49, 'all completed latest-turn messages except the tail are list cells');
  assert(liveFooterRow?.message.id === 'message-49', 'only the changing tail remains in the footer');
  assert(
    rows.every((row) => row.message.id && row.renderSignature === row.message),
    'each virtualized cell renders exactly one stable message object',
  );
  assert(
    findTranscriptRowIndex(rows, 'message-25') === 25,
    'message jump targets address the flattened cell index',
  );
});

Deno.test('live token deltas preserve exact FlatList row data identity', () => {
  const messages = [
    message('u1', 'user', 'first'),
    message('tool1', 'assistant', 'settled tool'),
    message('a1', 'assistant', 'a', 'streaming'),
  ];
  const initialTurns = splitTranscriptTurnsCached(messages, true);
  const initialRows = splitTranscriptRowsCached(
    initialTurns.historicalTurns,
    initialTurns.liveTurn,
  );
  const nextTurns = splitTranscriptTurnsCached(
    [
      messages[0]!,
      messages[1]!,
      message('a1', 'assistant', 'ab', 'streaming'),
    ],
    true,
    initialTurns.cache,
  );
  const nextRows = splitTranscriptRowsCached(
    nextTurns.historicalTurns,
    nextTurns.liveTurn,
    initialRows.cache,
  );

  assert(
    nextRows.rows === initialRows.rows,
    'a token replacement must not invalidate virtualized completed rows',
  );
  assert(
    nextRows.liveFooterRow !== initialRows.liveFooterRow,
    'only the changing footer message receives a new render identity',
  );
});

Deno.test('historical and live rows preserve message order and turn spacing flags', () => {
  const messages = [
    message('u1', 'user', 'first'),
    message('a1', 'assistant', 'answer'),
    message('u2', 'user', 'second'),
    message('a2', 'assistant', 'answer two'),
  ];
  const split = splitTranscriptTurnsCached(messages, false);
  const { rows, liveFooterRow } = splitTranscriptRows(
    split.historicalTurns,
    split.liveTurn,
  );

  assert(
    [...rows.map((row) => row.message.id), liveFooterRow?.message.id].join(',') ===
      'u1,a1,u2,a2',
    'flattening must retain the visible conversation order',
  );
  assert(rows[0]?.isLastMessageInTurn === false, 'the first historical message keeps turn spacing open');
  assert(rows[1]?.isLastMessageInTurn === true, 'the historical tail closes its turn spacing');
  assert(rows[2]?.isLastMessageInTurn === false, 'the live prefix stays visually inside its turn');
  assert(liveFooterRow?.isLastMessageInTurn === true, 'the footer closes the latest turn spacing');
});

Deno.test('transient tail replacement preserves finalized message identities', () => {
  const first = message('u1', 'user', 'first');
  const settled = message('a1', 'assistant', 'settled');
  const live = message('live', 'assistant', 'a', 'streaming');
  const next = upsertTransientAssistantMessage(
    [first, settled, live],
    message('new-live', 'assistant', 'ab', 'streaming'),
  );

  assert(next[0] === first && next[1] === settled, 'finalized entries stay intact');
  assert(next[2]?.id === 'live', 'the stable live identity is retained');
  assert(next[2]?.content === 'ab', 'the live content is replaced');
});

Deno.test('recovery cleanup still removes a non-tail live partial', () => {
  const next = upsertTransientAssistantMessage(
    [
      message('partial', 'assistant', 'old', 'live_partial'),
      message('u1', 'user', 'new request'),
    ],
    message('live', 'assistant', 'new', 'streaming'),
  );

  assert(!next.some((entry) => entry.kind === 'live_partial'), 'stale partial removed');
  assert(next.at(-1)?.content === 'new', 'current live message appended');
});
