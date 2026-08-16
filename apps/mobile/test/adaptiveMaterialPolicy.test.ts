import {
  resolveAdaptiveMaterialBlurIntensity,
  resolveAdaptiveMaterialMode,
  resolveAdaptiveMaterialOverlayColor,
  resolveLiquidGlassTintColor,
} from "../components/ui/adaptiveMaterialPolicy";

declare const Deno: {
  test(name: string, fn: () => void | Promise<void>): void;
};

function assertEquals<T>(actual: T, expected: T) {
  if (actual !== expected) {
    throw new Error(`Expected ${String(expected)}, received ${String(actual)}`);
  }
}

Deno.test("uses Liquid Glass only when the complete iOS API is available", () => {
  assertEquals(
    resolveAdaptiveMaterialMode({
      platform: "ios",
      reduceTransparency: false,
      glassApiAvailable: true,
      liquidGlassAvailable: true,
    }),
    "liquid-glass",
  );
});

Deno.test("uses blur for iOS API fallback and web", () => {
  assertEquals(
    resolveAdaptiveMaterialMode({
      platform: "ios",
      reduceTransparency: false,
      glassApiAvailable: false,
      liquidGlassAvailable: false,
    }),
    "blur",
  );
  assertEquals(
    resolveAdaptiveMaterialMode({
      platform: "web",
      reduceTransparency: false,
      glassApiAvailable: false,
      liquidGlassAvailable: false,
    }),
    "blur",
  );
});

Deno.test("uses a solid surface for accessibility and unsupported platforms", () => {
  assertEquals(
    resolveAdaptiveMaterialMode({
      platform: "ios",
      reduceTransparency: true,
      glassApiAvailable: true,
      liquidGlassAvailable: true,
    }),
    "solid",
  );
  assertEquals(
    resolveAdaptiveMaterialMode({
      platform: "android",
      reduceTransparency: false,
      glassApiAvailable: false,
      liquidGlassAvailable: false,
    }),
    "solid",
  );
});

Deno.test("material tones form a restrained blur progression", () => {
  assertEquals(resolveAdaptiveMaterialBlurIntensity("subtle", 20, 40), 14);
  assertEquals(resolveAdaptiveMaterialBlurIntensity("regular", 20, 40), 20);
  assertEquals(resolveAdaptiveMaterialBlurIntensity("elevated", 20, 40), 30);
  assertEquals(resolveAdaptiveMaterialBlurIntensity("strong", 20, 40), 40);
});

Deno.test("fallback scrims prioritize readability while liquid glass stays lightly tinted", () => {
  const surfaces = {
    glassBackground: "glass",
    glassBackgroundElevated: "glass-elevated",
    glassBackgroundPressed: "glass-pressed",
    surfaceOverlaySubtle: "overlay-subtle",
    surfaceOverlay: "overlay",
    surfaceOverlayElevated: "overlay-elevated",
  };

  assertEquals(
    resolveAdaptiveMaterialOverlayColor("regular", surfaces),
    "overlay-subtle",
  );
  assertEquals(
    resolveAdaptiveMaterialOverlayColor("elevated", surfaces),
    "overlay",
  );
  assertEquals(resolveLiquidGlassTintColor("subtle", surfaces), undefined);
  assertEquals(
    resolveLiquidGlassTintColor("strong", surfaces),
    "glass-pressed",
  );
});
