export type AdaptiveMaterialMode = "liquid-glass" | "blur" | "solid";
export type AdaptiveMaterialTone =
  | "subtle"
  | "regular"
  | "elevated"
  | "strong";

export interface AdaptiveMaterialSurfaces {
  glassBackground: string;
  glassBackgroundElevated: string;
  glassBackgroundPressed: string;
}

interface ResolveAdaptiveMaterialModeArgs {
  platform: string;
  reduceTransparency: boolean;
  glassApiAvailable: boolean;
  liquidGlassAvailable: boolean;
}

export function resolveAdaptiveMaterialMode({
  platform,
  reduceTransparency,
  glassApiAvailable,
  liquidGlassAvailable,
}: ResolveAdaptiveMaterialModeArgs): AdaptiveMaterialMode {
  if (reduceTransparency) return "solid";

  if (
    platform === "ios" &&
    glassApiAvailable &&
    liquidGlassAvailable
  ) {
    return "liquid-glass";
  }

  if (platform === "ios" || platform === "web") return "blur";
  return "solid";
}

export function resolveAdaptiveMaterialBlurIntensity(
  tone: AdaptiveMaterialTone,
  regularIntensity: number,
  intenseIntensity: number,
): number {
  if (tone === "subtle") return Math.max(12, regularIntensity - 6);
  if (tone === "regular") return regularIntensity;
  if (tone === "elevated") {
    return Math.round((regularIntensity + intenseIntensity) / 2);
  }
  return intenseIntensity;
}

export function resolveAdaptiveMaterialOverlayColor(
  mode: AdaptiveMaterialMode,
  tone: AdaptiveMaterialTone,
  surfaces: AdaptiveMaterialSurfaces,
): string | undefined {
  // Liquid glass owns its own adaptive contrast; a stacked scrim buries the
  // native effect. Readability tuning happens through the glass tint instead.
  if (mode === "liquid-glass") return undefined;

  // Blur fallback gets the designed translucent glass fills, never the heavy
  // surfaceOverlay* scrims that read as a solid slab over the material.
  if (tone === "subtle" || tone === "regular") return surfaces.glassBackground;
  if (tone === "elevated") return surfaces.glassBackgroundElevated;
  return surfaces.glassBackgroundPressed;
}

export function resolveLiquidGlassTintColor(
  tone: AdaptiveMaterialTone,
  surfaces: AdaptiveMaterialSurfaces,
): string | undefined {
  if (tone === "subtle") return undefined;
  if (tone === "regular") return surfaces.glassBackground;
  if (tone === "elevated") return surfaces.glassBackgroundElevated;
  return surfaces.glassBackgroundPressed;
}
