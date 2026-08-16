import { useEffect } from 'react';
import { useSharedValue, useAnimatedStyle, withDelay, withTiming, Easing } from 'react-native-reanimated';

const DURATION = 500;
const EASE = Easing.out(Easing.cubic);

export function useEntranceAnimation(ready: boolean) {
  const topBarY = useSharedValue(-40);
  const contentScale = useSharedValue(0.97);
  const bottomBarY = useSharedValue(60);

  useEffect(() => {
    if (!ready) return;

    // Slide/scale only. Never animate opacity on these wrappers: iOS
    // UIVisualEffectView / liquid glass will not sample through an ancestor
    // with a Reanimated alpha, even after that alpha returns to 1.
    topBarY.value = withDelay(0, withTiming(0, { duration: DURATION, easing: EASE }));
    contentScale.value = withDelay(80, withTiming(1, { duration: DURATION, easing: EASE }));
    bottomBarY.value = withDelay(120, withTiming(0, { duration: DURATION, easing: EASE }));
  }, [ready]);

  const topBarStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: topBarY.value }],
  }));

  const contentStyle = useAnimatedStyle(() => ({
    transform: [{ scale: contentScale.value }],
  }));

  const bottomBarStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: bottomBarY.value }],
  }));

  return { topBarStyle, contentStyle, bottomBarStyle };
}
