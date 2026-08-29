import {
  type SharedValue,
  useSharedValue,
} from 'react-native-reanimated';
import { useMemo } from 'react';

import type { GooeyProgresses } from './fabGooey';

function useSixProgresses(): GooeyProgresses {
  const p0 = useSharedValue(0);
  const p1 = useSharedValue(0);
  const p2 = useSharedValue(0);
  const p3 = useSharedValue(0);
  const p4 = useSharedValue(0);
  const p5 = useSharedValue(0);
  return useMemo(
    () => [p0, p1, p2, p3, p4, p5],
    [p0, p1, p2, p3, p4, p5],
  );
}

/**
 * One motion authority shared by React Native controls, the Skia fallback and
 * the optional iOS Liquid Glass host. Keeping these identities in ChatBar
 * prevents a native surface from chasing a second animation clock.
 */
export interface FabGlassMotion {
  pillProgresses: GooeyProgresses;
  attachmentProgresses: GooeyProgresses;
  providerProgresses: GooeyProgresses;
  providerReorderX: GooeyProgresses;
  providerDragging: GooeyProgresses;
  providerScrollX: SharedValue<number>;
  providerViewportClip: SharedValue<number>;
  providerEditProgress: SharedValue<number>;
  providerDragIndex: SharedValue<number>;
  providerDropIndex: SharedValue<number>;
  providerDragX: SharedValue<number>;
  providerDragScrollDelta: SharedValue<number>;
}

export function useFabGlassMotion(): FabGlassMotion {
  const pillProgresses = useSixProgresses();
  const attachmentProgresses = useSixProgresses();
  const providerProgresses = useSixProgresses();
  const providerReorderX = useSixProgresses();
  const providerDragging = useSixProgresses();
  const providerScrollX = useSharedValue(0);
  const providerViewportClip = useSharedValue(0);
  const providerEditProgress = useSharedValue(0);
  const providerDragIndex = useSharedValue(-1);
  const providerDropIndex = useSharedValue(-1);
  const providerDragX = useSharedValue(0);
  const providerDragScrollDelta = useSharedValue(0);

  return useMemo(
    () => ({
      pillProgresses,
      attachmentProgresses,
      providerProgresses,
      providerReorderX,
      providerDragging,
      providerScrollX,
      providerViewportClip,
      providerEditProgress,
      providerDragIndex,
      providerDropIndex,
      providerDragX,
      providerDragScrollDelta,
    }),
    [
      attachmentProgresses,
      pillProgresses,
      providerDragIndex,
      providerDragging,
      providerDragScrollDelta,
      providerDragX,
      providerDropIndex,
      providerEditProgress,
      providerProgresses,
      providerReorderX,
      providerScrollX,
      providerViewportClip,
    ],
  );
}
