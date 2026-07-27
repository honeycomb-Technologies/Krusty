import {
  BackHandler,
  Platform,
  Pressable,
  StyleSheet,
  View,
  useWindowDimensions,
  type StyleProp,
  type ViewStyle,
} from "react-native";
import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import Animated, {
  Easing,
  interpolate,
  runOnJS,
  useAnimatedStyle,
  useSharedValue,
  withSpring,
  withTiming,
} from "react-native-reanimated";

import { useThemeContext } from "../../hooks/useTheme";
import { BlurView } from "../../platform/blur";
import * as Haptics from "../../platform/haptics";
import { resolveAppBottomSheetHeight } from "./sheetMetrics";

const SPRING = { damping: 24, stiffness: 300, mass: 0.82 };
const CLOSE_DISTANCE = 56;
const CLOSE_VELOCITY = 650;

interface AppBottomSheetProps {
  visible: boolean;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  accessibilityLabel: string;
  contentStyle?: StyleProp<ViewStyle>;
  testID?: string;
}

export function AppBottomSheet({
  visible,
  onClose,
  children,
  footer,
  accessibilityLabel,
  contentStyle,
  testID,
}: AppBottomSheetProps) {
  const { theme } = useThemeContext();
  const { height: windowHeight } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const [mounted, setMounted] = useState(visible);
  const progress = useSharedValue(visible ? 1 : 0);
  const dragOffset = useSharedValue(0);
  const sheetHeight = resolveAppBottomSheetHeight(windowHeight, insets.top);
  const t = theme.colors;

  useEffect(() => {
    if (visible) {
      setMounted(true);
      dragOffset.value = 0;
      progress.value = withSpring(1, SPRING);
      return;
    }

    progress.value = withTiming(0, {
      duration: 210,
      easing: Easing.out(Easing.cubic),
    });
    const timer = setTimeout(() => {
      dragOffset.value = 0;
      setMounted(false);
    }, 230);
    return () => clearTimeout(timer);
  }, [dragOffset, progress, visible]);

  const close = useCallback(() => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onClose();
  }, [onClose]);

  useEffect(() => {
    if (!visible) {
      return;
    }

    if (Platform.OS === "web") {
      const handleKeyDown = (event: KeyboardEvent) => {
        if (event.key === "Escape") {
          close();
        }
      };
      document.addEventListener("keydown", handleKeyDown);
      return () => document.removeEventListener("keydown", handleKeyDown);
    }

    const subscription = BackHandler.addEventListener(
      "hardwareBackPress",
      () => {
        close();
        return true;
      },
    );
    return () => subscription.remove();
  }, [close, visible]);

  const panelStyle = useAnimatedStyle(() => {
    const downwardDrag = Math.max(0, Math.min(dragOffset.value, sheetHeight));
    const translateY =
      interpolate(progress.value, [0, 1], [sheetHeight, 0]) + downwardDrag;
    return {
      height: sheetHeight,
      opacity: progress.value,
      transform: [{ translateY }],
    };
  });

  const backdropStyle = useAnimatedStyle(() => ({
    opacity: interpolate(progress.value, [0, 1], [0, 1]),
    pointerEvents:
      progress.value > 0.05 ? ("auto" as const) : ("none" as const),
  }));

  const dragGesture = Gesture.Pan()
    .activeOffsetY([-10, 10])
    .failOffsetX([-24, 24])
    .onUpdate((event) => {
      dragOffset.value = Math.max(0, event.translationY);
    })
    .onEnd((event) => {
      if (
        event.translationY > CLOSE_DISTANCE ||
        event.velocityY > CLOSE_VELOCITY
      ) {
        runOnJS(close)();
        return;
      }
      dragOffset.value = withSpring(0, SPRING);
    });

  if (!mounted) {
    return null;
  }

  const isDark = theme.scheme === "dark";

  return (
    <View style={styles.root} pointerEvents="box-none">
      <Animated.View style={[styles.backdrop, backdropStyle]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Close ${accessibilityLabel}`}
          style={StyleSheet.absoluteFill}
          onPress={close}
        />
      </Animated.View>

      <Animated.View
        testID={testID}
        accessibilityViewIsModal
        accessibilityLabel={accessibilityLabel}
        style={[
          styles.sheet,
          panelStyle,
          {
            borderColor: t.border,
            paddingBottom: Math.max(insets.bottom, 8),
          },
        ]}
      >
        <BlurView
          intensity={52}
          tint={
            isDark ? "systemChromeMaterialDark" : "systemChromeMaterialLight"
          }
          style={StyleSheet.absoluteFill}
        />
        <View
          pointerEvents="none"
          style={[
            StyleSheet.absoluteFill,
            {
              backgroundColor: isDark
                ? "rgba(11,17,25,0.94)"
                : "rgba(255,255,255,0.95)",
            },
          ]}
        />

        <GestureDetector gesture={dragGesture}>
          <Animated.View
            accessible
            accessibilityRole="adjustable"
            accessibilityLabel={`Drag down to close ${accessibilityLabel}`}
            style={styles.handleZone}
          >
            <View
              style={[
                styles.handle,
                { backgroundColor: t.mutedForeground },
              ]}
            />
          </Animated.View>
        </GestureDetector>

        <View style={[styles.content, contentStyle]}>{children}</View>
        {footer ? <View>{footer}</View> : null}
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    ...StyleSheet.absoluteFillObject,
    zIndex: 700,
  },
  backdrop: {
    ...StyleSheet.absoluteFillObject,
    zIndex: 0,
    backgroundColor: "rgba(0,0,0,0.46)",
  },
  sheet: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
    zIndex: 1,
    overflow: "hidden",
    borderTopLeftRadius: 24,
    borderTopRightRadius: 24,
    borderWidth: StyleSheet.hairlineWidth,
    borderBottomWidth: 0,
  },
  handleZone: {
    height: 28,
    alignItems: "center",
    justifyContent: "center",
    zIndex: 3,
  },
  handle: {
    width: 44,
    height: 5,
    borderRadius: 999,
    opacity: 0.46,
  },
  content: {
    flex: 1,
    minHeight: 0,
  },
});
