declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("native performance spans use fixed privacy-safe signposts", async () => {
  const nativeSource = await Deno.readTextFile(
    new URL(
      "../modules/krusty-diagnostics/ios/KrustyDiagnosticsModule.swift",
      import.meta.url,
    ).pathname,
  );
  const performanceSource = await Deno.readTextFile(
    new URL("../../../packages/state/src/performance.ts", import.meta.url).pathname,
  );
  const providerSource = await Deno.readTextFile(
    new URL("../diagnostics/MobileDiagnosticsProvider.tsx", import.meta.url).pathname,
  );

  assert(
    nativeSource.includes("private let allowedNames: Set<String>")
      && nativeSource.includes("guard allowedNames.contains(name) else { return }"),
    "native signposts must reject arbitrary labels",
  );
  assert(
    nativeSource.includes('"KrustyPerformance"')
      && !nativeSource.includes("detail, privacy:"),
    "native intervals must never contain session detail",
  );
  assert(
    performanceSource.includes("__KRUSTY_NATIVE_PERFORMANCE__?.begin(spanId, name)")
      && performanceSource.includes("__KRUSTY_NATIVE_PERFORMANCE__?.end(spanId, name)"),
    "JS spans must bracket matching native intervals",
  );
  assert(
    providerSource.includes("KrustyDiagnosticsModule?.getBuildNumber()"),
    "uploads must identify the installed CFBundleVersion",
  );
});
