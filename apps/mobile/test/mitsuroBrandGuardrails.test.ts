declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
  stat(path: URL): Promise<unknown>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

async function source(path: string): Promise<string> {
  return Deno.readTextFile(new URL(path, import.meta.url));
}

Deno.test("Mitsuro uses one canonical point-up rounded cell", async () => {
  const mark = await source("../components/brand/mitsuro-mark.tsx");
  const hive = await source("../components/brand/hive-icon.tsx");
  const wordmark = await source("../components/brand/mitsuro-wordmark.tsx");
  const brandVectors = await Promise.all([
    source("../../../assets/branding/mitsuro/mitsuro-cell-flat.svg"),
    source("../../../assets/branding/mitsuro/mitsuro-cell-mono.svg"),
    source("../../../assets/branding/mitsuro/mitsuro-cell-dimensional.svg"),
    source("../../../assets/branding/mitsuro/mitsuro-hive.svg"),
    source("../../../assets/branding/mitsuro/mitsuro-wordmark.svg"),
    source("../../../assets/branding/mitsuro/mitsuro-lockup-horizontal.svg"),
    source("../../../assets/branding/mitsuro/mitsuro-splash-mark.svg"),
  ]);

  const canonicalCell =
    "M365 132h294c47 0 76 17 99 58l151 269c20 36 20 70 0 106L758 834c-23 41-52 58-99 58H365c-47 0-76-17-99-58L115 565c-20-36-20-70 0-106l151-269c23-41 52-58 99-58Z";
  const pointUpTransform =
    "translate(512 512) rotate(30) scale(.88) translate(-512 -512)";
  assert(mark.includes(canonicalCell), "brand mark must retain the full approved cell path");
  assert(
    mark.includes(pointUpTransform),
    "the product cell must retain its exact point-up transform",
  );
  assert(
    hive.includes("MITSURO_CELL_PATH"),
    "Hive must be composed from the canonical product cell",
  );
  assert(
    wordmark.includes("scale(.35) rotate(30) scale(.88)"),
    "the wordmark o counter must use the canonical point-up cell",
  );
  for (const vector of brandVectors) {
    assert(vector.includes(canonicalCell), "every Mitsuro mark vector must share the canonical path");
    assert(vector.includes(pointUpTransform), "every Mitsuro mark vector must share the point-up transform");
  }
});

Deno.test("platform identity is Mitsuro without breaking compatibility IDs", async () => {
  const config = JSON.parse(await source("../app.json"));
  const expo = config.expo;

  assert(expo.name === "Mitsuro", "the installed app display name must be Mitsuro");
  // Expo project 6e327449-... is still named "krusty" on expo.dev. EAS requires
  // app.json slug to match that project slug until the Expo project is renamed.
  // Display name + deep-link schemes remain Mitsuro-canonical.
  const allowedSlugs = new Set(["mitsuro", "krusty"]);
  assert(
    allowedSlugs.has(expo.slug),
    "the Expo slug must be mitsuro (canonical) or krusty (EAS project transition)",
  );
  assert(
    expo.slug === "mitsuro" ||
      expo.extra?.eas?.projectId === "6e327449-af3c-4138-b1c4-7ceca2baf243",
    "krusty slug is only allowed for the frozen EAS projectId during transition",
  );
  assert(
    Array.isArray(expo.scheme) &&
      expo.scheme[0] === "mitsuro" &&
      expo.scheme.includes("krusty"),
    "Mitsuro must be the canonical deep-link scheme while the prior scheme remains compatible",
  );
  assert(
    expo.splash?.backgroundColor === "#0e0e11",
    "native splash must use Graphite Brass foundation",
  );
  const splashPlugin = expo.plugins.find(
    (plugin: unknown) => Array.isArray(plugin) && plugin[0] === "expo-splash-screen",
  );
  assert(
    splashPlugin?.[1]?.android?.image === "./assets/splash-icon.png",
    "Android splash must generate the drawable referenced by its native launch theme",
  );
});

