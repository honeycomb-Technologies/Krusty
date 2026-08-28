import { useCallback, useEffect, useRef, useState } from "react";
import { StyleSheet, useWindowDimensions, View } from "react-native";
import Animated, {
  cancelAnimation,
  runOnJS,
  useAnimatedStyle,
  useReducedMotion,
  useSharedValue,
  withTiming,
} from "react-native-reanimated";
import * as SplashScreen from "expo-splash-screen";

import { MitsuroTraceMark } from "../brand";

SplashScreen.preventAutoHideAsync();

const WEB_SPLASH_MS = 1600;
const EXIT_FADE_MS = 260;
const SPLASH_BACKGROUND = "#0e0e11";
const SPLASH_MARK = "#9d73ff";

interface Props {
  children: React.ReactNode;
  onComplete?: () => void;
}

export function SplashOverlay({ children, onComplete }: Props) {
  const [overlayVisible, setOverlayVisible] = useState(true);
  const completedRef = useRef(false);
  const reduceMotion = useReducedMotion();
  const overlayOpacity = useSharedValue(1);
  const { height, width } = useWindowDimensions();
  const markSize = Math.min(width * 0.55, height * 0.31);
  const overlayStyle = useAnimatedStyle(() => ({
    opacity: overlayOpacity.value,
  }));

  const completeSplash = useCallback(() => {
    if (completedRef.current) return;
    completedRef.current = true;
    onComplete?.();
    if (reduceMotion) {
      overlayOpacity.value = 0;
      setOverlayVisible(false);
      return;
    }
    overlayOpacity.value = withTiming(
      0,
      { duration: EXIT_FADE_MS },
      (finished) => {
        if (finished) {
          runOnJS(setOverlayVisible)(false);
        }
      },
    );
  }, [onComplete, overlayOpacity, reduceMotion]);

  useEffect(() => {
    SplashScreen.hideAsync();
    const timer = setTimeout(completeSplash, reduceMotion ? 0 : WEB_SPLASH_MS);
    return () => {
      clearTimeout(timer);
      cancelAnimation(overlayOpacity);
    };
  }, [completeSplash, overlayOpacity, reduceMotion]);

  return (
    <View style={styles.root}>
      <View style={StyleSheet.absoluteFill}>{children}</View>
      {overlayVisible
        ? (
          <Animated.View style={[styles.overlay, overlayStyle]}>
            <MitsuroTraceMark
              size={markSize}
              color={SPLASH_MARK}
              fill="transparent"
              duration={WEB_SPLASH_MS}
            />
          </Animated.View>
        )
        : null}
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    backgroundColor: SPLASH_BACKGROUND,
  },
  overlay: {
    ...StyleSheet.absoluteFillObject,
    alignItems: "center",
    justifyContent: "center",
    pointerEvents: "none",
    zIndex: 10,
    backgroundColor: SPLASH_BACKGROUND,
  },
});
