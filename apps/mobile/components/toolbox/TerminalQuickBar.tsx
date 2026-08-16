import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Keyboard,
  Platform,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { ClipboardPaste } from "lucide-react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, {
  runOnJS,
  useAnimatedStyle,
  useSharedValue,
  withSpring,
  withTiming,
} from "react-native-reanimated";

import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";

interface TerminalQuickBarProps {
  disabled?: boolean;
  onInput: (data: string) => void;
  onPaste: () => void | Promise<void>;
  onRefocus?: () => void;
}

interface TerminalQuickKey {
  id: string;
  label: string;
  accessibilityLabel: string;
  data: string;
}

export const TERMINAL_QUICK_KEYS: readonly TerminalQuickKey[] = [
  {
    id: "interrupt",
    label: "^C",
    accessibilityLabel: "Send Control C",
    data: "\u0003",
  },
  {
    id: "escape",
    label: "esc",
    accessibilityLabel: "Send Escape",
    data: "\u001b",
  },
  { id: "tab", label: "⇥", accessibilityLabel: "Send Tab", data: "\t" },
  {
    id: "enter",
    label: "↵",
    accessibilityLabel: "Send Enter",
    data: "\r",
  },
  {
    id: "clear",
    label: "^L",
    accessibilityLabel: "Clear terminal",
    data: "\u000c",
  },
];

type TerminalDirection = "up" | "down" | "left" | "right";

const TERMINAL_DIRECTION_KEYS: Record<TerminalDirection, string> = {
  up: "\u001b[A",
  down: "\u001b[B",
  left: "\u001b[D",
  right: "\u001b[C",
};

const TERMINAL_DIRECTIONS: readonly TerminalDirection[] = [
  "up",
  "right",
  "down",
  "left",
];
const DIRECTION_PUCK_SIZE = 48;
const DIRECTION_TRAVEL = 9;
const DIRECTION_DEAD_ZONE = 5;
const DIRECTION_HOLD_DELAY_MS = 320;
const DIRECTION_REPEAT_MS = 150;
const TERMINAL_OVERLAY_OFFSET = 10;
const DIRECTION_SPRING = {
  damping: 20,
  stiffness: 320,
  mass: 0.45,
  overshootClamping: true,
} as const;

function directionCodeFromVector(dx: number, dy: number): number {
  "worklet";
  if (Math.max(Math.abs(dx), Math.abs(dy)) < DIRECTION_DEAD_ZONE) return -1;
  if (Math.abs(dx) > Math.abs(dy)) return dx > 0 ? 1 : 3;
  return dy > 0 ? 2 : 0;
}