Deno.test("splash is a vector six-side simultaneous trace", async () => {
  const animation = JSON.parse(await source("../assets/animations/splash.json"));
  const shapeLayers = animation.layers.filter((layer: { ty: number }) => layer.ty === 4);
  const traceLayer = shapeLayers.find(
    (layer: { nm?: string }) => layer.nm === "Six sides trace together",
  );
  const cellWell = shapeLayers.find(
    (layer: { nm?: string }) => layer.nm === "Cell well",
  );
  const sideGroups = traceLayer?.shapes ?? [];
  const paths = sideGroups.flatMap(
    (group: { it?: Array<{ ty: string }> }) =>
      (group.it ?? []).filter((item) => item.ty === "sh"),
  );
  const trims = sideGroups.flatMap(
    (group: { it?: Array<{ ty: string; e?: { k?: unknown } }> }) =>
      (group.it ?? []).filter((item) => item.ty === "tm"),
  );

  assert(animation.assets.length === 0, "splash must not embed the old raster logo");
  assert(paths.length === 6, "all six cell sides must be independently traced");
  assert(trims.length === 6, "each cell side must have its own simultaneous trim");
  assert(
    trims.every((trim: { e?: { a?: number } }) => trim.e?.a === 1),
    "every cell side must animate its trim path",
  );
  const sourceText = JSON.stringify(animation);
  assert(
    !sourceText.includes("0.7216,0.6039,0.3804"),
    "splash trace must not retain the old brass color",
  );
  assert(
    sourceText.includes("0.6157,0.451,1"),
    "native splash trace must retain the approved violet fallback",
  );
  assert(
    cellWell?.ks?.o?.a === 0 && cellWell.ks.o.k === 0,
    "splash center must stay transparent instead of brightening after the trace",
  );
});

Deno.test("legacy mascot components are no longer app entry points", async () => {
  const surfaces = await Promise.all([
    source("../components/chat/ChatBar.tsx"),
    source("../components/navigation/MobileAppHeader.tsx"),
    source("../components/chat-screen/BootScreen.tsx"),
    source("../components/chat-screen/ActiveConversationSurface.tsx"),
  ]);
  const joined = surfaces.join("\n");

  assert(!joined.includes("CrabIcon"), "composer must not expose the crab mascot");
  assert(!joined.includes("MakoSharkIcon"), "Hive must not expose the old shark mascot");
  assert(!joined.includes("KrustyLogo"), "empty states must use the Mitsuro logo");
});

Deno.test("shared product accents keep graphite foundation with violet motion", async () => {
  const tokens = await source("../../../packages/ui/src/tokens.ts");
  const beam = await source("../components/chat/border-beam/line-spec.json");
  const webLine = await source("../components/chat/ChatBarRunningLine.tsx");

  assert(tokens.includes("#0e0e11"), "graphite must remain the dark foundation");
  assert(
    tokens.includes("rgba(14, 14, 17, 0.36)"),
    "dark glass must tint with the graphite foundation, not a gray frost",
  );
  assert(
    !tokens.includes("rgba(232, 229, 234, 0.055)"),
    "dark glass must not use the old light-gray liquid tint",
  );
  assert(tokens.includes("#b89a61"), "brass must remain the restrained identity accent");
  assert(tokens.includes("#75617e"), "mineral violet must remain the interactive accent");
  assert(tokens.includes("#9a82a5"), "Pulse violet must remain the thinking accent");
  assert(!tokens.includes("#ff6b35"), "legacy orange must not return to shared tokens");
  assert(!tokens.includes("#7f8fa3"), "blue-gray must not return as shared app chrome");
  assert(beam.includes('"violet"'), "running beam must use the violet palette");
  assert(beam.includes("117, 78, 168"), "running beam must retain its deep violet lead");
  assert(!beam.includes("184, 154, 97"), "running beam must not retain the old brass lead");
  assert(!beam.includes("255, 107, 53"), "running beam must not retain legacy orange");
  assert(!beam.includes("127, 143, 163"), "running beam must not retain steel-blue chrome");
  assert(webLine.includes("hue-rotate(210deg)"), "web running line must retain its violet hue");
  assert(!webLine.includes("#b89a61"), "web running line must not reintroduce brass");
  assert(!webLine.includes("#e17a30"), "web running line must not retain rust orange");
});

Deno.test("app chrome uses shared graphite surfaces", async () => {
  const surfaces = await Promise.all([
    source("../components/SettingsModal.tsx"),
    source("../components/ReportsViewer.tsx"),
    source("../components/DirectoryPicker.tsx"),
    source("../components/sheets/AppBottomSheet.tsx"),
    source("../components/layout/DesktopShell.tsx"),
    source("../components/chat/ChatBar.tsx"),
    source("../components/chat/ChatBarModelPopover.tsx"),
    source("../components/chat/ChatTranscript.tsx"),
    source("../components/chat/ImagePreviewModal.tsx"),
    source("../components/chat/MarkdownContent.tsx"),
    source("../components/chat/PlanTracker.tsx"),
    source("../components/hive/HiveEditorModal.tsx"),
  ]);
  const joined = surfaces.join("\n");

  for (const retired of [
    "rgba(11,17,25",
    "rgba(14, 20, 30",
    "#1a1f2e",
    "#090d12",
    "#081018",
  ]) {
    assert(!joined.includes(retired), `app chrome must not retain ${retired}`);
  }
});

Deno.test("production router excludes the visual prototype", async () => {
  let exists = true;
  try {
    await Deno.stat(new URL("../app/navigation-preview.tsx", import.meta.url));
  } catch {
    exists = false;
  }
  assert(!exists, "navigation preview must not ship as an unauthenticated app route");
});
