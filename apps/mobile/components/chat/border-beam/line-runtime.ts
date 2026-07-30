/**
 * Line-only runtime adapted from Jakub Antalik's MIT-licensed border-beam.
 * See NOTICE.md in this directory for attribution.
 */

export const BLOB_FLOATS = 14;
export const MAX_LAYER_BLOBS = 16;

export const BLOB_LAYER_SKSL = `
uniform float2 uSize;
uniform float2 uRectOrigin;
uniform float2 uRectSize;
uniform float uRadius;
uniform float uBorderWidth;
uniform float uGeomKind;
uniform float uEdgeMaskPx;
uniform float uRadial[7];
uniform float uBlobs[${MAX_LAYER_BLOBS * BLOB_FLOATS}];
uniform float uBlobCount;
uniform float uCM[9];
uniform float uOpacity;

float roundedRectSDF(float2 p, float2 halfSize, float radius) {
  float2 q = abs(p) - halfSize + radius;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - radius;
}

float4 srcOver(float4 src, float4 dst) {
  return src + dst * (1.0 - src.a);
}

float stopAlpha(float t, float a0, float p1, float a1, float p2, float a2, float p3) {
  if (t >= p3) { return 0.0; }
  if (t <= 0.0) { return a0; }
  if (t < p1) { return mix(a0, a1, t / max(p1, 0.0001)); }
  if (t < p2) { return mix(a1, a2, (t - p1) / max(p2 - p1, 0.0001)); }
  return mix(a2, 0.0, (t - p2) / max(p3 - p2, 0.0001));
}

half4 main(float2 position) {
  float2 rectCenter = uRectOrigin + uRectSize * 0.5;
  float2 rel = position - rectCenter;
  int kind = int(uGeomKind);
  float geom = 1.0;
  float aa = 1.0;

  if (kind != 2) {
    float outerSDF = roundedRectSDF(rel, uRectSize * 0.5, uRadius);
    float outerCov = 1.0 - smoothstep(-aa, 0.0, outerSDF);
    if (kind == 1) {
      geom = outerCov;
    } else {
      float innerRadius = max(uRadius - uBorderWidth, 0.0);
      float innerSDF = roundedRectSDF(rel, uRectSize * 0.5 - uBorderWidth, innerRadius);
      float innerCov = 1.0 - smoothstep(-aa, 0.0, innerSDF);
      geom = outerCov - innerCov;
    }
    if (geom <= 0.0) { return half4(0.0); }
  }

  float mask = 1.0;
  if (kind == 1 && uEdgeMaskPx > 0.0) {
    float2 lp = position - uRectOrigin;
    float ev = max(1.0 - lp.y / uEdgeMaskPx,
                   1.0 - (uRectSize.y - lp.y) / uEdgeMaskPx);
    float eh = max(1.0 - lp.x / uEdgeMaskPx,
                   1.0 - (uRectSize.x - lp.x) / uEdgeMaskPx);
    mask *= clamp(max(ev, 0.0) + max(eh, 0.0), 0.0, 1.0);
  }

  if (uRadial[6] > 0.5) {
    float2 c = float2(uRadial[0], uRadial[1]);
    float t = length((position - c) / float2(max(uRadial[2], 0.001), max(uRadial[3], 0.001)));
    float midPos = uRadial[4];
    float midAlpha = uRadial[5];
    float m;
    if (t >= 1.0) {
      m = 0.0;
    } else if (t < midPos) {
      m = mix(1.0, midAlpha, t / max(midPos, 0.0001));
    } else {
      m = mix(midAlpha, 0.0, (t - midPos) / max(1.0 - midPos, 0.0001));
    }
    mask *= m;
  }
  if (mask <= 0.001) { return half4(0.0); }

  float4 acc = float4(0.0);
  int nBlobs = int(uBlobCount);
  for (int i = ${MAX_LAYER_BLOBS - 1}; i >= 0; i--) {
    if (i < nBlobs) {
      float rx = max(uBlobs[i * ${BLOB_FLOATS} + 0], 0.001);
      float ry = max(uBlobs[i * ${BLOB_FLOATS} + 1], 0.001);
      float2 c = float2(uBlobs[i * ${BLOB_FLOATS} + 2], uBlobs[i * ${BLOB_FLOATS} + 3]);
      float t = length((position - c) / float2(rx, ry));
      float alpha = stopAlpha(
        t,
        uBlobs[i * ${BLOB_FLOATS} + 7],
        uBlobs[i * ${BLOB_FLOATS} + 8],
        uBlobs[i * ${BLOB_FLOATS} + 9],
        uBlobs[i * ${BLOB_FLOATS} + 10],
        uBlobs[i * ${BLOB_FLOATS} + 11],
        uBlobs[i * ${BLOB_FLOATS} + 12]
      );
      if (alpha > 0.0) {
        float3 rgb = float3(
          uBlobs[i * ${BLOB_FLOATS} + 4],
          uBlobs[i * ${BLOB_FLOATS} + 5],
          uBlobs[i * ${BLOB_FLOATS} + 6]
        );
        acc = srcOver(float4(rgb * alpha, alpha), acc);
      }
    }
  }

  if (acc.a > 0.0001) {
    float3 rgb = acc.rgb / acc.a;
    float3 outRGB = float3(
      dot(float3(uCM[0], uCM[1], uCM[2]), rgb),
      dot(float3(uCM[3], uCM[4], uCM[5]), rgb),
      dot(float3(uCM[6], uCM[7], uCM[8]), rgb)
    );
    acc.rgb = clamp(outRGB, 0.0, 1.0) * acc.a;
  }

  float finalA = acc.a * geom * mask * clamp(uOpacity, 0.0, 1.0);
  if (acc.a > 0.0001) {
    return half4(half3(acc.rgb / acc.a * finalA), half(finalA));
  }
  return half4(0.0);
}
`;

