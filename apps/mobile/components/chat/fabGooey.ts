import type { SharedValue } from 'react-native-reanimated';

export const FAB_PILL = 56;
export const FAB_GAP = 10;
export const FAB_RADIUS = 18;
export const FAB_STEP = FAB_PILL + FAB_GAP;
export const GOOEY_PAD = 24;
export const GOOEY_SMIN = 18;
export const MAX_GOOEY_PILLS = 6;
export const FAB_POUR_OPEN_SPRING = {
  damping: 24,
  stiffness: 198,
  mass: 0.96,
} as const;
export const FAB_POUR_CLOSE_MS = 160;
/** Silhouette only — never a GlassView. Keep true; the native layer still defers Core import. */
export const FAB_GOOEY_ENABLED = true;
export type GooeyOrientation = 'vertical' | 'horizontal';

/**
 * Smooth-min metaballs: the same family as the running-line RuntimeEffect,
 * not an offscreen Blur/ColorMatrix image filter.
 */
export const GOOEY_SKSL = `
uniform float2 u_resolution;
uniform float2 u_anchor;
uniform float2 u_axis;
uniform float u_pad;
uniform float u_pill;
uniform float u_radius;
uniform float u_step;
uniform float u_count;
uniform float u_smin;
uniform float4 u_color;
uniform float u_p0;
uniform float u_p1;
uniform float u_p2;
uniform float u_p3;
uniform float u_p4;
uniform float u_p5;

float roundedRectSDF(float2 p, float2 halfSize, float radius) {
  float2 q = abs(p) - halfSize + radius;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float smin(float a, float b, float k) {
  float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
  return mix(b, a, h) - k * h * (1.0 - h);
}

float2 pillCenter(float index, float progress) {
  return u_anchor + u_axis * ((index + 1.0) * u_step * progress);
}

float mergePill(float field, float2 p, float halfPill, float index, float progress) {
  if (progress <= 0.008) {
    return field;
  }
  float2 center = pillCenter(index, progress);
  float d = roundedRectSDF(p - center, float2(halfPill, halfPill), u_radius);
  return smin(field, d, u_smin);
}

float endpointMotion(float progress) {
  return min(abs(progress), abs(1.0 - progress));
}

half4 main(float2 xy) {
  float activity = u_p0;
  activity = max(activity, u_p1);
  activity = max(activity, u_p2);
  activity = max(activity, u_p3);
  activity = max(activity, u_p4);
  activity = max(activity, u_p5);
  if (activity <= 0.008) {
    return half4(0.0);
  }

  float motion = 0.0;
  if (u_count > 0.5) { motion = max(motion, endpointMotion(u_p0)); }
  if (u_count > 1.5) { motion = max(motion, endpointMotion(u_p1)); }
  if (u_count > 2.5) { motion = max(motion, endpointMotion(u_p2)); }
  if (u_count > 3.5) { motion = max(motion, endpointMotion(u_p3)); }
  if (u_count > 4.5) { motion = max(motion, endpointMotion(u_p4)); }
  if (u_count > 5.5) { motion = max(motion, endpointMotion(u_p5)); }
  float bridgeVisibility = smoothstep(0.0, 0.08, motion);
  if (bridgeVisibility <= 0.001) {
    return half4(0.0);
  }

  float2 p = xy;
  float halfPill = u_pill * 0.5;
  float field = roundedRectSDF(
    p - u_anchor,
    float2(halfPill, halfPill),
    u_radius
  );

  if (u_count > 0.5) { field = mergePill(field, p, halfPill, 0.0, u_p0); }
  if (u_count > 1.5) { field = mergePill(field, p, halfPill, 1.0, u_p1); }
  if (u_count > 2.5) { field = mergePill(field, p, halfPill, 2.0, u_p2); }
  if (u_count > 3.5) { field = mergePill(field, p, halfPill, 3.0, u_p3); }
  if (u_count > 4.5) { field = mergePill(field, p, halfPill, 4.0, u_p4); }
  if (u_count > 5.5) { field = mergePill(field, p, halfPill, 5.0, u_p5); }

  float edge = 1.0 - smoothstep(-0.85, 0.85, field);
  float alpha = u_color.a * edge * bridgeVisibility;
  // Skia shaders return premultiplied colors. Straight RGB leaks through the
  // transparent Android canvas as a tinted rectangle and duplicate halos.
  return half4(u_color.rgb * alpha, alpha);
}
`;

export function gooeyCanvasHeight(
  pillCount: number,
  orientation: GooeyOrientation = 'vertical',
): number {
  return orientation === 'vertical'
    ? GOOEY_PAD * 2 + pillCount * FAB_STEP + FAB_PILL
    : FAB_PILL + GOOEY_PAD * 2;
}

export function gooeyCanvasWidth(
  pillCount = 0,
  orientation: GooeyOrientation = 'vertical',
): number {
  return orientation === 'vertical'
    ? FAB_PILL + GOOEY_PAD * 2
    : GOOEY_PAD * 2 + pillCount * FAB_STEP + FAB_PILL;
}

export function gooeyAnchorPoint(
  pillCount: number,
  orientation: GooeyOrientation = 'vertical',
): readonly [number, number] {
  const halfPill = FAB_PILL / 2;
  if (orientation === 'horizontal') {
    return [
      gooeyCanvasWidth(pillCount, orientation) - GOOEY_PAD - halfPill,
      GOOEY_PAD + halfPill,
    ];
  }
  return [
    GOOEY_PAD + halfPill,
    gooeyCanvasHeight(pillCount, orientation) - GOOEY_PAD - halfPill,
  ];
}

export function gooeyAxis(
  orientation: GooeyOrientation = 'vertical',
): readonly [number, number] {
  return orientation === 'vertical' ? [0, -1] : [-1, 0];
}

export function pillTravelY(index: number): number {
  'worklet';
  return (index + 1) * FAB_STEP;
}

export function gooeyAgentCenterY(pillCount: number): number {
  return GOOEY_PAD + pillCount * FAB_STEP + FAB_PILL / 2;
}

export function gooeyPillCenterY(
  index: number,
  pillCount: number,
  progress: number,
): number {
  const restY = GOOEY_PAD + (pillCount - 1 - index) * FAB_STEP + FAB_PILL / 2;
  return restY + (1 - progress) * pillTravelY(index);
}

export function gooeyFill(scheme: 'dark' | 'light'): string {
  return scheme === 'dark'
    ? 'rgba(25, 24, 29, 1)'
    : 'rgba(246, 243, 238, 1)';
}

export function parseGooeyFill(css: string): [number, number, number, number] {
  const match = css
    .trim()
    .match(
      /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/,
    );
  if (!match) return [25 / 255, 24 / 255, 29 / 255, 1];
  return [
    Number.parseFloat(match[1]) / 255,
    Number.parseFloat(match[2]) / 255,
    Number.parseFloat(match[3]) / 255,
    match[4] == null ? 1 : Number.parseFloat(match[4]),
  ];
}

export type GooeyProgresses = readonly [
  SharedValue<number>,
  SharedValue<number>,
  SharedValue<number>,
  SharedValue<number>,
  SharedValue<number>,
  SharedValue<number>,
];
