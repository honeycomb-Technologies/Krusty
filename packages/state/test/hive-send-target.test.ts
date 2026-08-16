import { resolveHiveSendTarget } from '../src/session/hiveSendTarget.ts';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals(actual: unknown, expected: unknown, message: string) {
  if (!Object.is(actual, expected)) {
    throw new Error(`${message}\nexpected: ${String(expected)}\nactual: ${String(actual)}`);
  }
}

Deno.test('hive send targets the already-loaded hive session (companion or Worker DM)', () => {
  const companion = resolveHiveSendTarget({
    sessionId: 'companion-1',
    sessionType: 'hive',
  });
  assertEquals(companion.kind, 'loaded-session', 'a loaded companion is the send target');
  assertEquals(
    companion.kind === 'loaded-session' ? companion.sessionId : null,
    'companion-1',
    'the loaded session id is preserved',
  );

  const workerDm = resolveHiveSendTarget({
    sessionId: 'worker-dm-9',
    sessionType: 'hive',
  });
  assertEquals(
    workerDm.kind === 'loaded-session' ? workerDm.sessionId : null,
    'worker-dm-9',
    'a loaded Worker DM must receive the send instead of the companion',
  );
});

Deno.test('hive send ensures the companion only when nothing usable is loaded', () => {
  assertEquals(
    resolveHiveSendTarget({ sessionId: null, sessionType: null }).kind,
    'ensure-companion',
    'an empty hive store falls back to the durable companion',
  );
  assertEquals(
    resolveHiveSendTarget({ sessionId: '   ', sessionType: 'hive' }).kind,
    'ensure-companion',
    'a blank session id is not a valid target',
  );
  assertEquals(
    resolveHiveSendTarget({ sessionId: 'code-1', sessionType: 'code' }).kind,
    'ensure-companion',
    'a non-hive session must never become the hive send target',
  );
});
