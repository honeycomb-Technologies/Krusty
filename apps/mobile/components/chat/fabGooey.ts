import type { SharedValue } from 'react-native-reanimated';

export const FAB_PILL = 56;
export const FAB_GAP = 10;
export const FAB_RADIUS = 18;
export const FAB_STEP = FAB_PILL + FAB_GAP;
export const GOOEY_PAD = 24;
export const GOOEY_SMIN = 18;
export const MAX_GOOEY_PILLS = 6;
/** Silhouette only — never a GlassView. Keep true; the native layer still defers Core import. */
export const FAB_GOOEY_ENABLED = true;

/**
 * Smooth-min metaballs: the same family as the running-line RuntimeEffect,
 * not an offscreen Blur/ColorMatrix image filter.
 */
export const GOOEY_SKSL = `
uniform float2 u_resolution;
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

float pillCenterY(float index, float progress) {
  float halfPill = u_pill * 0.5;
  float restY = u_pad + (u_count - 1.0 - index) * u_step + halfPill;
  return restY + (1.0 - progress) * (index + 1.0) * u_step;
}

float mergePill(float field, float2 p, float cx, float halfPill, float index, float progress) {
  if (progress <= 0.008) {
    return field;
  }
  float cy = pillCenterY(index, progress);
  float d = roundedRectSDF(p - float2(cx, cy), float2(halfPill, halfPill), u_radius);
  return smin(field, d, u_smin);
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

  float2 p = xy;
  float cx = u_resolution.x * 0.5;
  float halfPill = u_pill * 0.5;
  float agentCy = u_resolution.y - u_pad - halfPill;
  float field = roundedRectSDF(
    p - float2(cx, agentCy),
    float2(halfPill, halfPill),
    u_radius
  );

  if (u_count > 0.5) { field = mergePill(field, p, cx, halfPill, 0.0, u_p0); }
  if (u_count > 1.5) { field = mergePill(field, p, cx, halfPill, 1.0, u_p1); }
  if (u_count > 2.5) { field = mergePill(field, p, cx, halfPill, 2.0, u_p2); }
  if (u_count > 3.5) { field = mergePill(field, p, cx, halfPill, 3.0, u_p3); }
  if (u_count > 4.5) { field = mergePill(field, p, cx, halfPill, 4.0, u_p4); }
  if (u_count > 5.5) { field = mergePill(field, p, cx, halfPill, 5.0, u_p5); }

  float edge = 1.0 - smoothstep(-0.85, 0.85, field);
  return half4(u_color.rgb, u_color.a * edge);
}
`;

export function gooeyCanvasHeight(pillCount: number): number {
  return GOOEY_PAD * 2 + pillCount * FAB_STEP + FAB_PILL;
}

export function gooeyCanvasWidth(): number {
  return FAB_PILL + GOOEY_PAD * 2;
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
    ? 'rgba(36, 34, 42, 0.94)'
    : 'rgba(246, 243, 238, 0.94)';
}

export function parseGooeyFill(css: string): [number, number, number, number] {
  const match = css
    .trim()
    .match(
      /^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/,
    );
  if (!match) return [36 / 255, 34 / 255, 42 / 255, 0.94];
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
