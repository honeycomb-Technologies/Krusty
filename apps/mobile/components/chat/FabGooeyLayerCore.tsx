import { useMemo } from 'react';
import { Canvas, Fill, Shader, Skia } from '@shopify/react-native-skia';
import { useDerivedValue } from 'react-native-reanimated';

import {
  FAB_PILL,
  FAB_RADIUS,
  FAB_STEP,
  GOOEY_PAD,
  GOOEY_SKSL,
  GOOEY_SMIN,
  gooeyCanvasHeight,
  gooeyCanvasWidth,
  parseGooeyFill,
  type GooeyProgresses,
} from './fabGooey';

function makeGooeyEffect() {
  try {
    return Skia.RuntimeEffect.Make(GOOEY_SKSL);
  } catch {
    return null;
  }
}

const gooeyEffect = makeGooeyEffect();

export function FabGooeyLayer({
  progresses,
  pillCount,
  fill,
}: {
  progresses: GooeyProgresses;
  pillCount: number;
  fill: string;
}) {
  const width = gooeyCanvasWidth();
  const height = gooeyCanvasHeight(pillCount);
  const color = useMemo(() => parseGooeyFill(fill), [fill]);
  const p0 = progresses[0];
  const p1 = progresses[1];
  const p2 = progresses[2];
  const p3 = progresses[3];
  const p4 = progresses[4];
  const p5 = progresses[5];
  const uniforms = useDerivedValue(() => {
    'worklet';
    return {
      u_resolution: [width, height],
      u_pad: GOOEY_PAD,
      u_pill: FAB_PILL,
      u_radius: FAB_RADIUS,
      u_step: FAB_STEP,
      u_count: pillCount,
      u_smin: GOOEY_SMIN,
      u_color: [color[0], color[1], color[2], color[3]],
      u_p0: p0.value,
      u_p1: p1.value,
      u_p2: p2.value,
      u_p3: p3.value,
      u_p4: p4.value,
      u_p5: p5.value,
    };
  }, [color, height, p0, p1, p2, p3, p4, p5, pillCount, width]);

  if (!gooeyEffect || width <= 0 || height <= 0) return null;

  return (
    <Canvas pointerEvents="none" style={{ width, height }}>
      <Fill>
        <Shader source={gooeyEffect} uniforms={uniforms} />
      </Fill>
    </Canvas>
  );
}
