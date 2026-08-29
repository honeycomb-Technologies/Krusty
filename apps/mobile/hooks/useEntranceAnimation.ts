import { useCallback, useEffect, useState } from 'react';
import {
  Easing,
  runOnJS,
  useAnimatedStyle,
  useSharedValue,
  withDelay,
  withTiming,
} from 'react-native-reanimated';

const DURATION = 500;
const EASE = Easing.out(Easing.cubic);

export function useEntranceAnimation(ready: boolean) {
  const topBarY = useSharedValue(-40);
  const contentScale = useSharedValue(0.97);
  const bottomBarY = useSharedValue(60);
  const [settled, setSettled] = useState(false);
  const [materialSafe, setMaterialSafe] = useState(false);

  const markSettled = useCallback(() => setSettled(true), []);

  useEffect(() => {
    if (!settled) {
      setMaterialSafe(false);
      return;
    }

    // First commit removes every entrance transform. Native glass is allowed
    // only on the following paint, so it never observes the transformed tree.
    const safeFrame = requestAnimationFrame(() => setMaterialSafe(true));
    return () => cancelAnimationFrame(safeFrame);
  }, [settled]);

  useEffect(() => {
    if (!ready) {
      setSettled(false);
      setMaterialSafe(false);
      return;
    }

    setSettled(false);

    // Slide/scale only. Never animate opacity on these wrappers: iOS
    // UIVisualEffectView / liquid glass will not sample through an ancestor
    // with a Reanimated alpha, even after that alpha returns to 1.
    topBarY.value = withDelay(
      0,
      withTiming(0, { duration: DURATION, easing: EASE }),
    );
    contentScale.value = withDelay(
      80,
      withTiming(1, { duration: DURATION, easing: EASE }),
    );
    bottomBarY.value = withDelay(
      120,
      withTiming(0, { duration: DURATION, easing: EASE }, (finished) => {
        if (finished) runOnJS(markSettled)();
      }),
    );
  }, [markSettled, ready]);

  const topBarStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: topBarY.value }],
  }));

  const contentStyle = useAnimatedStyle(() => ({
    transform: [{ scale: contentScale.value }],
  }));

  const bottomBarStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: bottomBarY.value }],
  }));

  return { topBarStyle, contentStyle, bottomBarStyle, settled, materialSafe };
}
