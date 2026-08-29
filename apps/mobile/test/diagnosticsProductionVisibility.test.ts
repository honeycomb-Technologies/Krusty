declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: string): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("production hides diagnostics controls without disabling collection", async () => {
  const settings = await Deno.readTextFile(
    new URL("../components/settings/SettingsPanel.tsx", import.meta.url).pathname,
  );
  const rootLayout = await Deno.readTextFile(
    new URL("../app/_layout.tsx", import.meta.url).pathname,
  );
  const provider = await Deno.readTextFile(
    new URL("../diagnostics/MobileDiagnosticsProvider.tsx", import.meta.url).pathname,
  );

  const developmentComponent = settings.indexOf(
    "function DevelopmentDiagnosticsDisclosure",
  );
  const diagnosticsHook = settings.indexOf(
    "const diagnostics = useMobileDiagnostics();",
  );
  const settingsPanel = settings.indexOf("export function SettingsPanel");
  const developmentGate = settings.indexOf("{__DEV__ ? (", settingsPanel);
  const diagnosticsDisclosure = settings.indexOf(
    "<DevelopmentDiagnosticsDisclosure",
    developmentGate,
  );
  const gateEnd = settings.indexOf(") : null}", diagnosticsDisclosure);

  assert(
    developmentGate >= 0
      && diagnosticsDisclosure > developmentGate
      && gateEnd > diagnosticsDisclosure,
    "the user-visible Diagnostics disclosure must only mount in development",
  );
  assert(
    developmentComponent >= 0
      && diagnosticsHook > developmentComponent
      && diagnosticsHook < settingsPanel
      && !settings
        .slice(settingsPanel)
        .includes("const diagnostics = useMobileDiagnostics();"),
    "only the development disclosure may subscribe to diagnostics context",
  );
  assert(
    rootLayout.includes("<MobileDiagnosticsProvider>")
      && rootLayout.includes("<RootNavigator />")
      && rootLayout.indexOf("<MobileDiagnosticsProvider>")
        < rootLayout.indexOf("<RootNavigator />"),
    "the root must continue mounting background mobile diagnostics in production",
  );
  assert(
    provider.includes("await MitsuroDiagnosticsModule.listMetricKitPayloads()"),
    "the production provider must continue draining native MetricKit payloads",
  );
});
