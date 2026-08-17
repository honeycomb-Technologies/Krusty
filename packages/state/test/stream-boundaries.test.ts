/**
 * Regression coverage for live assistant subturn boundaries and duplicate
 * provider tool-start events.
 */

import { createStreamCallbacks } from '../src/session/streaming.ts';
import { mergeDelegationEventCursor } from '../src/session/serverState.ts';
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
  let setCount = 0;
  let state: any = {
    messages: [ref.current],
    queuedMessages: [],
    isLoading: true,
    isStreaming: true,
    isThinking: false,
    thinkingContent: '',
    delegationEventCursor: 40,
  };
  const set = (partial: any) => {
    setCount += 1;
    const update = typeof partial === 'function' ? partial(state) : partial;
    state = { ...state, ...update };
  };
  const callbacks = createStreamCallbacks(ref, set, () => state, {
    planStore: { getState: () => ({ setItems() {} }) } as never,
    sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
    persistSessionMode: async () => {},
  });

  return { callbacks, ref, state: () => state, setCount: () => setCount };
}

Deno.test('delegation SSE events request an immediate canonical refresh', () => {
  const ref = { current: createStreamingAssistantMessage() };
  let refreshes = 0;
  const state: any = { messages: [ref.current], delegationEventCursor: 40 };
  const callbacks = createStreamCallbacks(ref, () => {}, () => state, {
    planStore: { getState: () => ({ setItems() {} }) } as never,
    sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
    persistSessionMode: async () => {},
    onDelegationEvent: () => refreshes += 1,
  });
  const event = {
    event_id: 41,
    parent_session_id: 'session-1',
    delegation_group_id: 'group-1',
    delegation_task_id: 'task-1',
    event_type: 'task_running' as const,
    payload: { state: 'running' },
    created_at: '2026-08-08T12:00:00Z',
  };
  callbacks.onDelegationEvent?.(event);
  assertEquals(refreshes, 1, 'new delegation lifecycle events trigger canonical refresh');
  callbacks.onDelegationEvent?.({ ...event, event_id: 39 });
  assertEquals(refreshes, 1, 'already-applied events do not trigger duplicate refreshes');
});

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

Deno.test('live sparse delegation events preserve the HTTP replay cursor', () => {
  const harness = testHarness();
  const event = {
    event_id: 41,
    parent_session_id: 'session-1',
    delegation_group_id: 'group-1',
    delegation_task_id: 'task-1',
    event_type: 'task_running' as const,
    payload: { state: 'running' },
    created_at: '2026-08-08T12:00:00Z',
  };

  harness.callbacks.onDelegationEvent?.(event);
  assertEquals(
    harness.state().delegationEventCursor,
    40,
    'SSE must not advance the cursor before canonical snapshot state is applied',
  );
  harness.callbacks.onDelegationEvent?.({ ...event, event_id: 43 });
  assertEquals(
    harness.state().delegationEventCursor,
    40,
    'globally sparse event IDs remain replayable for this session',
  );
});

Deno.test('an older in-flight snapshot cannot regress the live delegation cursor', () => {
  assertEquals(
    mergeDelegationEventCursor(42, 41),
    42,
    'cursor merges must be monotonic when SSE wins the race',
  );
  assertEquals(
    mergeDelegationEventCursor(null, 41),
    41,
    'the first durable snapshot establishes the replay cursor',
  );
});

Deno.test('bursty thinking and tool output deltas commit once per frame', async () => {
  const thinking = testHarness();
  const thinkingBaseline = thinking.setCount();
  thinking.callbacks.onThinkingDelta('one ');
  thinking.callbacks.onThinkingDelta('two');
  await new Promise((resolve) => setTimeout(resolve, 25));

  assertEquals(
    thinking.setCount() - thinkingBaseline,
    1,
    'thinking deltas should share one presentation update',
  );
  assertEquals(
    thinking.ref.current.thinking,
    'one two',
    'thinking content should retain every delta',
  );

  const tools = testHarness();
  tools.callbacks.onToolCallStart('tool-stream', 'bash');
  const toolBaseline = tools.setCount();
  tools.callbacks.onToolOutputDelta?.('tool-stream', 'one ');
  tools.callbacks.onToolOutputDelta?.('tool-stream', 'two');
  await new Promise((resolve) => setTimeout(resolve, 25));

  assertEquals(
    tools.setCount() - toolBaseline,
    1,
    'tool output deltas should share one presentation update',
  );
  assertEquals(
    tools.ref.current.toolCalls?.[0]?.output,
    'one two',
    'tool output should retain every delta',
  );
});

