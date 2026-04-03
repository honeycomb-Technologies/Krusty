import { useEffect } from 'react';
import { useSharedValue, useAnimatedStyle, withDelay, withTiming, Easing } from 'react-native-reanimated';

const DURATION = 500;
const EASE = Easing.out(Easing.cubic);

export function useEntranceAnimation(ready: boolean) {
  const topBarY = useSharedValue(-40);
  const topBarOpacity = useSharedValue(0);

  const contentOpacity = useSharedValue(0);
  const contentScale = useSharedValue(0.97);

  const bottomBarY = useSharedValue(60);
  const bottomBarOpacity = useSharedValue(0);

  useEffect(() => {
    if (!ready) return;

    // Top bar slides down
    topBarY.value = withDelay(0, withTiming(0, { duration: DURATION, easing: EASE }));
    topBarOpacity.value = withDelay(0, withTiming(1, { duration: DURATION }));

    // Content fades in with subtle scale
    contentOpacity.value = withDelay(80, withTiming(1, { duration: DURATION }));
    contentScale.value = withDelay(80, withTiming(1, { duration: DURATION, easing: EASE }));

    // Bottom bar slides up
    bottomBarY.value = withDelay(120, withTiming(0, { duration: DURATION, easing: EASE }));
    bottomBarOpacity.value = withDelay(120, withTiming(1, { duration: DURATION }));
  }, [ready]);

  const topBarStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: topBarY.value }],
    opacity: topBarOpacity.value,
  }));

  const contentStyle = useAnimatedStyle(() => ({
    opacity: contentOpacity.value,
    transform: [{ scale: contentScale.value }],
  }));

  const bottomBarStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: bottomBarY.value }],
    opacity: bottomBarOpacity.value,
  }));

  return { topBarStyle, contentStyle, bottomBarStyle };
}
