import { memo, useEffect, useState } from 'react';
import {
  AppState,
  StyleSheet,
  View,
  type AppStateStatus,
  type StyleProp,
  type ViewStyle,
} from 'react-native';
import { useClock } from '@shopify/react-native-skia';
import {
  cancelAnimation,
  Easing,
  useReducedMotion,
  useSharedValue,
  withTiming,
  type SharedValue,
} from 'react-native-reanimated';

import { KrustyLineBeam } from './border-beam/KrustyLineBeam.native';

const RUN_LINE_MIN_HEIGHT = 18;
const RUN_LINE_CORNER_OVERDRAW = 18;
const RUN_LINE_FADE_IN_MS = 260;
const RUN_LINE_FADE_OUT_MS = 180;
const RUN_LINE_DURATION_SECONDS = 3.1;

/** Shared with ChatBar for non-desktop corner climb. */
export const RUN_LINE_CORNER_CLIMB = 35;

export interface ChatBarRunningLineProps {
  active: boolean;
  width: number;
  cornerClimb: number;
  theme?: 'dark' | 'light';
  style?: StyleProp<ViewStyle>;
}

interface ActiveLineBeamProps {
  width: number;
  height: number;
  borderRadius: number;
  theme: 'dark' | 'light';
  reduceMotion: boolean;
  fade: SharedValue<number>;
}

function ActiveLineBeam({
  width,
  height,
  borderRadius,
  theme,
  reduceMotion,
  fade,
}: ActiveLineBeamProps) {
  const clock = useClock();
  const frozenClock = useSharedValue((RUN_LINE_DURATION_SECONDS * 1000) / 2);

  return (
    <KrustyLineBeam
      width={width}
      height={height}
      borderRadius={borderRadius}
      theme={theme}
      duration={RUN_LINE_DURATION_SECONDS}
      strength={theme === 'dark' ? 0.92 : 0.78}
      brightness={theme === 'dark' ? 1.18 : 1.04}
      saturation={theme === 'dark' ? 1.26 : 1.45}
      hueRange={8}
      clock={reduceMotion ? frozenClock : clock}
      fade={fade}
    />
  );
}

function ChatBarRunningLineComponent({
  active,
  width,
  cornerClimb,
  theme = 'dark',
  style,
}: ChatBarRunningLineProps) {
  const reduceMotion = useReducedMotion();
  const [appState, setAppState] = useState<AppStateStatus>(AppState.currentState);
  const [mounted, setMounted] = useState(active && AppState.currentState === 'active');
  const fade = useSharedValue(0);
  const shouldShow = active && appState === 'active';

  useEffect(() => {
    const subscription = AppState.addEventListener('change', setAppState);
    return () => subscription.remove();
  }, []);

  useEffect(() => {
    let unmountTimer: ReturnType<typeof setTimeout> | null = null;
    cancelAnimation(fade);
    if (shouldShow) {
      setMounted(true);
      fade.value = withTiming(1, {
        duration: reduceMotion ? 0 : RUN_LINE_FADE_IN_MS,
        easing: Easing.out(Easing.cubic),
      });
    } else {
      fade.value = withTiming(0, {
        duration: reduceMotion ? 0 : RUN_LINE_FADE_OUT_MS,
        easing: Easing.in(Easing.cubic),
      });
      unmountTimer = setTimeout(
        () => setMounted(false),
        reduceMotion ? 0 : RUN_LINE_FADE_OUT_MS,
      );
    }

    return () => {
      if (unmountTimer) clearTimeout(unmountTimer);
      cancelAnimation(fade);
    };
  }, [fade, reduceMotion, shouldShow]);

  if (!mounted || width <= 0) return null;

  const height = Math.max(
    RUN_LINE_MIN_HEIGHT,
    cornerClimb + RUN_LINE_CORNER_OVERDRAW,
  );
  const borderRadius = cornerClimb > 0 ? cornerClimb + 9 : 0;

  return (
    <View
      pointerEvents="none"
      style={[styles.track, { width, height }, style]}
    >
      <ActiveLineBeam
        width={width}
        height={height}
        borderRadius={borderRadius}
        theme={theme}
        reduceMotion={reduceMotion}
        fade={fade}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  track: {
    overflow: 'visible',
  },
});

export const ChatBarRunningLine = memo(ChatBarRunningLineComponent);
