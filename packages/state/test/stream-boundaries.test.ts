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

Deno.test('staged Worker input stays queued until its exact successor is assigned', () => {
  const { callbacks, state } = testHarness();
  state().messages.splice(0, 0, {
    id: 'user-current',
    role: 'user',
    content: 'follow up',
    workerStagedInputId: 'input-1',
  });

  callbacks.onWorkerInputStaged?.({
    type: 'worker_input_staged',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    active_run_id: 'run-1',
    staged_input_id: 'input-1',
    successor_run_id: null,
  });
  let message = state().messages.find((candidate: any) => candidate.id === 'user-current');
  assert(message?.isQueued, 'staged input must not look live or completed');
  assertEquals(message?.workerStagedInputId, 'input-1', 'durable input identity is retained');

  callbacks.onWorkerInputStaged?.({
    type: 'worker_input_staged',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    active_run_id: 'run-1',
    staged_input_id: 'input-1',
    successor_run_id: 'run-2',
  });
  message = state().messages.find((candidate: any) => candidate.id === 'user-current');
  assert(!message?.isQueued, 'only durable successor assignment clears the queued state');
  assertEquals(message?.successorRunId, 'run-2', 'successor identity remains exact');
});

Deno.test('a staged Worker event never binds an unrelated trailing user row', () => {
  const { callbacks, state } = testHarness();
  state().messages.splice(0, 0, {
    id: 'user-newer',
    role: 'user',
    content: 'newer queued input',
    isQueued: true,
  });

  callbacks.onWorkerInputStaged?.({
    type: 'worker_input_staged',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    active_run_id: 'run-1',
    staged_input_id: 'older-input',
    successor_run_id: 'run-2',
  });

  const newer = state().messages.find((message: any) => message.id === 'user-newer');
  assertEquals(newer?.workerStagedInputId, undefined, 'identity is never guessed by row order');
  assert(newer?.isQueued, 'an unrelated queued row stays queued');
});

Deno.test('a mismatched finish cannot move the selected session or claim its queue', () => {
  const ref = { current: createStreamingAssistantMessage() };
  let state: any = {
    sessionId: 'session-a',
    messages: [ref.current],
    queuedMessages: [{ id: 'queued-a', content: 'A only', attachments: [] }],
    isLoading: true,
    isStreaming: true,
    isThinking: false,
    thinkingContent: '',
    sendMessage: () => Promise.resolve(),
  };
  const callbacks = createStreamCallbacks(
    ref,
    (partial: any) => {
      const update = typeof partial === 'function' ? partial(state) : partial;
      state = { ...state, ...update };
    },
    () => state,
    {
      planStore: { getState: () => ({ setItems() {} }) } as never,
      sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
      persistSessionMode: async () => {},
      expectedSessionId: 'session-a',
    },
  );

  callbacks.onFinish('session-b', 'completed');
  assertEquals(state.sessionId, 'session-a', 'selection remains on the request owner');
  assertEquals(state.queuedMessages.length, 1, 'A queue remains unclaimed');
});

Deno.test('uncommitted Worker response prose is discarded on error', () => {
  const { callbacks, state } = testHarness();
  callbacks.onWorkerResponsePending?.({
    type: 'worker_response_pending',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    run_id: 'run-1',
  });
  callbacks.onTextDelta('this was never committed');
  callbacks.onError('canonical response commit was rejected');

  assertEquals(
    state().messages.filter((message: any) => message.role === 'assistant'),
    [],
    'provider prose must not survive without the exact commit boundary',
  );
});

Deno.test('only an exact committed completed Worker response becomes durable-looking', () => {
  const { callbacks, state } = testHarness();
  callbacks.onWorkerResponsePending?.({
    type: 'worker_response_pending',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    run_id: 'run-1',
  });
  callbacks.onTextDelta('canonical response');
  callbacks.onWorkerResponseCommitted?.({
    type: 'worker_response_committed',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    run_id: 'run-1',
  });
  callbacks.onFinish('worker-dm', 'completed');

  const assistant = state().messages.find(
    (message: any) => message.role === 'assistant',
  );
  assertEquals(assistant?.content, 'canonical response', 'committed text remains visible');
  assertEquals(assistant?.kind, undefined, 'completed response is finalized');
});

Deno.test('mismatched Worker commit cannot authenticate the visible draft', () => {
  const { callbacks, state } = testHarness();
  callbacks.onWorkerResponsePending?.({
    type: 'worker_response_pending',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    run_id: 'run-1',
  });
  callbacks.onTextDelta('wrong run');
  callbacks.onWorkerResponseCommitted?.({
    type: 'worker_response_committed',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    run_id: 'run-replacement',
  });
  callbacks.onFinish('worker-dm', 'completed');

  assertEquals(
    state().messages.filter((message: any) => message.role === 'assistant'),
    [],
    'only the exact run boundary may finalize streamed Worker prose',
  );
});

