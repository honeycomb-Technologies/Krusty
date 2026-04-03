import { useEffect } from 'react';
import { View, StyleSheet } from 'react-native';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withTiming,
  withSpring,
  withRepeat,
  withSequence,
  withDelay,
  Easing,
} from 'react-native-reanimated';
import { useThemeContext } from '../../hooks/useTheme';

const BAR_COUNT = 40;
const BAR_WIDTH = 2.5;
const BAR_GAP = 1.5;
const MAX_HEIGHT = 32;
const MIN_HEIGHT = 3;

interface WaveformProps {
  active: boolean;
  volume?: number; // 0.0 - 1.0 amplitude from mic
}

function WaveBar({ index, active, volume = 0 }: { index: number; active: boolean; volume: number }) {
  const { theme } = useThemeContext();
  const height = useSharedValue(MIN_HEIGHT);

  // Each bar gets a slightly different response to create the equalizer effect
  const sensitivity = 0.5 + Math.sin(index * 0.7) * 0.3 + Math.cos(index * 1.3) * 0.2;
  // Center bars are taller, edges taper off
  const center = BAR_COUNT / 2;
  const distFromCenter = Math.abs(index - center) / center;
  const taper = 1 - distFromCenter * 0.6;

  useEffect(() => {
    if (active && volume > 0.01) {
      const amplitude = volume * sensitivity * taper;
      const target = MIN_HEIGHT + amplitude * (MAX_HEIGHT - MIN_HEIGHT);
      height.value = withSpring(target, { damping: 12, stiffness: 400, mass: 0.3 });
    } else if (active) {
      // Continuous idle breathing — staggered per bar for organic wave
      const base = MIN_HEIGHT + taper * 4;
      const amp = taper * 2.5;
      const dur = 600 + index * 15;
      height.value = withDelay(
        index * 25,
        withRepeat(
          withSequence(
            withTiming(base + amp, { duration: dur, easing: Easing.inOut(Easing.sin) }),
            withTiming(base - amp * 0.3, { duration: dur, easing: Easing.inOut(Easing.sin) }),
          ),
          -1,
        ),
      );
    } else {
      height.value = withTiming(MIN_HEIGHT, { duration: 200, easing: Easing.out(Easing.cubic) });
    }
  }, [active, volume]);

  const animStyle = useAnimatedStyle(() => ({
    height: height.value,
  }));

  const color = active
    ? theme.colors.userMessage + (volume > 0.3 ? 'ff' : volume > 0.1 ? 'cc' : '88')
    : theme.colors.mutedForeground + '30';

  return (
    <Animated.View
      style={[styles.bar, { backgroundColor: color }, animStyle]}
    />
  );
}

export function Waveform({ active, volume = 0 }: WaveformProps) {
  return (
    <View style={styles.container}>
      {Array.from({ length: BAR_COUNT }, (_, i) => (
        <WaveBar key={i} index={i} active={active} volume={volume} />
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    gap: BAR_GAP,
    height: MAX_HEIGHT + 8,
    paddingHorizontal: 2,
  },
  bar: {
    width: BAR_WIDTH,
    borderRadius: BAR_WIDTH / 2,
    minHeight: MIN_HEIGHT,
  },
});
