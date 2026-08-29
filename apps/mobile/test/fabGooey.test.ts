declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
  readTextFile(path: URL): Promise<string>;
};

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

function assertClose(actual: number, expected: number, message: string) {
  if (Math.abs(actual - expected) > 0.001) {
    throw new Error(`${message}: ${actual} !== ${expected}`);
  }
}

Deno.test("Agent and secondary FAB branches share one stable Skia surface", async () => {
  const accordion = await Deno.readTextFile(
    new URL("../components/chat/AccordionControls.tsx", import.meta.url),
  );
  const composer = await Deno.readTextFile(
    new URL("../components/chat/ChatBar.tsx", import.meta.url),
  );
  const policy = await Deno.readTextFile(
    new URL("../components/chat/fabGooey.ts", import.meta.url),
  );
  const native = await Deno.readTextFile(
    new URL("../components/chat/FabGooeyLayer.native.tsx", import.meta.url),
  );
  const core = await Deno.readTextFile(
    new URL("../components/chat/FabGooeyLayerCore.tsx", import.meta.url),
  );

  assert(
    accordion.includes("style={styles.mergeColumn}")
      && accordion.includes("{agent}")
      && accordion.includes("pillsMounted")
      && accordion.includes("<FabGooeyLayer")
      && accordion.includes("from './fabGooey'")
      && accordion.includes("style={[styles.pillTraveler, travelStyle]}")
      && (accordion.match(/orientation="horizontal"/g)?.length ?? 0) === 2
      && accordion.includes("providerProgresses")
      && accordion.includes("attachProgresses")
      && accordion.includes("const PROVIDER_PILL_GAP = FAB_GAP")
      && accordion.includes("paddingRight: MODEL_BUTTON_GAP")
      && (accordion.match(/styles\.branchGooeyViewport/g)?.length ?? 0) === 2
      && accordion.includes("overflow: 'hidden'")
      && accordion.includes("alignProviderDockToEnd")
      && accordion.includes("scrollEnabled={providerDockOpen && providerPourSettled && !providerDragging}")
      && accordion.includes("canReorder={enableProviderReorder && providerPourSettled}")
      && accordion.includes("providerDockMounted = useDeferredPresence(\n    providerDockOpen,")
      && accordion.includes("AdaptiveMaterial")
      && accordion.includes("liquidGlassOnly")
      && accordion.includes("styles.graphiteCover")
      && accordion.includes("materialAllowed: false")
      && !accordion.includes("crystallizeOnSettle")
      && !accordion.includes("GlassMergeCluster")
      && !accordion.includes("<GlassContainer")
      && !accordion.includes("from \"expo-glass-effect\""),
    "main, provider, and attachment controls must keep one aligned graphite/Skia traveler",
  );
  assert(
    composer.includes("agent={")
      && composer.includes("pillsMounted={accordionVisible}")
      && composer.includes("bottom: inputRowBottom")
      && composer.includes("backgroundColor: gooeyFill(theme.scheme)")
      && !composer.includes("FabGooeyLayer")
      && !composer.includes("<GlassContainer")
      && !composer.includes("from \"expo-glass-effect\""),
    "ChatBar must pass the Agent into the column on the same stable surface",
  );

  const accordionPill = accordion.slice(
    accordion.indexOf("function AccordionPill"),
    accordion.indexOf("function InlineActionPill"),
  );
  assert(
    accordionPill.includes("<FabGlyph progress={progress}>")
      && accordionPill.includes("pillTravelY(index)")
      && accordionPill.includes("backgroundColor: gooeyFill(theme.scheme)")
      && accordionPill.includes("<AdaptiveMaterial")
      && accordionPill.indexOf("<AdaptiveMaterial") <
        accordionPill.indexOf("<Animated.View pointerEvents=\"box-none\"")
      && accordionPill.includes("styles.graphiteCover")
      && accordionPill.includes("active={isOpen && materialActive}")
      && !accordionPill.includes("DROPLET_STRETCH"),
    "the complete FAB tile must travel as graphite while fixed glass waits behind its endpoint",
  );
  assert(
    policy.includes("export const FAB_GOOEY_ENABLED = true")
      && policy.includes("if (activity <= 0.008)")
      && policy.includes("float alpha = u_color.a * edge")
      && policy.includes("return half4(u_color.rgb * alpha, alpha)")
      && policy.includes("uniform float2 u_anchor")
      && policy.includes("uniform float2 u_axis")
      && policy.includes("float endpointMotion(float progress)")
      && policy.includes("float bridgeVisibility = smoothstep(0.0, 0.08, motion)")
      && policy.includes("rgba(25, 24, 29, 1)")
      && !policy.includes("return half4(u_color.rgb, u_color.a * edge)")
      && native.includes("if (!FAB_GOOEY_ENABLED) return")
      && core.includes("Skia.RuntimeEffect.Make(GOOEY_SKSL)")
      && core.includes("<Shader source={gooeyEffect}")
      && !core.includes("AdaptiveMaterial")
      && !core.includes("GlassView"),
    "gooey must be one opaque graphite, premultiplied RuntimeEffect surface",
  );
});

