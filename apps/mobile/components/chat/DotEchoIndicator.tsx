import { useEffect } from "react";
import { StyleSheet, View } from "react-native";
import Animated, {
  Easing,
  cancelAnimation,
  interpolate,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withDelay,
  withRepeat,
  withSequence,
  withTiming,
} from "react-native-reanimated";

const ECHO_DELAYS_MS = [240, 120, 60, 0, 60, 120, 240] as const;

interface DotEchoIndicatorProps {
  color: string;
}

export function DotEchoIndicator({ color }: DotEchoIndicatorProps) {
  return (
    <View
      pointerEvents="none"
      style={styles.row}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {ECHO_DELAYS_MS.map((delayMs, index) => (
        <EchoDot
          key={`${delayMs}-${index}`}
          color={color}
          delayMs={delayMs}
        />
      ))}
    </View>
  );
}

function EchoDot({ color, delayMs }: { color: string; delayMs: number }) {
  const progress = useSharedValue(0);
  const reduceMotion = useReducedMotion();

  useEffect(() => {
    if (reduceMotion) {
      progress.value = 0.72;
      return;
    }

    progress.value = withDelay(
      delayMs,
      withRepeat(
        withSequence(
          withTiming(1, {
            duration: 460,
            easing: Easing.inOut(Easing.quad),
          }),
          withTiming(0, {
            duration: 690,
            easing: Easing.inOut(Easing.quad),
          }),
        ),
        -1,
        false,
      ),
    );

    return () => cancelAnimation(progress);
  }, [delayMs, progress, reduceMotion]);

  const animatedStyle = useAnimatedStyle(() => ({
    opacity: interpolate(progress.value, [0, 1], [0.34, 1]),
    transform: [
      { scale: interpolate(progress.value, [0, 1], [0.55, 1]) },
    ],
  }));

  return (
    <Animated.View
      style={[styles.dot, { backgroundColor: color }, animatedStyle]}
    />
  );
}

const styles = StyleSheet.create({
  row: {
    width: 28,
    height: 14,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  dot: {
    width: 3,
    height: 3,
    borderRadius: 2,
  },
});