Deno.test('a replayed Worker commit without its pending boundary discards the draft', () => {
  const { callbacks, state } = testHarness();
  callbacks.onTextDelta('streamed before reconnect');
  callbacks.onWorkerResponseCommitted?.({
    type: 'worker_response_committed',
    worker_id: 'worker-1',
    session_id: 'worker-dm',
    run_id: 'run-1',
  });
  callbacks.onFinish('worker-dm', 'completed');

  assertEquals(
    state().messages.filter((message: any) => message.role === 'assistant'),
    [],
    'missing exact pending identity must fail closed and reload canonically',
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

Deno.test('queued follow-up starts on its exact session with its send options', async () => {
  const ref = { current: createStreamingAssistantMessage() };
  const sends: unknown[][] = [];
  let state: any = {
    sessionId: 'worker-a-dm',
    messages: [
      ref.current,
      {
        id: 'user-queued-local-a',
        role: 'user',
        content: 'queued Worker follow-up',
        isQueued: true,
      },
    ],
    queuedMessages: [{
      id: 'user-queued-local-a',
      content: 'queued Worker follow-up',
      attachments: [],
      sendOptions: { sessionType: 'hive' },
    }],
    isLoading: true,
    isStreaming: true,
    isThinking: false,
    thinkingContent: '',
    sendMessage: (...args: unknown[]) => {
      sends.push(args);
      return Promise.resolve();
    },
  };
  const callbacks = createStreamCallbacks(
    ref,
    (partial: any) => {
      const update = typeof partial === 'function' ? partial(state) : partial;
      state = { ...state, ...update };
    },
    () => state,
    {
      planStore: { getState: () => ({ setItems() {} }) } as never,
      sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
      persistSessionMode: async () => {},
    },
  );

  callbacks.onFinish('worker-a-dm', 'completed');
  await Promise.resolve();

  const successorOptions = sends[0]?.[2] as any;
  assertEquals(sends[0]?.[0], 'queued Worker follow-up', 'content stays exact');
  assertEquals(sends[0]?.[1], [], 'attachments stay exact');
  assertEquals(
    successorOptions?.sessionType,
    'hive',
    'the queued turn retains the exact Worker send contract',
  );
  assertEquals(
    successorOptions?.queuedSuccessor?.sessionId,
    'worker-a-dm',
    'the successor claim stays bound to its exact session',
  );
  assertEquals(
    state.queuedMessages.length,
    1,
    'the callback leaves the queue intact until the store persists its claim',
  );
  assert(
    state.messages.some((message: any) =>
      message.id === 'user-queued-local-a' && message.isQueued
    ),
    'the claimed row remains visibly queued until the real send is accepted',
  );
});

Deno.test('pinched queued follow-up preserves A as durable source and B as target', async () => {
  const ref = { current: createStreamingAssistantMessage() };
  const sends: unknown[][] = [];
  let state: any = {
    sessionId: 'session-a',
    messages: [ref.current, {
      id: 'queued-a',
      role: 'user',
      content: 'follow pinch',
      isQueued: true,
    }],
    queuedMessages: [{
      id: 'queued-a',
      content: 'follow pinch',
      attachments: [],
      sendOptions: { sessionType: 'hive', hiveConversationKind: 'worker_dm' },
    }],
    isLoading: true,
    isStreaming: true,
    isThinking: false,
    thinkingContent: '',
    sendMessage: (...args: unknown[]) => {
      sends.push(args);
      return Promise.resolve();
    },
  };
  const callbacks = createStreamCallbacks(
    ref,
    (partial: any) => {
      const update = typeof partial === 'function' ? partial(state) : partial;
      state = { ...state, ...update };
    },
    () => state,
    {
      planStore: { getState: () => ({ setItems() {} }) } as never,
      sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
      persistSessionMode: async () => {},
      expectedSessionId: 'session-a',
      isSessionCurrent: (sessionId) => sessionId === state.sessionId,
      onSessionOwnershipChange: (sessionId) => {
        state.sessionId = sessionId;
      },
    },
  );
  callbacks.onSessionPinched?.({
    type: 'session_pinched',
    reason: 'overflow',
    source_session_id: 'session-a',
    new_session_id: 'session-b',
    estimated_tokens_before: 200000,
  });
  callbacks.onFinish('session-b', 'completed');
  await Promise.resolve();
  const claim = (sends[0]?.[2] as any)?.queuedSuccessor;
  assertEquals(claim?.sessionId, 'session-b', 'B is the validated target');
  assertEquals(claim?.sourceSessionId, 'session-a', 'A remains durable source');
});

Deno.test('a synchronously detached queued follow-up remains visibly unsent', async () => {
  const ref = { current: createStreamingAssistantMessage() };
  let active = true;
  let sends = 0;
  let state: any = {
    sessionId: 'worker-a-dm',
    messages: [
      ref.current,
      {
        id: 'user-queued-local-a',
        role: 'user',
        content: 'do not false-commit me',
        isQueued: true,
      },
    ],
    queuedMessages: [{
      id: 'user-queued-local-a',
      content: 'do not false-commit me',
      attachments: [],
      sendOptions: { sessionType: 'hive' },
    }],
    isLoading: true,
    isStreaming: true,
    isThinking: false,
    thinkingContent: '',
    sendMessage: () => {
      sends += 1;
      return Promise.resolve();
    },
  };
  const callbacks = createStreamCallbacks(
    ref,
    (partial: any) => {
      const update = typeof partial === 'function' ? partial(state) : partial;
      state = { ...state, ...update };
      if (update.isStreaming === false) active = false;
    },
    () => state,
    {
      planStore: { getState: () => ({ setItems() {} }) } as never,
      sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
      persistSessionMode: async () => {},
      isActive: () => active,
      isSessionCurrent: () => active,
    },
  );

  callbacks.onFinish('worker-a-dm', 'completed');
  active = false;
  await Promise.resolve();

  assertEquals(sends, 0, 'a detached continuation cannot send into another session');
  assertEquals(
    state.queuedMessages.length,
    1,
    'the unsent payload remains recoverable instead of being discarded',
  );
  const queued = state.messages.find(
    (message: any) => message.id === 'user-queued-local-a',
  );
  assert(queued?.isQueued, 'the unsent row must not look durably sent');
});

Deno.test('queued lag reconciliation is carried across the successor turn', async () => {
  const deferredReloads = new Set<string>();
  const first = testHarness();
  first.state().sessionId = 'session-a';
  first.state().queuedMessages = [{
    id: 'user-queued-successor',
    content: 'successor',
    attachments: [],
  }];
  first.state().sendMessage = () => Promise.resolve();
  const firstCallbacks = createStreamCallbacks(
    first.ref,
    (partial: any) => {
      const current = first.state();
      const update = typeof partial === 'function' ? partial(current) : partial;
      Object.assign(current, update);
    },
    first.state,
    {
      planStore: { getState: () => ({ setItems() {} }) } as never,
      sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
      persistSessionMode: async () => {},
      deferCanonicalReload: (sessionId) => deferredReloads.add(sessionId),
      consumeCanonicalReload: (sessionId) => deferredReloads.delete(sessionId),
    },
  );
  firstCallbacks.onLagged?.(1);
  firstCallbacks.onFinish('session-a', 'completed');
  await Promise.resolve();
  assert(
    deferredReloads.has('session-a'),
    'the successor must inherit the skipped canonical reconciliation',
  );

  let reloads = 0;
  const secondRef = { current: createStreamingAssistantMessage() };
  let secondState: any = {
    sessionId: 'session-a',
    messages: [secondRef.current],
    queuedMessages: [],
    isLoading: true,
    isStreaming: true,
    isThinking: false,
    thinkingContent: '',
    loadSession: async (sessionId: string, refresh: boolean) => {
      assertEquals([sessionId, refresh], ['session-a', true], 'reload remains exact');
      reloads += 1;
    },
  };
  const secondCallbacks = createStreamCallbacks(
    secondRef,
    (partial: any) => {
      const update = typeof partial === 'function'
        ? partial(secondState)
        : partial;
      secondState = { ...secondState, ...update };
    },
    () => secondState,
    {
      planStore: { getState: () => ({ setItems() {} }) } as never,
      sessionsStore: { getState: () => ({ loadSessions() {} }) } as never,
      persistSessionMode: async () => {},
      deferCanonicalReload: (sessionId) => deferredReloads.add(sessionId),
      consumeCanonicalReload: (sessionId) => deferredReloads.delete(sessionId),
    },
  );
  secondCallbacks.onFinish('session-a', 'completed');
  await Promise.resolve();
  await Promise.resolve();

  assertEquals(reloads, 1, 'successor completion performs the deferred reload');
  assertEquals(deferredReloads.size, 0, 'the reload obligation is consumed once');
});
