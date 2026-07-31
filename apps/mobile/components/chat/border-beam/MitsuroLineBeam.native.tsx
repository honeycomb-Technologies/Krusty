import { useMemo } from 'react';
import { StyleSheet } from 'react-native';
import {
  Blur,
  Canvas,
  Fill,
  Group,
  Paint,
  Shader,
  Skia,
} from '@shopify/react-native-skia';
import { useDerivedValue, type SharedValue } from 'react-native-reanimated';

import lineSpec from './line-spec.json';
import {
  BLOB_FLOATS,
  BLOB_LAYER_SKSL,
  MAX_LAYER_BLOBS,
  composedFilterMatrix,
  lineFrameValues,
  multiplier,
  padTo,
  parseCssColor,
  pingPong,
  simpleBlob,
  stopsBlob,
  type GradientStop,
  type LineKeyframeTables,
} from './line-runtime';

const blobEffect = Skia.RuntimeEffect.Make(BLOB_LAYER_SKSL);

interface LineBlobDefinition {
  color: { r: number; g: number; b: number; a: number };
  sizeW: number;
  sizeH: number;
  offsetX: number;
  offsetY: number;
}

interface BloomGradientDefinition {
  xPct: number | null;
  yOffPx: number;
  w: { base: number; mult: string };
  h: { base: number; mult: string };
  stops: GradientStop[];
}

export interface MitsuroLineBeamProps {
  width: number;
  height: number;
  borderRadius: number;
  theme: 'dark' | 'light';
  clock: SharedValue<number>;
  fade: SharedValue<number>;
  duration?: number;
  strength?: number;
  brightness?: number;
  saturation?: number;
  hueRange?: number;
}

/**
 * The border-beam line renderer, trimmed to Mitsuro's violet palette.
 * The rounded rectangle begins above the canvas so only the phone's lower edge
 * and corners are visible; the top edge can never flash through the composer.
 */
