/**
 * Regression coverage for live assistant subturn boundaries and duplicate
 * provider tool-start events.
 */

import { createStreamCallbacks } from '../src/session/streaming.ts';
import { createStreamingAssistantMessage } from '../src/session/transient.ts';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${message}\nexpected: ${JSON.stringify(expected)}\nactual: ${JSON.stringify(actual)}`,
    );
  }
}

function testHarness() {
  const ref = { current: createStreamingAssistantMessage() };
  let state: any = {
    messages: [ref.current],
    queuedMessages: [],
    isLoading: true,
    isStreaming: true,
    isThinking: false,
    thinkingContent: '',
  };
  const set = (partial: any) => {
    const update = typeof partial === 'function' ? partial(state) : partial;
    state = { ...state, ...update };
  };
  const callbacks = createStreamCallbacks(ref, set, () => state, {
    planStore: { getState: () => ({ setItems() {} }) } as never,
    sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
    persistSessionMode: async () => {},
  });

  return { callbacks, ref, state: () => state };
}

Deno.test('duplicate tool starts remain one live tool block', () => {
  const { callbacks, ref } = testHarness();

  callbacks.onToolCallStart('tool-1', 'bash');
  callbacks.onToolCallStart('tool-1', 'bash');
  callbacks.onToolCallComplete('tool-1', 'bash', { command: 'pwd' });
  callbacks.onToolResult('tool-1', 'done', false);

  assertEquals(ref.current.toolCalls?.length, 1, 'tool call should be idempotent');
  assertEquals(
    ref.current.renderParts?.filter((part) => part.type === 'tool').length,
    1,
    'tool render part should be idempotent',
  );
  assertEquals(ref.current.toolCalls?.[0]?.status, 'success', 'result updates the tool');
});

Deno.test('tool-loop completion starts a distinct live assistant subturn', () => {
  const { callbacks, ref, state } = testHarness();
  const firstId = ref.current.id;

  callbacks.onTextDelta('Before tool');
  callbacks.onTurnComplete?.(1, true);

  assert(ref.current.id !== firstId, 'a continuing model loop should rotate the live id');
  assertEquals(state().messages.length, 1, 'do not append an empty next block');
  assertEquals(state().messages[0].content, 'Before tool', 'first subturn is preserved');
  assertEquals(state().messages[0].kind, undefined, 'first subturn is finalized');

  callbacks.onTextDelta('After tool');
  callbacks.onTurnComplete?.(2, false);

  assertEquals(state().messages.length, 2, 'next provider pass gets its own block');
  assertEquals(
    state().messages.map((message: any) => message.content),
    ['Before tool', 'After tool'],
    'live subturn order matches stored replay order',
  );
  assertEquals(state().messages[1].kind, 'streaming', 'last subturn remains live until finish');
});

Deno.test('live steering stays between distinct assistant subturns', () => {
  const { callbacks, state } = testHarness();

  callbacks.onTextDelta('Initial answer');
  callbacks.onTurnComplete?.(1, true);
  callbacks.onSteeringInjected?.('steer-1', 'Change direction');
  callbacks.onTextDelta('Revised answer');

  assertEquals(
    state().messages.map((message: any) => [message.role, message.content]),
    [
      ['assistant', 'Initial answer'],
      ['user', 'Change direction'],
      ['assistant', 'Revised answer'],
    ],
    'assistant and steering segments must retain chronological visual order',
  );
});

Deno.test('steering before first assistant output leaves no empty assistant block', () => {
  const { callbacks, state } = testHarness();

  callbacks.onSteeringInjected?.('steer-early', 'Add this detail');
  callbacks.onTextDelta('Combined answer');

  assertEquals(
    state().messages.map((message: any) => [message.role, message.content]),
    [
      ['user', 'Add this detail'],
      ['assistant', 'Combined answer'],
    ],
    'top-of-loop steering must prune the unused optimistic assistant block',
  );
});