export interface RGBA {
  r: number;
  g: number;
  b: number;
  a: number;
}

export function parseCssColor(css: string): RGBA {
  const match = css
    .trim()
    .match(/^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:,\s*([\d.]+)\s*)?\)$/);
  if (!match) return { r: 1, g: 1, b: 1, a: 1 };
  return {
    r: Number.parseFloat(match[1]) / 255,
    g: Number.parseFloat(match[2]) / 255,
    b: Number.parseFloat(match[3]) / 255,
    a: match[4] == null ? 1 : Number.parseFloat(match[4]),
  };
}

export function simpleBlob(
  rx: number,
  ry: number,
  cx: number,
  cy: number,
  r: number,
  g: number,
  b: number,
  a: number,
): number[] {
  'worklet';
  return [rx, ry, cx, cy, r, g, b, a, 1 / 3, a * (2 / 3), 2 / 3, a / 3, 1, 0];
}

export interface GradientStop {
  r: number;
  g: number;
  b: number;
  a: number;
  pos: number;
}

export function stopsBlob(
  rx: number,
  ry: number,
  cx: number,
  cy: number,
  stops: GradientStop[],
): number[] {
  'worklet';
  const first = stops[0];
  const r = first.r / 255;
  const g = first.g / 255;
  const b = first.b / 255;
  const a0 = first.a;
  const s1 = stops[1] ?? { a: 0, pos: 1 };
  const s2 = stops[2] ?? { a: s1.a / 2, pos: (s1.pos + 1) / 2 };
  const s3 = stops[3] ?? { a: 0, pos: Math.max(s2.pos, 1) };

  if (stops.length === 3) {
    const mid = { a: stops[1].a / 2, pos: (stops[1].pos + stops[2].pos) / 2 };
    return [
      rx, ry, cx, cy, r, g, b, a0,
      stops[1].pos, stops[1].a, mid.pos, mid.a, stops[2].pos, 0,
    ];
  }
  if (stops.length === 4) {
    return [rx, ry, cx, cy, r, g, b, a0, s1.pos, s1.a, s2.pos, s2.a, s3.pos, 0];
  }
  const end = s1.pos;
  return [
    rx, ry, cx, cy, r, g, b, a0,
    end / 3, a0 * (2 / 3), (2 * end) / 3, a0 / 3, end, 0,
  ];
}

export function padTo(values: number[], length: number): number[] {
  'worklet';
  if (values.length >= length) return values.slice(0, length);
  return values.concat(new Array<number>(length - values.length).fill(0));
}