Deno.test('queued stream deltas are discarded after the attachment generation changes', () => {
  type FrameCallback = (timestamp: number) => void;
  const runtime = globalThis as typeof globalThis & {
    requestAnimationFrame?: (callback: FrameCallback) => number;
  };
  const originalRequestAnimationFrame = runtime.requestAnimationFrame;
  let queuedFrame: FrameCallback | null = null;
  let active = true;
  Object.defineProperty(globalThis, 'requestAnimationFrame', {
    configurable: true,
    value: (callback: (timestamp: number) => void) => {
      queuedFrame = callback;
      return 1;
    },
  });

  try {
    const ref = { current: createStreamingAssistantMessage() };
    let state: any = {
      messages: [ref.current],
      queuedMessages: [],
      isLoading: true,
      isStreaming: true,
      isThinking: false,
      thinkingContent: '',
    };
    let setCount = 0;
    const callbacks = createStreamCallbacks(
      ref,
      (partial: any) => {
        setCount += 1;
        const update = typeof partial === 'function' ? partial(state) : partial;
        state = { ...state, ...update };
      },
      () => state,
      {
        planStore: { getState: () => ({ setItems() {} }) } as never,
        sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
        persistSessionMode: async () => {},
        isActive: () => active,
      },
    );

    callbacks.onTextDelta('belongs to old session');
    assert(queuedFrame, 'text delta should queue a presentation frame');
    active = false;
    const runQueuedFrame = queuedFrame as FrameCallback;
    runQueuedFrame(0);

    assertEquals(setCount, 0, 'detached frame must not update store state');
    assertEquals(ref.current.content, '', 'detached content must not enter the live message');
  } finally {
    Object.defineProperty(globalThis, 'requestAnimationFrame', {
      configurable: true,
      value: originalRequestAnimationFrame,
    });
  }
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
  callbacks.onTurnComplete?.(2, false);

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
  callbacks.onTurnComplete?.(1, false);

  assertEquals(
    state().messages.map((message: any) => [message.role, message.content]),
    [
      ['user', 'Add this detail'],
      ['assistant', 'Combined answer'],
    ],
    'top-of-loop steering must prune the unused optimistic assistant block',
  );
});

Deno.test('rollover-staged steering remains visibly queued after stream finish', () => {
  const { callbacks, state } = testHarness();
  state().messages.push({
    id: 'user-steering-staged',
    role: 'user',
    content: 'apply on restart',
    isQueued: true,
    queuedUntilNextRun: true,
  });

  callbacks.onFinish('session-1');

  const staged = state().messages.find(
    (message: any) => message.id === 'user-steering-staged',
  );
  assert(staged?.isQueued, 'durable rollover steering must remain visibly queued');
  assert(
    staged?.queuedUntilNextRun,
    'queued-next-run status must survive current stream finalization',
  );
});

Deno.test('delivered but uninjected steering rolls forward visibly at finish', () => {
  const { callbacks, state } = testHarness();
  state().messages.push({
    id: 'user-steering-delivered',
    role: 'user',
    content: 'arrived during terminal rollover',
    isQueued: true,
    queuedUntilNextRun: false,
  });

  callbacks.onFinish('session-1');

  const staged = state().messages.find(
    (message: any) => message.id === 'user-steering-delivered',
  );
  assert(staged?.isQueued, 'uninjected steering must not look completed');
  assert(
    staged?.queuedUntilNextRun,
    'terminal rollover must expose durable next-run recovery status',
  );
});
