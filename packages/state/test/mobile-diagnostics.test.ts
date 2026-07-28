import {
  buildDiagnosticUploadBatch,
  MobileDiagnosticRecorder,
  sanitizeDiagnosticFields,
} from '../src/diagnostics/index.ts';

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test('diagnostics schema cannot retain sensitive arbitrary values', () => {
  const sanitized = sanitizeDiagnosticFields({
    name: 'https://honey.example/api?token=secret',
    surface: '/Users/jacob/private/file.ts',
    state: 'active',
    outcome: 'complete',
    code: 'Bearer secret',
    durationMs: 12.345,
  });

  assert(sanitized.name === 'redacted', 'URLs must be rejected');
  assert(sanitized.surface === 'redacted', 'paths must be rejected');
  assert(sanitized.code === 'redacted', 'free-form credential strings must be rejected');
  assert(sanitized.state === 'active', 'known short labels remain useful');
  assert(sanitized.durationMs === 12.3, 'numeric timing is retained and rounded');
  assert(!('message' in sanitized), 'messages have no representable field');
  const arbitrary = sanitizeDiagnosticFields({ name: 'innocent looking user text' });
  assert(
    arbitrary.name?.startsWith('label_'),
    'unknown strings must be reduced to a stable non-reversible label',
  );
});

Deno.test('baseline recorder enforces event and byte bounds', () => {
  let now = 1_000;
  const recorder = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-12345678',
    now: () => now++,
  });
  for (let index = 0; index < 1_000; index += 1) {
    recorder.record('navigation', { name: `route.${index}` });
  }
  const snapshot = recorder.snapshot();
  assert(snapshot.eventCount <= 256, 'baseline ring must be hard capped');
  assert(snapshot.approximateBytes <= 96 * 1024, 'baseline bytes must be hard capped');
});

Deno.test('stress mode is explicit, time bounded, and returns to baseline', () => {
  let now = 10_000;
  const recorder = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-12345678',
    now: () => now,
  });
  recorder.startStressRun(10_000);
  assert(recorder.getMode() === 'stress', 'stress run starts explicitly');
  now += 10_001;
  assert(recorder.getMode() === 'baseline', 'stress run expires automatically');
  assert(recorder.consumeStressCompletion(), 'automatic expiry requests final upload once');
  assert(!recorder.consumeStressCompletion(), 'stress completion signal is one-shot');
});

Deno.test('upload batches are bounded and acknowledgements retain the remainder', () => {
  const recorder = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-12345678',
  });
  recorder.startStressRun();
  for (let index = 0; index < 300; index += 1) {
    recorder.record('request', { name: 'sessions.list', outcome: 'complete' });
  }
  const before = recorder.snapshot().eventCount;
  const batch = recorder.createBatch();
  assert(batch !== null, 'non-empty recorder creates a batch');
  assert(batch.events.length <= 128, 'upload batch event count is bounded');
  assert(JSON.stringify(batch).length * 2 <= 100 * 1024, 'upload batch bytes are bounded');
  recorder.acknowledge(batch.events.map((event) => event.id));
  assert(recorder.snapshot().eventCount === before - batch.events.length, 'only acknowledged events are removed');
});

Deno.test('completed stress capture persists and drains every bounded batch', () => {
  const recorder = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-stress123',
  });
  recorder.startStressRun();
  for (let index = 0; index < 500; index += 1) {
    recorder.record('request', { name: 'sessions.list', outcome: 'complete' });
  }
  recorder.stopStressRun();
  const pending = recorder.createCompletionPersistenceBatch();
  assert(pending !== null && pending.events.length > 128, 'completion persistence retains the full bounded run');

  const resumed = new MobileDiagnosticRecorder({
    installationId: pending.installationId,
    runId: pending.runId,
    startedAtMs: pending.runStartedAtMs,
  });
  resumed.resumeStressCompletion(pending.createdAtMs);
  assert(resumed.restore(pending) === pending.events.length, 'restart restores the original run identity and all events');

  let drained = 0;
  let batch = resumed.createBatch();
  while (batch) {
    drained += batch.events.length;
    resumed.acknowledge(batch.events.map((event) => event.id));
    batch = resumed.createBatch();
  }
  assert(drained === pending.events.length, 'completion draining cannot drop a tail batch');
  assert(resumed.snapshot().eventCount === 0, 'all completed events are acknowledged');
});

