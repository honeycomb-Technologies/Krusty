import type { MitsuroClient, ModelKey } from '@mitsuro/api';
import {
  persistCurrentModel,
  persistSessionModel,
} from '../src/session/persistence.ts';
import type { SessionStoreState } from '../src/session/types.ts';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertDeepEquals(actual: unknown, expected: unknown, message: string) {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${message}\nexpected: ${JSON.stringify(expected)}\nactual: ${JSON.stringify(actual)}`,
    );
  }
}

const exactGrokKey: ModelKey = {
  provider: 'grok',
  model_id: 'grok-4.5',
  auth_scope: 'oauth',
  api_format: 'open_ai_responses',
};

Deno.test('session and default persistence forward exact model identity', async () => {
  const updates: Array<{ id: string; data: unknown }> = [];
  const defaults: Array<{ model: string | null; key: ModelKey | null | undefined }> = [];
  let reloads = 0;
  const client = {
    updateSession: async (id: string, data: unknown) => {
      updates.push({ id, data });
      return {};
    },
    setCurrentModel: async (
      model: string | null,
      key?: ModelKey | null,
    ) => {
      defaults.push({ model, key });
      return { ok: true };
    },
  } as unknown as MitsuroClient;
  const sessionsStore = {
    getState: () => ({
      loadSessions: () => {
        reloads += 1;
      },
    }),
  };
  const getState = () => ({
    sessionId: 'session-1',
    sessionType: 'code',
  }) as SessionStoreState;

  await persistSessionModel(
    client,
    sessionsStore as never,
    getState,
    exactGrokKey.model_id,
    exactGrokKey,
  );
  await persistCurrentModel(client, exactGrokKey.model_id, exactGrokKey);

  assertDeepEquals(
    updates,
    [{
      id: 'session-1',
      data: { model: 'grok-4.5', model_key: exactGrokKey },
    }],
    'session persistence must retain the exact key',
  );
  assertDeepEquals(
    defaults,
    [{ model: 'grok-4.5', key: exactGrokKey }],
    'default persistence must retain the exact key',
  );
  assertDeepEquals(reloads, 1, 'session persistence should refresh the list once');
});

Deno.test('Hive session model persistence remains runtime-owned', async () => {
  const updates: Array<{ id: string; data: unknown }> = [];
  const defaults: Array<{ model: string | null; key: ModelKey | null | undefined }> = [];
  let reloads = 0;
  const client = {
    updateSession: async (id: string, data: unknown) => {
      updates.push({ id, data });
      return {};
    },
    setCurrentModel: async (
      model: string | null,
      key?: ModelKey | null,
    ) => {
      defaults.push({ model, key });
      return { ok: true };
    },
  } as unknown as MitsuroClient;
  const sessionsStore = {
    getState: () => ({
      loadSessions: () => {
        reloads += 1;
      },
    }),
  };
  const getState = () => ({
    sessionId: 'worker-dm',
    sessionType: 'hive',
  }) as SessionStoreState;

  await persistSessionModel(
    client,
    sessionsStore as never,
    getState,
    exactGrokKey.model_id,
    exactGrokKey,
  );
  await persistCurrentModel(client, exactGrokKey.model_id, exactGrokKey);

  assertDeepEquals(
    updates,
    [],
    'generic session persistence must not write Hive runtime-owned metadata',
  );
  assertDeepEquals(
    reloads,
    0,
    'skipping a Hive session update must not reload the session list',
  );
  assertDeepEquals(
    defaults,
    [{ model: 'grok-4.5', key: exactGrokKey }],
    'global current-model persistence must remain independent of the Hive fence',
  );
});

Deno.test('legacy model persistence remains slug-only compatible', async () => {
  const updates: unknown[] = [];
  const defaults: unknown[] = [];
  const client = {
    updateSession: async (_id: string, data: unknown) => {
      updates.push(data);
      return {};
    },
    setCurrentModel: async (model: string | null, key?: ModelKey | null) => {
      defaults.push({ model, key });
      return { ok: true };
    },
  } as unknown as MitsuroClient;

  await persistSessionModel(
    client,
    { getState: () => ({ loadSessions: () => {} }) } as never,
    () => ({ sessionId: 'legacy-session' }) as SessionStoreState,
    'legacy-model',
  );
  await persistCurrentModel(client, 'legacy-model');

  assertDeepEquals(
    updates,
    [{ model: 'legacy-model' }],
    'legacy session persistence must not invent an exact key',
  );
  assertDeepEquals(
    defaults,
    [{ model: 'legacy-model' }],
    'legacy default persistence must leave the key undefined',
  );
});
