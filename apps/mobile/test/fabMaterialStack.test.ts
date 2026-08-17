declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

Deno.test("FAB material strength is stable before and after interaction", async () => {
  const accordion = await Deno.readTextFile(
    new URL("../components/chat/AccordionControls.tsx", import.meta.url),
  );
  const composer = await Deno.readTextFile(
    new URL("../components/chat/ChatBar.tsx", import.meta.url),
  );
  const material = await Deno.readTextFile(
    new URL("../components/ui/AdaptiveMaterial.tsx", import.meta.url),
  );
  const header = await Deno.readTextFile(
    new URL("../components/navigation/MobileAppHeader.tsx", import.meta.url),
  );
  const modelPopover = await Deno.readTextFile(
    new URL("../components/chat/ChatBarModelPopover.tsx", import.meta.url),
  );

  assert(
    !accordion.includes("style={styles.fabMaterialLayer}")
      && !composer.includes("style={styles.fabMaterialLayer}"),
    "FABs must use the centralized AdaptiveMaterial compositor layer",
  );
  assert(
    !accordion.includes("g.backgroundElevated")
      && accordion.match(/: 'transparent'/g)?.length === 4,
    "every idle accordion FAB must expose the shared material instead of stacking a private tint",
  );
  assert(
    accordion.match(/tone="regular"/g)?.length === 4
      && composer.match(/tone="regular"/g)?.length === 2,
    "composer and FAB controls must use one floating material tone",
  );
  assert(
    material.includes('const [webBlurReady, setWebBlurReady]')
      && material.includes('requestAnimationFrame(() => setWebBlurReady(true))')
      && material.includes("intensity={webBlurReady ? resolvedBlurIntensity : 0}"),
    "web blur must promote from zero after the underlying frame is committed",
  );
  assert(
    !material.includes('isolation: "isolate"')
      && !material.includes("zIndex: -1")
      && !accordion.includes("isolation: 'isolate'")
      && !composer.includes("isolation: 'isolate'"),
    "floating materials must see the real page backdrop instead of an isolated negative stack",
  );
  assert(
    material.includes("export const AdaptiveMaterial = memo(AdaptiveMaterialComponent)")
      && !composer.includes("backgroundColor: t.glass.backgroundElevated,"),
    "composer and FABs must expose the shared material instead of private tints",
  );
  const liquidGlassBranch = material.slice(
    material.indexOf('if (materialMode === "liquid-glass")'),
    material.indexOf('if (materialMode === "blur")'),
  );
  assert(
    liquidGlassBranch.includes("<NativeGlassView")
      && liquidGlassBranch.includes("borderRadius={borderRadius}")
      && !liquidGlassBranch.includes("overlayColor"),
    "liquid glass must render the bare native effect with its native corner radius and no stacked scrim",
  );
  assert(
    (material.match(/\{ backgroundColor: overlayColor \}/g)?.length ?? 0) === 1,
    "only the blur fallback may add a translucent glass-fill overlay",
  );
  assert(
    accordion.match(/styles\.materialHost/g)?.length === 4
      && accordion.includes("position: 'relative'")
      && (composer.match(/position: 'relative'/g)?.length ?? 0) >= 2,
    "every composer and FAB material host must anchor its background layer",
  );
  assert(
    (header.match(/position: "relative"/g)?.length ?? 0) >= 3
      && modelPopover.includes("position: 'relative'"),
    "every non-absolute chat-chrome material host must bound its backdrop to the component",
  );
  assert(
    accordion.includes("<Animated.View style={animatedStyle}>")
      && !accordion.includes("styles.pointerBoxNone,\n        animatedStyle,"),
    "full-width accordion rows must stay static while only the bounded FAB animates",
  );
  assert(
    composer.includes("if (Platform.OS === 'web') return;")
      && composer.includes("const kActive = accordionOpen;"),
    "web glass controls must stay mounted after first use without staying visually active",
  );
});

Deno.test("glass chrome never sits under an animated opacity ancestor", async () => {
  const entrance = await Deno.readTextFile(
    new URL("../hooks/useEntranceAnimation.ts", import.meta.url),
  );
  const accordion = await Deno.readTextFile(
    new URL("../components/chat/AccordionControls.tsx", import.meta.url),
  );
  const composer = await Deno.readTextFile(
    new URL("../components/chat/ChatBar.tsx", import.meta.url),
  );
  const layout = await Deno.readTextFile(
    new URL("../app/_layout.tsx", import.meta.url),
  );

  assert(
    !entrance.includes("opacity:")
      && !entrance.includes("Opacity"),
    "entrance wrappers must slide/scale only; ancestor alpha kills iOS liquid glass",
  );
  assert(
    !accordion.includes("opacity: opacityProgress")
      && !accordion.includes("opacity: revealOpacityProgress")
      && !accordion.includes("opacity: progress.value"),
    "accordion glass pills must hide with scale/translate, not Reanimated opacity",
  );
  assert(
    !composer.includes("modelPopoverOpacity")
      && !composer.includes("opacity: modelPopoverOpacity"),
    "the model popover must not fade its AdaptiveMaterial ancestor",
  );
  assert(
    layout.includes("animation: 'none'"),
    "the root stack must not fade-in screens that host liquid glass",
  );
});

Deno.test("liquid glass stays on chat chrome, not drawers or sheets", async () => {
  const solidSurfaces = [
    "../components/sheets/AppBottomSheet.tsx",
    "../components/SettingsModal.tsx",
    "../components/ReportsViewer.tsx",
    "../components/DirectoryPicker.tsx",
    "../components/chat/ImagePreviewModal.tsx",
    "../components/ui/GlassCard.tsx",
    "../components/chat/SessionDrawer.tsx",
    "../components/ToolboxPanel.tsx",
    "../components/settings/sections.tsx",
  ];
  const sources = await Promise.all(
    solidSurfaces.map((file) =>
      Deno.readTextFile(new URL(file, import.meta.url))
    ),
  );
  const leaked = solidSurfaces.filter((file, index) =>
    sources[index].includes("AdaptiveMaterial")
  );

  assert(
    leaked.length === 0,
    `AdaptiveMaterial is chat chrome only; unexpected uses: ${leaked.join(", ")}`,
  );
  assert(
    sources[0].includes("backgroundColor: t.background"),
    "session and toolbox drawers must use a solid theme fill",
  );
  assert(
    sources[5].includes(
      "backgroundColor: elevated ? t.surfaceElevated : t.surface",
    ),
    "settings and list cards must stay solid theme surfaces",
  );
});
