declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test('JS hot-path probing stays explicit, sampled, and bounded', async () => {
  const source = await Deno.readTextFile(
    new URL('../diagnostics/jsHotPathProbe.ts', import.meta.url).pathname,
  );

  assert(
    source.includes("EXPO_PUBLIC_KRUSTY_JS_HOTPATH_PROBE !== '1'"),
    'the builtin wrapper must remain disabled unless a profiling build explicitly enables it',
  );
  assert(
    source.includes('ARRAY_FROM_SAMPLE_EVERY = 1_024'),
    'callsite stack capture must stay sampled rather than running on every call',
  );
  assert(
    source.includes('MAX_REPORTED_CALLSITES = 8')
      && source.includes('callsites.clear()'),
    'probe aggregation must stay bounded between reports',
  );
  assert(
    source.includes('KrustyDiagnosticsModule?.recordJsHotPathProbe(payload)'),
    'Release probes must use the bounded native diagnostics log rather than a stripped JS console',
  );
});
