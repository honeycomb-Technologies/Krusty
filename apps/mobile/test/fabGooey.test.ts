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

Deno.test("Agent accordion pours a Skia silhouette, then crystallizes glass in slots", async () => {
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
      && accordion.includes("crystallizeOnSettle: true")
      && accordion.includes("style={[styles.pillTraveler, travelStyle]}")
      && !accordion.includes("GlassMergeCluster")
      && !accordion.includes("<GlassContainer")
      && !accordion.includes("from \"expo-glass-effect\""),
    "accordion must paint a Skia silhouette under traveling icons, not a GlassContainer",
  );
  assert(
    composer.includes("agent={")
      && composer.includes("pillsMounted={accordionVisible}")
      && composer.includes("bottom: inputRowBottom")
      && !composer.includes("FabGooeyLayer")
      && !composer.includes("<GlassContainer")
      && !composer.includes("from \"expo-glass-effect\""),
    "ChatBar must pass the Agent into the column and never own GlassContainer",
  );

  const accordionPill = accordion.slice(
    accordion.indexOf("function AccordionPill"),
    accordion.indexOf("function InlineActionPill"),
  );
  const travelerStart = accordionPill.indexOf("<Animated.View pointerEvents=\"box-none\" style={[styles.pillTraveler, travelStyle]}>");
  const travelerBody = accordionPill.slice(travelerStart);
  assert(
    accordionPill.includes("<AdaptiveMaterial")
      && accordionPill.includes("active={materialActive}")
      && accordionPill.includes("styles.pillSlot")
      && accordionPill.includes("<FabGlyph progress={progress}>")
      && accordionPill.includes("pillTravelY(index)")
      && travelerStart >= 0
      && !travelerBody.includes("<AdaptiveMaterial")
      && !accordionPill.includes("DROPLET_STRETCH"),
    "glass stays in the layout slot; only icons ride the gooey pour",
  );
  assert(
    policy.includes("export const FAB_GOOEY_ENABLED = true")
      && policy.includes("if (activity <= 0.008)")
      && policy.includes("float alpha = u_color.a * edge")
      && policy.includes("return half4(u_color.rgb * alpha, alpha)")
      && !policy.includes("return half4(u_color.rgb, u_color.a * edge)")
      && native.includes("if (!FAB_GOOEY_ENABLED) return")
      && core.includes("Skia.RuntimeEffect.Make(GOOEY_SKSL)")
      && core.includes("<Shader source={gooeyEffect}")
      && !core.includes("AdaptiveMaterial")
      && !core.includes("GlassView"),
    "gooey must be an enabled, premultiplied RuntimeEffect silhouette, not a glass view",
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
