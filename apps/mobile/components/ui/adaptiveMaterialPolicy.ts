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
  surfaceOverlaySubtle: string;
  surfaceOverlay: string;
  surfaceOverlayElevated: string;
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
  tone: AdaptiveMaterialTone,
  surfaces: AdaptiveMaterialSurfaces,
): string {
  if (tone === "subtle") return surfaces.glassBackground;
  if (tone === "regular") return surfaces.surfaceOverlaySubtle;
  if (tone === "elevated") return surfaces.surfaceOverlay;
  return surfaces.surfaceOverlayElevated;
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
