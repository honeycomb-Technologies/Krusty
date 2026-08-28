declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("desktop shell forwards Hive navigation into the collapsed sidebar", async () => {
  const [shell, list] = await Promise.all([
    Deno.readTextFile(
      new URL("../components/layout/DesktopShell.tsx", import.meta.url),
    ),
    Deno.readTextFile(
      new URL("../components/chat/SessionList.tsx", import.meta.url),
    ),
  ]);

  assert(
    shell.includes("onSelectHiveView={onSelectHiveView}"),
    "DesktopShell must forward the Hive view callback into SessionList",
  );
  assert(
    list.includes("onSelectHiveView && activeTab === 2") &&
      list.includes("HIVE_PRIMARY_NAV_ITEMS.map"),
    "the desktop Hive sidebar must expose every primary Hive view",
  );
});