export function MitsuroLineBeam({
  width,
  height,
  borderRadius,
  theme,
  clock,
  fade,
  duration = lineSpec.defaults.duration.line,
  strength = 0.92,
  brightness = 1.18,
  saturation,
  hueRange = 8,
}: MitsuroLineBeamProps) {
  const preset = lineSpec.sizePresets.line;
  const colors = lineSpec.sizeThemePresets.line[theme];
  const finalSaturation = saturation ?? colors.saturation;
  const finalHueRange = Math.min(hueRange, lineSpec.defaults.lineHueRangeCap);
  const finalStrength = Math.max(0, Math.min(1, strength));
  const tables = lineSpec.line.keyframes as LineKeyframeTables;

  const staticData = useMemo(() => {
    const parsePalette = (
      entries: Array<{
        color: string;
        sizeW: number;
        sizeH: number;
        offsetX: number;
        offsetY: number;
      }>,
    ): LineBlobDefinition[] =>
      entries.map((entry) => ({
        color: parseCssColor(entry.color),
        sizeW: entry.sizeW,
        sizeH: entry.sizeH,
        offsetX: entry.offsetX,
        offsetY: entry.offsetY,
      }));

    return {
      strokeBlobs: parsePalette(lineSpec.palettes.line.violet[theme]),
      innerBlobs: parsePalette(lineSpec.palettes.lineInner.violet),
      bloom: lineSpec.line.bloomGradients.violet[
        theme
      ] as BloomGradientDefinition[],
      whiteHighlight: lineSpec.line.whiteHighlight[theme],
    };
  }, [theme]);

  const mask = lineSpec.line.beamMaskEllipse;
  const bloomMask = lineSpec.line.bloomMaskEllipse;

  // The virtual rounded rectangle extends one canvas-height above the visible
  // surface. This preserves the original lower-corner wrap without drawing a
  // second horizontal edge through the composer.
  const base = useMemo(
    () => ({
      uSize: [width, height],
      uRectOrigin: [0, -height],
      uRectSize: [width, height * 2],
      uRadius: borderRadius,
      uBorderWidth: preset.borderWidth,
    }),
    [borderRadius, height, preset.borderWidth, width],
  );

  const strokeUniforms = useDerivedValue(() => {
    'worklet';
    const time = clock.value / 1000;
    const values = lineFrameValues(tables, time, duration);
    const filter = composedFilterMatrix(
      -finalHueRange +
        2 *
          finalHueRange *
          pingPong(time / lineSpec.defaults.rotateHueShiftPeriod),
      brightness,
      finalSaturation,
    );
    const beamX = values.x * width;
    const blobs: number[] = [];
    const highlight = staticData.whiteHighlight;

    blobs.push(
      ...stopsBlob(
        highlight.w * values.w,
        highlight.h * values.h,
        beamX,
        height + highlight.yOffset,
        highlight.stops.map(([position, alpha]) => ({
          r: highlight.color[0],
          g: highlight.color[1],
          b: highlight.color[2],
          a: alpha,
          pos: position / 100,
        })),
      ),
    );
    for (const entry of staticData.strokeBlobs) {
      blobs.push(
        ...simpleBlob(
          entry.sizeW * values.w,
          entry.sizeH * values.h,
          beamX + entry.offsetX,
          height + entry.offsetY,
          entry.color.r,
          entry.color.g,
          entry.color.b,
          entry.color.a,
        ),
      );
    }

    return {
      ...base,
      uGeomKind: 0,
      uEdgeMaskPx: 0,
      uRadial: [
        beamX,
        height,
        mask.w * values.w,
        mask.h * values.h,
        mask.softStop[0] / 100,
        mask.softStop[1],
        1,
      ],
      uBlobs: padTo(blobs, MAX_LAYER_BLOBS * BLOB_FLOATS),
      uBlobCount: blobs.length / BLOB_FLOATS,
      uCM: filter,
      uOpacity: fade.value * values.edge * finalStrength * colors.strokeOpacity,
    };
  }, [
    base,
    brightness,
    colors.strokeOpacity,
    duration,
    finalHueRange,
    finalSaturation,
    finalStrength,
    height,
    mask,
    staticData,
    tables,
    width,
  ]);

  const innerUniforms = useDerivedValue(() => {
    'worklet';
    const time = clock.value / 1000;
    const values = lineFrameValues(tables, time, duration);
    const filter = composedFilterMatrix(
      -finalHueRange +
        2 *
          finalHueRange *
          pingPong(time / lineSpec.defaults.rotateHueShiftPeriod),
      brightness,
      finalSaturation,
    );
    const beamX = values.x * width;
    const blobs: number[] = [];
    for (const entry of staticData.innerBlobs) {
      blobs.push(
        ...simpleBlob(
          entry.sizeW * values.w,
          entry.sizeH * values.h,
          beamX + entry.offsetX,
          height - Math.abs(entry.offsetY),
          entry.color.r,
          entry.color.g,
          entry.color.b,
          entry.color.a,
        ),
      );
    }

    return {
      ...base,
      uGeomKind: 1,
      uEdgeMaskPx: lineSpec.rotate.innerEdgeMaskPx,
      uRadial: [
        beamX,
        height,
        mask.w * values.w,
        mask.h * values.h,
        mask.softStop[0] / 100,
        mask.softStop[1],
        1,
      ],
      uBlobs: padTo(blobs, MAX_LAYER_BLOBS * BLOB_FLOATS),
      uBlobCount: blobs.length / BLOB_FLOATS,
      uCM: filter,
      uOpacity: fade.value * values.edge * finalStrength * colors.innerOpacity,
    };
  }, [
    base,
    brightness,
    colors.innerOpacity,
    duration,
    finalHueRange,
    finalSaturation,
    finalStrength,
    height,
    mask,
    staticData,
    tables,
    width,
  ]);

  const bloomUniforms = useDerivedValue(() => {
    'worklet';
    const time = clock.value / 1000;
    const values = lineFrameValues(tables, time, duration);
    const bloomHueRange = finalHueRange + lineSpec.defaults.lineBloomHueRangeBonus;
    const filter = composedFilterMatrix(
      -bloomHueRange +
        2 *
          bloomHueRange *
          pingPong(time / lineSpec.defaults.lineBloomHueShiftPeriod),
      brightness,
      finalSaturation,
    );
    const beamX = values.x * width;
    const blobs: number[] = [];
    for (const entry of staticData.bloom) {
      const centerX =
        entry.xPct == null ? beamX : (entry.xPct / 100) * width;
      blobs.push(
        ...stopsBlob(
          entry.w.base * multiplier(entry.w.mult, values),
          entry.h.base * multiplier(entry.h.mult, values),
          centerX,
          height + entry.yOffPx,
          entry.stops,
        ),
      );
    }

    return {
      ...base,
      uGeomKind: 1,
      uEdgeMaskPx: 0,
      uRadial: [
        beamX,
        height,
        bloomMask.w * values.w,
        bloomMask.h * values.h,
        bloomMask.softStop[0] / 100,
        bloomMask.softStop[1],
        1,
      ],
      uBlobs: padTo(blobs, MAX_LAYER_BLOBS * BLOB_FLOATS),
      uBlobCount: blobs.length / BLOB_FLOATS,
      uCM: filter,
      uOpacity: fade.value * values.edge * finalStrength * colors.bloomOpacity,
    };
  }, [
    base,
    bloomMask,
    brightness,
    colors.bloomOpacity,
    duration,
    finalHueRange,
    finalSaturation,
    finalStrength,
    height,
    staticData,
    tables,
    width,
  ]);

  if (!blobEffect || width <= 0 || height <= 0) return null;

  return (
    <Canvas pointerEvents="none" style={StyleSheet.absoluteFill}>
      <Fill>
        <Shader source={blobEffect} uniforms={innerUniforms} />
      </Fill>
      <Fill>
        <Shader source={blobEffect} uniforms={strokeUniforms} />
      </Fill>
      <Group
        layer={
          <Paint>
            <Blur blur={lineSpec.line.bloomBlurPx} />
          </Paint>
        }
      >
        <Fill>
          <Shader source={blobEffect} uniforms={bloomUniforms} />
        </Fill>
      </Group>
    </Canvas>
  );
}