function TerminalDirectionPad({
  disabled,
  onInput,
  onFinish,
}: {
  disabled: boolean;
  onInput: (data: string) => void;
  onFinish: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [activeDirection, setActiveDirection] = useState<
    TerminalDirection | null
  >(
    null,
  );
  const heldDirectionRef = useRef<TerminalDirection | null>(null);
  const repeatTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const knobX = useSharedValue(0);
  const knobY = useSharedValue(0);
  const pressed = useSharedValue(0);
  const tickPulse = useSharedValue(0);
  const activeDirectionCode = useSharedValue(-1);

  const stopRepeat = useCallback(() => {
    if (repeatTimerRef.current) clearTimeout(repeatTimerRef.current);
    repeatTimerRef.current = null;
  }, []);

  const pulseTick = useCallback(() => {
    tickPulse.value = 1;
    tickPulse.value = withTiming(0, { duration: 130 });
  }, [tickPulse]);

  const emitDirection = useCallback((direction: TerminalDirection) => {
    pulseTick();
    onInput(TERMINAL_DIRECTION_KEYS[direction]);
  }, [onInput, pulseTick]);

  const scheduleRepeat = useCallback((delay: number) => {
    stopRepeat();
    const tick = () => {
      const direction = heldDirectionRef.current;
      if (!direction) return;
      emitDirection(direction);
      repeatTimerRef.current = setTimeout(tick, DIRECTION_REPEAT_MS);
    };
    repeatTimerRef.current = setTimeout(tick, delay);
  }, [emitDirection, stopRepeat]);

  const selectDirectionCode = useCallback((code: number) => {
    const direction = TERMINAL_DIRECTIONS[code] ?? null;
    if (disabled) return;
    if (!direction) {
      stopRepeat();
      heldDirectionRef.current = null;
      setActiveDirection(null);
      return;
    }
    if (heldDirectionRef.current === direction) return;

    heldDirectionRef.current = direction;
    setActiveDirection(direction);
    stopRepeat();
    emitDirection(direction);
    scheduleRepeat(DIRECTION_HOLD_DELAY_MS);
    if (Platform.OS === "ios") {
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    }
  }, [disabled, emitDirection, scheduleRepeat, stopRepeat]);

  const finishGesture = useCallback(() => {
    stopRepeat();
    heldDirectionRef.current = null;
    setActiveDirection(null);
    onFinish();
  }, [onFinish, stopRepeat]);

  useEffect(() => {
    if (!disabled) return;
    stopRepeat();
    heldDirectionRef.current = null;
    setActiveDirection(null);
  }, [disabled, stopRepeat]);

  useEffect(() => stopRepeat, [stopRepeat]);

  const gesture = useMemo(() =>
    Gesture.Pan()
      .enabled(!disabled)
      .minDistance(0)
      .shouldCancelWhenOutside(false)
      .onBegin((event) => {
        pressed.value = 1;
        const dx = event.x - DIRECTION_PUCK_SIZE / 2;
        const dy = event.y - DIRECTION_PUCK_SIZE / 2;
        const distance = Math.max(1, Math.hypot(dx, dy));
        const travel = Math.min(DIRECTION_TRAVEL, distance);
        knobX.value = dx / distance * travel;
        knobY.value = dy / distance * travel;
        const code = directionCodeFromVector(dx, dy);
        activeDirectionCode.value = code;
        if (code >= 0) runOnJS(selectDirectionCode)(code);
      })
      .onUpdate((event) => {
        const dx = event.x - DIRECTION_PUCK_SIZE / 2;
        const dy = event.y - DIRECTION_PUCK_SIZE / 2;
        const distance = Math.max(1, Math.hypot(dx, dy));
        const travel = Math.min(DIRECTION_TRAVEL, distance);
        knobX.value = dx / distance * travel;
        knobY.value = dy / distance * travel;
        const code = directionCodeFromVector(dx, dy);
        if (code !== activeDirectionCode.value) {
          activeDirectionCode.value = code;
          runOnJS(selectDirectionCode)(code);
        }
      })
      .onFinalize(() => {
        pressed.value = 0;
        activeDirectionCode.value = -1;
        knobX.value = withSpring(0, DIRECTION_SPRING);
        knobY.value = withSpring(0, DIRECTION_SPRING);
        runOnJS(finishGesture)();
      }), [
    activeDirectionCode,
    disabled,
    finishGesture,
    knobX,
    knobY,
    pressed,
    selectDirectionCode,
  ]);

  const puckStyle = useAnimatedStyle(() => ({
    transform: [{ scale: 1 + pressed.value * 0.035 }],
  }));

  const knobStyle = useAnimatedStyle(() => ({
    opacity: 0.72 + pressed.value * 0.28,
    transform: [
      { translateX: knobX.value },
      { translateY: knobY.value },
      { scale: 1 + tickPulse.value * 0.12 },
    ],
  }));

  return (
    <GestureDetector gesture={gesture}>
      <Animated.View
        accessible
        accessibilityRole="adjustable"
        accessibilityLabel="Terminal directional control"
        accessibilityHint="Tap a side, hold to repeat, or drag in a direction"
        style={[
          styles.directionPad,
          { backgroundColor: t.surfaceOverlay, opacity: disabled ? 0.45 : 1 },
          puckStyle,
        ]}
      >
        <Text
          style={[
            styles.directionCue,
            styles.directionUp,
            {
              color: activeDirection === "up"
                ? t.userMessage
                : t.mutedForeground,
            },
          ]}
        >
          ↑
        </Text>
        <Text
          style={[
            styles.directionCue,
            styles.directionDown,
            {
              color: activeDirection === "down"
                ? t.userMessage
                : t.mutedForeground,
            },
          ]}
        >
          ↓
        </Text>
        <Text
          style={[
            styles.directionCue,
            styles.directionLeft,
            {
              color: activeDirection === "left"
                ? t.userMessage
                : t.mutedForeground,
            },
          ]}
        >
          ←
        </Text>
        <Text
          style={[
            styles.directionCue,
            styles.directionRight,
            {
              color: activeDirection === "right"
                ? t.userMessage
                : t.mutedForeground,
            },
          ]}
        >
          →
        </Text>
        <Animated.View
          style={[
            styles.directionKnob,
            { backgroundColor: t.userMessage },
            knobStyle,
          ]}
        />
      </Animated.View>
    </GestureDetector>
  );
}

function useTerminalKeyboardInset(): number {
  const [inset, setInset] = useState(0);

  useEffect(() => {
    if (Platform.OS === "web") {
      const viewport = window.visualViewport;
      if (!viewport) return;
      const update = () => {
        const coveredHeight = window.innerHeight -
          (viewport.height + viewport.offsetTop);
        setInset(Math.max(0, Math.min(coveredHeight, 420)));
      };
      update();
      viewport.addEventListener("resize", update);
      viewport.addEventListener("scroll", update);
      return () => {
        viewport.removeEventListener("resize", update);
        viewport.removeEventListener("scroll", update);
      };
    }

    const showEvent = Platform.OS === "ios"
      ? "keyboardWillShow"
      : "keyboardDidShow";
    const hideEvent = Platform.OS === "ios"
      ? "keyboardWillHide"
      : "keyboardDidHide";
    const showSubscription = Keyboard.addListener(showEvent, (event) => {
      setInset(Math.max(0, Math.min(event.endCoordinates.height, 420)));
    });
    const hideSubscription = Keyboard.addListener(hideEvent, () => setInset(0));
    return () => {
      showSubscription.remove();
      hideSubscription.remove();
    };
  }, []);

  return inset;
}

export function TerminalQuickBar({
  disabled = false,
  onInput,
  onPaste,
  onRefocus,
}: TerminalQuickBarProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const keyboardInset = useTerminalKeyboardInset();

  const finishAction = useCallback(() => {
    if (!onRefocus) return;
    if (Platform.OS === "web") requestAnimationFrame(onRefocus);
    else setTimeout(onRefocus, 0);
  }, [onRefocus]);

  const runInput = useCallback((data: string) => {
    if (disabled) return;
    if (Platform.OS === "ios") {
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    }
    onInput(data);
    finishAction();
  }, [disabled, finishAction, onInput]);

  const runPaste = useCallback(async () => {
    if (disabled) return;
    if (Platform.OS === "ios") {
      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    }
    try {
      await onPaste();
      finishAction();
    } catch {
      if (Platform.OS === "ios") {
        void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error);
      }
    }
  }, [disabled, finishAction, onPaste]);

  const renderQuickKey = (key: TerminalQuickKey) => (
    <Pressable
      key={key.id}
      accessibilityRole="button"
      accessibilityLabel={key.accessibilityLabel}
      disabled={disabled}
      onPress={() => runInput(key.data)}
      style={({ pressed }) => [
        styles.key,
        {
          backgroundColor: pressed ? `${t.userMessage}1F` : "transparent",
          opacity: disabled ? 0.45 : 1,
        },
      ]}
    >
      <Text style={[styles.keyLabel, { color: t.foreground }]}>
        {key.label}
      </Text>
    </Pressable>
  );

  return (
    <View
      pointerEvents="box-none"
      style={[
        styles.shell,
        { bottom: TERMINAL_OVERLAY_OFFSET + keyboardInset },
      ]}
    >
      <View
        style={[
          styles.controlsRow,
          { backgroundColor: `${t.background}EB` },
        ]}
      >
        <View style={[styles.controlCluster, styles.leftCluster]}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Paste into terminal"
            disabled={disabled}
            onPress={() => void runPaste()}
            style={({ pressed }) => [
              styles.key,
              styles.iconKey,
              {
                backgroundColor: pressed ? `${t.userMessage}1F` : "transparent",
                opacity: disabled ? 0.45 : 1,
              },
            ]}
          >
            <ClipboardPaste size={15} color={t.userMessage} strokeWidth={1.8} />
          </Pressable>
          {TERMINAL_QUICK_KEYS.slice(0, 2).map(renderQuickKey)}
        </View>

        <TerminalDirectionPad
          disabled={disabled}
          onInput={onInput}
          onFinish={finishAction}
        />

        <View style={[styles.controlCluster, styles.rightCluster]}>
          {TERMINAL_QUICK_KEYS.slice(2).map(renderQuickKey)}
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  shell: {
    position: "absolute",
    left: 8,
    right: 8,
    zIndex: 10,
    alignItems: "center",
  },
  controlsRow: {
    width: "100%",
    maxWidth: 400,
    minHeight: 54,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 8,
    paddingHorizontal: 6,
    paddingVertical: 3,
    borderRadius: 27,
  },
  controlCluster: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 2,
  },
  leftCluster: {
    justifyContent: "flex-end",
  },
  rightCluster: {
    justifyContent: "flex-start",
  },
  key: {
    minWidth: 32,
    height: 38,
    paddingHorizontal: 4,
    borderRadius: 19,
    flexShrink: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  iconKey: {
    paddingHorizontal: 0,
  },
  keyLabel: {
    fontSize: 12,
    fontWeight: "600",
    letterSpacing: 0.15,
  },
  directionPad: {
    width: DIRECTION_PUCK_SIZE,
    height: DIRECTION_PUCK_SIZE,
    borderRadius: DIRECTION_PUCK_SIZE / 2,
    alignItems: "center",
    justifyContent: "center",
  },
  directionKnob: {
    width: 14,
    height: 14,
    borderRadius: 7,
  },
  directionCue: {
    position: "absolute",
    fontSize: 9,
    fontWeight: "700",
  },
  directionUp: {
    top: 2,
    left: 21,
  },
  directionDown: {
    bottom: 2,
    left: 21,
  },
  directionLeft: {
    top: 17,
    left: 5,
  },
  directionRight: {
    top: 17,
    right: 5,
  },
});
