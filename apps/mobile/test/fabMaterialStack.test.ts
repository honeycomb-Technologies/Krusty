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
  const glassCard = await Deno.readTextFile(
    new URL("../components/ui/GlassCard.tsx", import.meta.url),
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
      && material.includes("{ backgroundColor: overlayColor }")
      && !composer.includes("backgroundColor: t.glass.backgroundElevated,"),
    "native glass, blur fallback, composer, and FABs must share the same readable scrim",
  );
  assert(
    accordion.match(/styles\.materialHost/g)?.length === 4
      && accordion.includes("position: 'relative'")
      && (composer.match(/position: 'relative'/g)?.length ?? 0) >= 2,
    "every composer and FAB material host must anchor its background layer",
  );
  assert(
    (header.match(/position: "relative"/g)?.length ?? 0) >= 3
      && modelPopover.includes("position: 'relative'")
      && glassCard.includes("position: 'relative'"),
    "every non-absolute material host must bound its backdrop to the component",
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
