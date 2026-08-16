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

  assert(
    !accordion.includes("style={styles.fabMaterialLayer}")
      && !composer.includes("style={styles.fabMaterialLayer}"),
    "FABs must preserve AdaptiveMaterial's protected web compositor layer",
  );
  assert(
    accordion.match(/: g\.backgroundElevated,/g)?.length === 4,
    "every accordion FAB family must paint the elevated idle glass token",
  );
  assert(
    accordion.match(/tone="elevated"/g)?.length === 4
      && composer.includes(": t.glass.backgroundElevated,")
      && composer.includes('tone="elevated"'),
    "FAB blur and scrim strength must not depend on a first tap",
  );
  assert(
    material.includes('const [webBlurReady, setWebBlurReady]')
      && material.includes('requestAnimationFrame(() => setWebBlurReady(true))')
      && material.includes("intensity={webBlurReady ? resolvedBlurIntensity : 0}"),
    "web blur must promote from zero after the underlying frame is committed",
  );
  assert(
    material.includes('isolation: "isolate"')
      && material.includes("zIndex: -1"),
    "the protected negative-z blur layer must contain its own web raster",
  );
  assert(
    material.includes("export const AdaptiveMaterial = memo(AdaptiveMaterialComponent)")
      && composer.includes("backgroundColor: t.glass.backgroundElevated,"),
    "composer state changes must not re-render or retint stable blur nodes",
  );
  assert(
    accordion.match(/styles\.materialHost/g)?.length === 4
      && accordion.includes("isolation: 'isolate'")
      && composer.match(/isolation: 'isolate'/g)?.length === 2,
    "every composer and FAB material host must contain its negative-z backdrop",
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
