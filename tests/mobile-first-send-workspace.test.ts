import test from 'node:test';
import assert from 'node:assert/strict';

import type { ChatRequest, StreamCallbacks } from '../packages/api/src/types';
import { createSessionStore } from '../packages/state/src/session/store';
import { MemoryStorage } from '../packages/state/src/storage';
import { createWorkspaceStore } from '../packages/state/src/workspace';
import { resolveFirstSendIntent } from '../apps/mobile/app/(tabs)/chat-screen/sendIntent';

function createStoreHarness() {
  const streamRequests: ChatRequest[] = [];
  const client = {
    async streamChat(
      request: ChatRequest,
      callbacks: StreamCallbacks,
      _signal?: AbortSignal,
    ) {
      streamRequests.push(request);
      callbacks.onFinish('stream-created-session');
    },
    async setCurrentModel() {
      return { ok: true };
    },
    async updateSession() {
      throw new Error('updateSession should not be called by sendMessage tests');
    },
    async getSessionState() {
      throw new Error('getSessionState should not be called without an existing session');
    },
    async removeSessionPresence() {
      return undefined;
    },
  };
  const storage = new MemoryStorage();
  const workspace = createWorkspaceStore(storage);
  const sessionsStore = {
    getState: () => ({
      loadSessions: async () => undefined,
    }),
  };
  const planStore = {
    getState: () => ({
      setVisible: () => undefined,
    }),
  };

  const sessionStore = createSessionStore(
    client as never,
    storage,
    workspace,
    sessionsStore as never,
    planStore as never,
  );

  return { sessionStore, streamRequests, workspace };
}

test('code first-send with a selected workspace streams a new code session with workspace context', async () => {
  const { sessionStore, streamRequests, workspace } = createStoreHarness();
  workspace.getState().setWorkspace('/repo/project', null, 'selected');

  const intent = resolveFirstSendIntent({
    currentSessionId: null,
    sessionType: 'code',
    workspace: workspace.getState(),
  });

  assert.equal(intent.shouldCreateSessionBeforeSend, false);
  await sessionStore
    .getState()
    .sendMessage('inspect this repository', [], false, intent.sendOptions);

  assert.equal(streamRequests.length, 1);
  assert.deepEqual(streamRequests[0], {
    session_id: undefined,
    message: 'inspect this repository',
    content: undefined,
    project_dir: '/repo/project',
    working_dir: '/repo/project',
    workspace_mode: 'selected',
    session_type: 'code',
    research_enabled: undefined,
    model: undefined,
    fast_mode: undefined,
    thinking_enabled: 'medium',
    permission_mode: 'supervised',
    mode: 'build',
  });
});

test('code first-send without a selected workspace keeps explicit neutral precreate behavior', () => {
  const { workspace } = createStoreHarness();

  const intent = resolveFirstSendIntent({
    currentSessionId: null,
    sessionType: 'code',
    workspace: workspace.getState(),
  });

  assert.equal(intent.shouldCreateSessionBeforeSend, true);
  assert.deepEqual(intent.sendOptions, { sessionType: 'code' });
});

test('chat first-send does not inherit a stale selected code workspace implicitly', () => {
  const { workspace } = createStoreHarness();
  workspace.getState().setWorkspace('/repo/previous-code-session', 'old-code-session', 'selected');

  const intent = resolveFirstSendIntent({
    currentSessionId: null,
    sessionType: 'chat',
    workspace: workspace.getState(),
  });

  assert.equal(intent.shouldCreateSessionBeforeSend, true);
  assert.deepEqual(intent.sendOptions, { sessionType: 'chat' });
});
