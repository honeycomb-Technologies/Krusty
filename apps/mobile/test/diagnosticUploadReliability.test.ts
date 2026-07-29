declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test('diagnostic completion drains checkpoints before one empty marker', async () => {
  const provider = await Deno.readTextFile(
    new URL('../diagnostics/MobileDiagnosticsProvider.tsx', import.meta.url).pathname,
  );

  assert(
    provider.includes('completed: false'),
    'event-bearing checkpoints must not close a stress run',
  );
  assert(
    provider.includes('const marker = createCompletionMarkerBatch(recorder)'),
    'completion must use a dedicated marker after checkpoint draining',
  );
  assert(
    provider.includes('completed: true'),
    'the dedicated marker must be marked completed',
  );
  assert(
    provider.includes('void flush(true);'),
    'restored or reconnected pending completions must retry immediately',
  );
});

Deno.test('diagnostics controls keep stop actionable and label active checkpoints', async () => {
  const settings = await Deno.readTextFile(
    new URL('../components/settings/sections.tsx', import.meta.url).pathname,
  );

  assert(
    settings.includes('{mode === "stress" ? "Upload checkpoint" : "Upload now"}'),
    'the active secondary action must describe a checkpoint upload',
  );
  assert(
    !settings.includes('onPress={onStopAndUpload}\n                disabled={uploadState === "uploading"}'),
    'Stop must remain actionable while a checkpoint is uploading',
  );
});