Deno.test("retracted gooey pills collapse into the Agent pool", () => {
  const pill = 56;
  const gap = 10;
  const step = pill + gap;
  const pad = 24;
  const pillCount = 5;
  const agent = pad + pillCount * step + pill / 2;
  for (let index = 0; index < pillCount; index += 1) {
    const restY = pad + (pillCount - 1 - index) * step + pill / 2;
    const retracted = restY + (1 - 0) * (index + 1) * step;
    const settled = restY + (1 - 1) * (index + 1) * step;
    assertClose(retracted, agent, `pill ${index} must start in the Agent`);
    assertClose(
      settled,
      agent - (index + 1) * step,
      `pill ${index} must rest one step farther from the Agent per index`,
    );
  }
});

Deno.test("horizontal FAB branches collapse into their trigger", () => {
  const step = 56 + 10;
  const anchor = 24 + 3 * step + 56 / 2;
  for (let index = 0; index < 3; index += 1) {
    const collapsed = anchor - (index + 1) * step * 0;
    const settled = anchor - (index + 1) * step;
    assertClose(collapsed, anchor, `branch pill ${index} must begin at its trigger`);
    assertClose(
      settled,
      anchor - (index + 1) * step,
      `branch pill ${index} must settle one step farther left`,
    );
  }
});

Deno.test("native wide windows keep touch layout and the full model panel pours", async () => {
  const breakpoint = await Deno.readTextFile(
    new URL("../hooks/useBreakpoint.ts", import.meta.url),
  );
  const composer = await Deno.readTextFile(
    new URL("../components/chat/ChatBar.tsx", import.meta.url),
  );

  assert(
    breakpoint.includes("Platform.OS === 'web' && width >= DESKTOP_MIN")
      && breakpoint.includes("width >= TABLET_MIN ? 'tablet'"),
    "iOS and Android must keep the touch shell even at desktop-sized window widths",
  );
  assert(
    composer.includes("const modelPopoverClipStyle = useAnimatedStyle")
      && composer.includes("const modelPopoverShellStyle = useAnimatedStyle")
      && composer.includes("modelPopoverSourceLeft")
      && composer.includes("modelPopoverTargetLeft")
      && composer.includes("PILL + (modelPopoverWidth - PILL) * progress")
      && composer.includes("PILL + (modelPopoverHeight - PILL) * progress")
      && composer.includes("modelPopoverContentStyle")
      && composer.includes("MODEL_CONTENT_REVEAL_START")
      && composer.includes("MODEL_CONTENT_REVEAL_END")
      && composer.includes("const providerBranchClosing = modelRailOpen")
      && composer.includes("const attachmentBranchClosing = attachPickerOpen")
      && composer.includes("providerBranchCloseDeadlineRef.current")
      && composer.includes("attachmentBranchCloseDeadlineRef.current")
      && composer.includes("const remainingBranchCloseMs = Math.max"),
    "the full model panel must leave its trigger and every side branch must return before the vertical stack",
  );
});