export function composedFilterMatrix(
  hueDegrees: number,
  brightness: number,
  saturation: number,
): number[] {
  'worklet';
  const rad = (hueDegrees * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  const hue = [
    (0.213 + cos * 0.787 - sin * 0.213) * brightness,
    (0.715 - cos * 0.715 - sin * 0.715) * brightness,
    (0.072 - cos * 0.072 + sin * 0.928) * brightness,
    (0.213 - cos * 0.213 + sin * 0.143) * brightness,
    (0.715 + cos * 0.285 + sin * 0.14) * brightness,
    (0.072 - cos * 0.072 - sin * 0.283) * brightness,
    (0.213 - cos * 0.213 - sin * 0.787) * brightness,
    (0.715 - cos * 0.715 + sin * 0.715) * brightness,
    (0.072 + cos * 0.928 + sin * 0.072) * brightness,
  ];
  const sat = [
    0.213 + 0.787 * saturation,
    0.715 - 0.715 * saturation,
    0.072 - 0.072 * saturation,
    0.213 - 0.213 * saturation,
    0.715 + 0.285 * saturation,
    0.072 - 0.072 * saturation,
    0.213 - 0.213 * saturation,
    0.715 - 0.715 * saturation,
    0.072 + 0.928 * saturation,
  ];
  const output = new Array<number>(9).fill(0);
  for (let row = 0; row < 3; row += 1) {
    for (let column = 0; column < 3; column += 1) {
      let sum = 0;
      for (let k = 0; k < 3; k += 1) {
        sum += sat[row * 3 + k] * hue[k * 3 + column];
      }
      output[row * 3 + column] = sum;
    }
  }
  return output;
}

export function pingPong(phase: number): number {
  'worklet';
  return (1 - Math.cos(2 * Math.PI * phase)) / 2;
}

function cssEaseInOut(x: number): number {
  'worklet';
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  let low = 0;
  let high = 1;
  let t = x;
  for (let index = 0; index < 12; index += 1) {
    const inverse = 1 - t;
    const bezierX =
      3 * inverse * inverse * t * 0.42 +
      3 * inverse * t * t * 0.58 +
      t * t * t;
    if (bezierX < x) low = t;
    else high = t;
    t = (low + high) / 2;
  }
  const inverse = 1 - t;
  return 3 * inverse * t * t + t * t * t;
}

function sampleKeyframes(table: number[][], progress: number, ease: boolean): number {
  'worklet';
  const percent = progress * 100;
  if (percent <= table[0][0]) return table[0][1];
  for (let index = 1; index < table.length; index += 1) {
    const [startPercent, startValue] = table[index - 1];
    const [endPercent, endValue] = table[index];
    if (percent <= endPercent) {
      const fraction =
        endPercent > startPercent
          ? (percent - startPercent) / (endPercent - startPercent)
          : 1;
      const interpolated = ease ? cssEaseInOut(fraction) : fraction;
      return startValue + (endValue - startValue) * interpolated;
    }
  }
  return table[table.length - 1][1];
}

export interface LineFrameValues {
  x: number;
  w: number;
  h: number;
  spike: number;
  spike2: number;
  edge: number;
}

export interface LineKeyframeTables {
  travel: { x: number[][]; w: number[][] };
  edgeFade: number[][];
  breathe: number[][];
  spike: number[][];
  spike2: number[][];
  durationScale: {
    travel: number;
    edgeFade: number;
    breathe: number;
    spike: number;
    spike2: number;
  };
}

export function lineFrameValues(
  tables: LineKeyframeTables,
  timeSeconds: number,
  duration: number,
): LineFrameValues {
  'worklet';
  const cycle = (period: number) => (timeSeconds / period) % 1;
  return {
    x: sampleKeyframes(tables.travel.x, cycle(duration * tables.durationScale.travel), false),
    w: sampleKeyframes(tables.travel.w, cycle(duration * tables.durationScale.travel), false),
    h: sampleKeyframes(tables.breathe, cycle(duration * tables.durationScale.breathe), true),
    spike: sampleKeyframes(tables.spike, cycle(duration * tables.durationScale.spike), true),
    spike2: sampleKeyframes(tables.spike2, cycle(duration * tables.durationScale.spike2), true),
    edge: sampleKeyframes(tables.edgeFade, cycle(duration * tables.durationScale.edgeFade), false),
  };
}

export function multiplier(name: string, values: LineFrameValues): number {
  'worklet';
  if (name === 'spike') return values.spike;
  if (name === 'spike2') return values.spike2;
  if (name === 'inv-spike') return 2 - values.spike;
  if (name === 'inv-spike2') return 2 - values.spike2;
  if (name === 'h') return values.h;
  if (name === 'w') return values.w;
  return 1;
}
