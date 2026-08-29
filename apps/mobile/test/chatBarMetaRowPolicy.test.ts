declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test('phone composer hides the context ring without hiding useful status', async () => {
  const metaRow = await Deno.readTextFile(
    new URL('../components/chat/ChatBarMetaRow.tsx', import.meta.url),
  );
  const composer = await Deno.readTextFile(
    new URL('../components/chat/ChatBar.tsx', import.meta.url),
  );

  assert(
    metaRow.includes('{isDesktop || workspaceContext ? (')
      && metaRow.includes(
        '{isDesktop ? (\n            <View style={styles.gaugeRing}>',
      )
      && metaRow.includes('{workspaceContext ? (')
      && metaRow.includes('{currentModelLabel}')
      && metaRow.includes('{thinkingLabel}'),
    'mobile must omit the decorative context ring while retaining workspace, model, and thinking status',
  );
  assert(
    composer.includes('<ChatBarMetaRow'),
    'the status row must remain mounted; this polish must not remove composer metadata',
  );
});