Deno.test('active stress deadline and run identity survive recorder restart', () => {
  let now = 20_000;
  const source = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-active123',
    now: () => now,
  });
  source.startStressRun(10_000);
  source.record('navigation', { name: '(tabs)>index' });
  const pending = source.createCompletionPersistenceBatch();
  const deadline = source.getStressEndsAtMs();
  assert(pending !== null && deadline !== null, 'active stress state is persistable');

  const resumed = new MobileDiagnosticRecorder({
    installationId: pending.installationId,
    runId: pending.runId,
    startedAtMs: pending.runStartedAtMs,
    now: () => now,
  });
  resumed.resumeActiveStress(deadline);
  resumed.restore(pending);
  assert(resumed.getMode() === 'stress', 'capture resumes in stress mode before its deadline');
  assert(resumed.runId === source.runId, 'active capture preserves its run identity');
  now = deadline + 1;
  assert(resumed.getMode() === 'baseline', 'restored capture expires on its original deadline');
  assert(resumed.consumeStressCompletion(), 'restored expiry requests completion upload');
});

Deno.test('pending recovery re-sanitizes fields, deduplicates, and rejects stale batches', () => {
  let now = 1_000_000;
  const source = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-old1234',
    now: () => now,
  });
  source.record('webview', { surface: 'browser', outcome: 'terminate' });
  const pending = source.createPersistenceBatch();
  assert(pending !== null, 'pending batch exists');

  const target = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-new1234',
    now: () => now,
  });
  assert(target.restore(pending) === 1, 'fresh pending event is recovered');
  assert(target.restore(pending) === 0, 'the same pending event is not restored twice');

  const stale = { ...pending, createdAtMs: now - 73 * 60 * 60 * 1000 };
  assert(target.restore(stale) === 0, 'stale pending data is discarded');
  assert(
    target.restore({ ...pending, installationId: 'install-other123' }) === 0,
    'pending data cannot cross installation identities',
  );
});

Deno.test('authenticated upload payload matches the bounded server contract', () => {
  let now = 5_000;
  const recorder = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-12345678',
    now: () => now++,
  });
  recorder.record('longtask', { name: 'js.longtask', durationMs: 125 });
  recorder.record('navigation', { name: '(tabs)>settings' });
  const batch = recorder.createBatch();
  assert(batch !== null, 'batch exists');
  const upload = buildDiagnosticUploadBatch(batch, {
    appVersion: '0.9.20',
    buildNumber: '261',
    platform: 'ios',
    osVersion: '26.6',
    deviceClass: 'mobile',
    captureLevel: 'baseline',
  });

  assert(upload.run.id === 'run-12345678', 'run identity is pseudonymous');
  assert(upload.events[0]?.category === 'runtime', 'long task category is server-compatible');
  assert(upload.events[0]?.name === 'long_task', 'long task report name is canonical');
  assert(upload.events[1]?.attributes.route === '(tabs)>settings', 'route is classified, not a raw URL');
  assert(!JSON.stringify(upload).includes('prompt'), 'payload has no content-bearing keys');
});

Deno.test('MetricKit uploads contain only fixed summaries and stay below the route budget', () => {
  const recorder = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-12345678',
  });
  recorder.startStressRun();
  for (let index = 0; index < 128; index += 1) {
    recorder.record('request', {
      name: 'sessions.list',
      surface: 'chat',
      state: 'streaming',
      outcome: 'complete',
      code: 'request.complete',
      count: index,
    });
  }
  const batch = recorder.createBatch();
  assert(batch !== null, 'batch exists');
  const nativePayloads = Array.from({ length: 16 }, (_, index) => ({
    id: `native-${index}`,
    kind: index % 2 === 0 ? ('metric' as const) : ('diagnostic' as const),
    receivedAtMs: 1_000 + index,
    summarySchemaVersion: 1 as const,
    sourcePayloadBytes: 2 * 1024 * 1024,
    hasApplicationLaunchMetrics: true,
    hasApplicationResponsivenessMetrics: true,
    hasMemoryMetrics: true,
    hasCpuMetrics: true,
    hasDiskIoMetrics: true,
    hasDisplayMetrics: true,
    hasNetworkTransferMetrics: true,
    hasApplicationExitMetrics: true,
    hasCellularConditionMetrics: true,
    hasLocationActivityMetrics: true,
    hasAnimationMetrics: true,
    crashDiagnosticCount: 1_000,
    hangDiagnosticCount: 1_000,
    cpuExceptionDiagnosticCount: 1_000,
    diskWriteExceptionDiagnosticCount: 1_000,
  }));
  const upload = buildDiagnosticUploadBatch(
    batch,
    {
      appVersion: '0.9.20',
      buildNumber: '261',
      platform: 'ios',
      osVersion: '26.6',
      deviceClass: 'mobile',
      captureLevel: 'stress',
    },
    nativePayloads,
  );
  const encoded = new TextEncoder().encode(JSON.stringify(upload));
  assert(encoded.byteLength < 480 * 1024, 'serialized UTF-8 stays below the safe request budget');
  assert(upload.native_payloads.length === 16, 'every listed native payload is included');
  const summary = JSON.parse(upload.native_payloads[0]?.payload_json ?? '{}');
  assert(summary.schema_version === 1, 'summary schema is explicit');
  assert(summary.has_memory_metrics === true, 'allowlisted metric indicators remain useful');
  assert(!('callStackTree' in summary), 'call stacks are not representable');
  assert(!('exceptionReason' in summary), 'exception reasons are not representable');
});

Deno.test('MetricKit uploads reject native payload overflow instead of silently omitting IDs', () => {
  const recorder = new MobileDiagnosticRecorder({
    installationId: 'install-12345678',
    runId: 'run-12345678',
  });
  recorder.record('heartbeat', { durationMs: 100 });
  const batch = recorder.createBatch();
  assert(batch !== null, 'batch exists');
  const nativePayload = {
    id: 'native-1',
    kind: 'metric' as const,
    receivedAtMs: 1_000,
    summarySchemaVersion: 1 as const,
    sourcePayloadBytes: 100,
    hasApplicationLaunchMetrics: false,
    hasApplicationResponsivenessMetrics: false,
    hasMemoryMetrics: false,
    hasCpuMetrics: false,
    hasDiskIoMetrics: false,
    hasDisplayMetrics: false,
    hasNetworkTransferMetrics: false,
    hasApplicationExitMetrics: false,
    hasCellularConditionMetrics: false,
    hasLocationActivityMetrics: false,
    hasAnimationMetrics: false,
    crashDiagnosticCount: 0,
    hangDiagnosticCount: 0,
    cpuExceptionDiagnosticCount: 0,
    diskWriteExceptionDiagnosticCount: 0,
  };
  let rejected = false;
  try {
    buildDiagnosticUploadBatch(
      batch,
      {
        appVersion: '0.9.20',
        buildNumber: '261',
        platform: 'ios',
        osVersion: '26.6',
        deviceClass: 'mobile',
        captureLevel: 'baseline',
      },
      Array.from({ length: 17 }, (_, index) => ({ ...nativePayload, id: `native-${index}` })),
    );
  } catch {
    rejected = true;
  }
  assert(rejected, 'payload overflow is explicit so callers cannot acknowledge omitted IDs');
});
