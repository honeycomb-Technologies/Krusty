import { useCallback, useEffect, useRef, useState } from 'react';
import {
  View,
  Pressable,
  ScrollView,
  StyleSheet,
  type GestureResponderEvent,
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from 'react-native';
import { BlurView } from '../../platform/blur';
import { LinearGradient } from '../../platform/linear-gradient';
import * as Haptics from '../../platform/haptics';
import Animated, {
  type SharedValue,
  useSharedValue,
  useAnimatedStyle,
  useAnimatedReaction,
  withSpring,
  withTiming,
  withDelay,
  interpolate,
  runOnJS,
  Extrapolation,
} from 'react-native-reanimated';
import { Gesture, GestureDetector } from 'react-native-gesture-handler';
import {
  Brain,
  Hammer,
  Compass,
  Paperclip,
  Bot,
  FlaskConical,
  ShieldCheck,
  ShieldOff,
  Zap,
  Image as ImageIcon,
  Camera,
  FileText,
} from 'lucide-react-native';
import { useThemeContext } from '../../hooks/useTheme';
import { useBreakpoint } from '../../hooks/useBreakpoint';
import { cycleThinkingLevel, type ThinkingLevel } from '@krusty/api';
import type { PermissionMode } from '@krusty/state';

interface AccordionControlsProps {
  thinkingLevel: ThinkingLevel;
  onThinkingChange: (level: ThinkingLevel) => void;
  permissionMode: PermissionMode;
  onPermissionModeToggle: () => void;
  fastModeEnabled?: boolean;
  fastModeSupported?: boolean;
  onFastModeToggle?: () => void;
  mode: 'build' | 'plan';
  onModeToggle: () => void;
  onAttach: () => void;
  attachPickerOpen: boolean;
  onPickPhoto: () => void;
  onPickCamera: () => void;
  onPickFile: () => void;
  onModelSelect: () => void;
  modelPickerOpen: boolean;
  providerFilters: ProviderFilterAction[];
  selectedProviderFilter: string | null;
  onProviderFilterToggle: (providerId: string) => void;
  onProviderFiltersReorder?: (providerIds: string[]) => void;
  model: string | null;
  isOpen: boolean;
  onToggle: () => void;
  sessionType?: 'chat' | 'code' | 'mako';
  researchEnabled?: boolean;
  onResearchToggle?: () => void;
}

interface ProviderFilterAction {
  id: string;
  label: string;
  icon: React.ReactNode;
}

const THINKING_ICON_ALPHA: Record<ThinkingLevel, string> = {
  off: '66',
  low: '66',
  medium: 'A6',
  high: 'D9',
  xhigh: '',
};

const SPRING_CONFIG = { damping: 18, stiffness: 350, mass: 0.6 };
const MAX_PILL_INDEX = 5;
const OPEN_STAGGER_MS = 40;
const CLOSE_STAGGER_MS = 28;
const ACTION_FADE_IN_MS = 70;
const ACTION_FADE_OUT_MS = 120;
const ATTACH_ACTION_COUNT = 3;
const DOCK_FADE_WIDTH = 34;
const MODEL_BUTTON_GAP = 10;
const PROVIDER_PILL_SIZE = 56;
const PROVIDER_PILL_GAP = 8;
const PROVIDER_PILL_STEP = PROVIDER_PILL_SIZE + PROVIDER_PILL_GAP;
const PROVIDER_CONTENT_PADDING_LEFT = DOCK_FADE_WIDTH - PROVIDER_PILL_GAP / 2;
const PROVIDER_CONTENT_PADDING_RIGHT = MODEL_BUTTON_GAP - PROVIDER_PILL_GAP / 2;
const PROVIDER_DOCK_HEIGHT = 72;
const PROVIDER_AUTO_SCROLL_EDGE_WIDTH = 52;
const PROVIDER_AUTO_SCROLL_MAX_STEP = 18;
const PROVIDER_REORDER_SPRING_CONFIG = { damping: 24, stiffness: 420, mass: 0.55 };
const PROVIDER_REORDER_LONG_PRESS_MS = 460;

/** Desktop provider filter — keeps staggered spring entrance without scale-to-zero. */
function DesktopFilterPill({
  index,
  active,
  label,
  onPress,
  children,
}: {
  index: number;
  active: boolean;
  label: string;
  onPress: () => void;
  children: React.ReactNode;
}) {
  const { theme } = useThemeContext();
  const progress = useSharedValue(0);

  useEffect(() => {
    progress.value = withDelay(
      index * OPEN_STAGGER_MS,
      withSpring(1, SPRING_CONFIG),
    );
  }, [index, progress]);

  const animatedStyle = useAnimatedStyle(() => ({
    opacity: progress.value,
    transform: [
      { translateX: interpolate(progress.value, [0, 1], [16, 0]) },
      { scale: interpolate(progress.value, [0, 1], [0.88, 1]) },
    ],
  }));

  const g = theme.colors.glass;

  return (
    <Animated.View style={[styles.desktopFilterHit, animatedStyle]}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Filter models by ${label}`}
        onPress={() => {
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          onPress();
        }}
        style={styles.desktopFilterHit}
      >
        <BlurView
          intensity={theme.colors.glassBlur}
          tint={
            theme.scheme === 'dark' ? 'systemMaterialDark' : 'systemMaterialLight'
          }
          style={styles.providerDockBlur}
        >
          <View
            style={[
              styles.providerDockPill,
              {
                backgroundColor: active
                  ? theme.colors.thinking + '18'
                  : g.background,
                borderColor: active
                  ? theme.colors.thinking + '80'
                  : g.border,
                borderWidth: StyleSheet.hairlineWidth,
                alignItems: 'center',
                justifyContent: 'center',
              },
            ]}
          >
            {children}
          </View>
        </BlurView>
      </Pressable>
    </Animated.View>
  );
}

function AccordionPill({
  children,
  index,
  isOpen,
  onPress,
  active = false,
  sideContent,
  disabled = false,
  compact = false,
}: {
  children: React.ReactNode;
  index: number;
  isOpen: boolean;
  onPress: () => void;
  active?: boolean;
  sideContent?: React.ReactNode;
  disabled?: boolean;
  /** When true, only as wide as the pill (no full-width row stretch). */
  compact?: boolean;
}) {
  const { theme } = useThemeContext();
  const progress = useSharedValue(0);
  const opacityProgress = useSharedValue(0);

  useEffect(() => {
    const delayMs = isOpen
      ? index * OPEN_STAGGER_MS
      : Math.max(0, MAX_PILL_INDEX - index) * CLOSE_STAGGER_MS;
    progress.value = withDelay(
      delayMs,
      withSpring(isOpen ? 1 : 0, SPRING_CONFIG),
    );
    opacityProgress.value = withDelay(
      delayMs,
      withTiming(isOpen ? 1 : 0, {
        duration: isOpen ? ACTION_FADE_IN_MS : ACTION_FADE_OUT_MS,
      }),
    );
  }, [index, isOpen, opacityProgress, progress]);

  const animatedStyle = useAnimatedStyle(() => ({
    opacity: opacityProgress.value,
    transform: [
      { translateY: interpolate(progress.value, [0, 1], [20, 0]) },
      { scale: interpolate(progress.value, [0, 1], [0.8, 1]) },
    ],
  }));

  const g = theme.colors.glass;

  return (
    <Animated.View
      pointerEvents="box-none"
      style={[
        compact ? styles.pillOuterCompact : styles.pillOuter,
        animatedStyle,
      ]}
    >
      {sideContent}
      <Pressable disabled={!isOpen || disabled} onPress={onPress}>
        <BlurView
          intensity={theme.colors.glassBlur}
          tint={theme.scheme === 'dark' ? 'systemMaterialDark' : 'systemMaterialLight'}
          style={styles.pillBlur}
        >
          <View
            style={[
              styles.pill,
              {
                backgroundColor: active ? theme.colors.thinking + '18' : g.background,
                borderColor: active ? theme.colors.thinking + '80' : g.border,
              },
            ]}
          >
            {children}
          </View>
        </BlurView>
      </Pressable>
    </Animated.View>
  );
}

function InlineActionPill({
  index,
  itemCount,
  isOpen,
  onPress,
  children,
  active = false,
  size = 56,
  accessibilityLabel,
  closeStaggerMs = CLOSE_STAGGER_MS,
}: {
  index: number;
  itemCount: number;
  isOpen: boolean;
  onPress: () => void;
  children: React.ReactNode;
  active?: boolean;
  size?: number;
  accessibilityLabel?: string;
  closeStaggerMs?: number;
}) {
  const { theme } = useThemeContext();
  const progress = useSharedValue(0);
  const opacityProgress = useSharedValue(0);

  useEffect(() => {
    const delayMs = isOpen
      ? index * OPEN_STAGGER_MS
      : Math.max(0, itemCount - index - 1) * closeStaggerMs;
    progress.value = withDelay(
      delayMs,
      withSpring(isOpen ? 1 : 0, SPRING_CONFIG),
    );
    opacityProgress.value = withDelay(
      delayMs,
      withTiming(isOpen ? 1 : 0, {
        duration: isOpen ? ACTION_FADE_IN_MS : ACTION_FADE_OUT_MS,
      }),
    );
  }, [isOpen, itemCount, closeStaggerMs, opacityProgress, progress]);

  const animatedStyle = useAnimatedStyle(() => ({
    opacity: opacityProgress.value,
    transform: [
      { translateX: interpolate(progress.value, [0, 1], [size * 0.75, 0]) },
      { scale: interpolate(progress.value, [0, 1], [0.01, 1]) },
    ],
  }));

  const g = theme.colors.glass;

  return (
    <Animated.View
      pointerEvents="box-none"
      style={[styles.inlineActionOuter, { width: size, height: size }, animatedStyle]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={accessibilityLabel}
        disabled={!isOpen}
        onPress={() => {
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          onPress();
        }}
      >
        <BlurView
          intensity={theme.colors.glassBlur}
          tint={theme.scheme === 'dark' ? 'systemMaterialDark' : 'systemMaterialLight'}
          style={styles.pillBlur}
        >
          <View
            style={[
              styles.inlinePill,
              {
                width: size,
                height: size,
                borderRadius: size >= 56 ? 18 : 14,
                backgroundColor: active ? theme.colors.thinking + '18' : g.background,
                borderColor: active ? theme.colors.thinking + '80' : g.border,
              },
            ]}
          >
            {children}
          </View>
        </BlurView>
      </Pressable>
    </Animated.View>
  );
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function ProviderDockPill({
  provider,
  index,
  itemCount,
  isOpen,
  active,
  editMode,
  canReorder,
  dragIndex,
  dropIndex,
  sharedDragX,
  dragScrollDelta,
  onPress,
  onDragStart,
  onDrop,
  onDragFinalize,
  onAutoScrollPointer,
  children,
}: {
  provider: ProviderFilterAction;
  index: number;
  itemCount: number;
  isOpen: boolean;
  active: boolean;
  editMode: boolean;
  canReorder: boolean;
  dragIndex: SharedValue<number>;
  dropIndex: SharedValue<number>;
  sharedDragX: SharedValue<number>;
  dragScrollDelta: SharedValue<number>;
  onPress: () => void;
  onDragStart: (providerId: string, index: number, absoluteX: number) => void;
  onDrop: (providerId: string, toIndex: number) => void;
  onDragFinalize: () => void;
  onAutoScrollPointer: (absoluteX: number) => void;
  children: React.ReactNode;
}) {
  const { theme } = useThemeContext();
  const editProgress = useSharedValue(0);
  const revealProgress = useSharedValue(0);
  const revealOpacityProgress = useSharedValue(0);
  const reorderX = useSharedValue(0);
  const dragging = useSharedValue(0);
  const rawDragX = useSharedValue(0);
  const suppressNextPressRef = useRef(false);
  const touchStartXRef = useRef(0);
  const touchDragActiveRef = useRef(false);

  useEffect(() => {
    editProgress.value = withTiming(editMode ? 1 : 0, { duration: 150 });
  }, [editMode]);

  useEffect(() => {
    const animationIndex = itemCount - index - 1;
    const delayMs = isOpen
      ? animationIndex * OPEN_STAGGER_MS
      : index * CLOSE_STAGGER_MS;
    revealProgress.value = withDelay(
      delayMs,
      withSpring(isOpen ? 1 : 0, SPRING_CONFIG),
    );
    revealOpacityProgress.value = withDelay(
      delayMs,
      withTiming(isOpen ? 1 : 0, {
        duration: isOpen ? ACTION_FADE_IN_MS : ACTION_FADE_OUT_MS,
      }),
    );
  }, [index, isOpen, itemCount, revealOpacityProgress, revealProgress]);

  useEffect(() => {
    reorderX.value = 0;
  }, [index, reorderX]);

  useAnimatedReaction(
    () => {
      const fromIndex = dragIndex.value;
      const targetIndex = dropIndex.value;

      if (fromIndex < 0 || targetIndex < 0 || index === fromIndex) {
        return 0;
      }

      if (targetIndex > fromIndex && index > fromIndex && index <= targetIndex) {
        return -PROVIDER_PILL_STEP;
      }

      if (targetIndex < fromIndex && index >= targetIndex && index < fromIndex) {
        return PROVIDER_PILL_STEP;
      }

      return 0;
    },
    (nextOffset) => {
      reorderX.value = withSpring(nextOffset, PROVIDER_REORDER_SPRING_CONFIG);
    },
    [index],
  );

  useAnimatedReaction(
    () => {
      if (dragIndex.value !== index) return null;

      const minDragX = -index * PROVIDER_PILL_STEP;
      const maxDragX = (itemCount - index - 1) * PROVIDER_PILL_STEP;
      const proposedDragX = rawDragX.value + dragScrollDelta.value;
      return Math.min(maxDragX, Math.max(minDragX, proposedDragX));
    },
    (nextDragX) => {
      if (nextDragX === null) return;
      sharedDragX.value = nextDragX;
      dropIndex.value = Math.min(
        itemCount - 1,
        Math.max(0, index + Math.round(nextDragX / PROVIDER_PILL_STEP)),
      );
    },
    [index, itemCount],
  );

  const handleDragStart = useCallback((absoluteX: number) => {
    suppressNextPressRef.current = true;
    onDragStart(provider.id, index, absoluteX);
  }, [index, onDragStart, provider.id]);

  const resetLocalDragState = useCallback(() => {
    touchDragActiveRef.current = false;
    rawDragX.value = 0;
    dragging.value = withTiming(0, { duration: 90 });
    sharedDragX.value = withSpring(0, PROVIDER_REORDER_SPRING_CONFIG);
  }, [dragging, rawDragX, sharedDragX]);

  const beginTouchDrag = useCallback((absoluteX: number) => {
    if (!isOpen || !canReorder || touchDragActiveRef.current) return;
    touchStartXRef.current = absoluteX;
    touchDragActiveRef.current = true;
    suppressNextPressRef.current = true;
    dragging.value = 1;
    dragIndex.value = index;
    dropIndex.value = index;
    rawDragX.value = 0;
    sharedDragX.value = 0;
    handleDragStart(absoluteX);
  }, [canReorder, dragIndex, dragging, dropIndex, handleDragStart, index, isOpen, rawDragX, sharedDragX]);

  const handleTouchStart = useCallback((event: GestureResponderEvent) => {
    const absoluteX = event.nativeEvent.pageX;
    touchStartXRef.current = absoluteX;
    if (editMode) {
      beginTouchDrag(absoluteX);
    }
  }, [beginTouchDrag, editMode]);

  const handleLongPress = useCallback((event: GestureResponderEvent) => {
    beginTouchDrag(event.nativeEvent.pageX);
  }, [beginTouchDrag]);

  const handleTouchMove = useCallback((event: GestureResponderEvent) => {
    if (!touchDragActiveRef.current) return;
    const absoluteX = event.nativeEvent.pageX;
    rawDragX.value = absoluteX - touchStartXRef.current;
    onAutoScrollPointer(absoluteX);
  }, [onAutoScrollPointer, rawDragX]);

  const finishTouchDrag = useCallback((cancelled: boolean) => {
    if (!touchDragActiveRef.current) return;

    if (cancelled) {
      resetLocalDragState();
      onDragFinalize();
      return;
    }

    const minDragX = -index * PROVIDER_PILL_STEP;
    const maxDragX = (itemCount - index - 1) * PROVIDER_PILL_STEP;
    const nextDragX = Math.min(
      maxDragX,
      Math.max(minDragX, rawDragX.value + dragScrollDelta.value),
    );
    const targetIndex = Math.min(
      itemCount - 1,
      Math.max(0, index + Math.round(nextDragX / PROVIDER_PILL_STEP)),
    );
    resetLocalDragState();
    onDrop(provider.id, targetIndex);
  }, [dragScrollDelta, index, itemCount, onDragFinalize, onDrop, provider.id, rawDragX, resetLocalDragState]);

  const handleTouchEnd = useCallback(() => {
    finishTouchDrag(false);
  }, [finishTouchDrag]);

  const handleTouchCancel = useCallback(() => {
    finishTouchDrag(true);
  }, [finishTouchDrag]);

  const animatedStyle = useAnimatedStyle(() => {
    const isActiveDrag = dragIndex.value === index;
    const revealTranslateX = interpolate(
      revealProgress.value,
      [0, 1],
      [PROVIDER_PILL_SIZE * 0.75, 0],
    );
    const revealScale = interpolate(revealProgress.value, [0, 1], [0.01, 1]);
    const lift = interpolate(editProgress.value, [0, 1], [0, -2]);
    const dragLift = interpolate(dragging.value, [0, 1], [0, -4]);
    const tilt = interpolate(
      editProgress.value,
      [0, 1],
      [0, index % 2 === 0 ? -1.2 : 1.2],
      Extrapolation.CLAMP,
    );

    return {
      opacity: revealOpacityProgress.value,
      zIndex: isActiveDrag ? 20 : editProgress.value ? 4 : 0,
      transform: [
        {
          translateX: revealTranslateX + (isActiveDrag ? sharedDragX.value : reorderX.value),
        },
        { translateY: lift + dragLift },
        { scale: revealScale * (1 + editProgress.value * 0.02 + dragging.value * 0.05) },
        { rotate: `${tilt}deg` },
      ],
    };
  });

  const g = theme.colors.glass;

  const pill = (
    <Animated.View
      pointerEvents={isOpen ? "auto" : "none"}
      style={[
        styles.providerDockCell,
        animatedStyle,
      ]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Filter models by ${provider.label}`}
        disabled={!isOpen}
        style={styles.providerDockPressable}
        delayLongPress={PROVIDER_REORDER_LONG_PRESS_MS}
        onLongPress={handleLongPress}
        onTouchStart={handleTouchStart}
        onTouchMove={handleTouchMove}
        onTouchEnd={handleTouchEnd}
        onTouchCancel={handleTouchCancel}
        onPress={() => {
          if (suppressNextPressRef.current) {
            suppressNextPressRef.current = false;
            return;
          }
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          onPress();
        }}
      >
        <BlurView
          intensity={theme.colors.glassBlur}
          tint={theme.scheme === 'dark' ? 'systemMaterialDark' : 'systemMaterialLight'}
          style={styles.providerDockBlur}
        >
          <View
            style={[
              styles.inlinePill,
              styles.providerDockPill,
              {
                backgroundColor: active ? theme.colors.thinking + '18' : g.background,
                borderColor: editMode || active ? theme.colors.thinking + '70' : g.border,
              },
            ]}
          >
            {children}
          </View>
        </BlurView>
      </Pressable>
    </Animated.View>
  );

  return pill;
}

export function AccordionControls({
  thinkingLevel,
  onThinkingChange,
  permissionMode,
  onPermissionModeToggle,
  fastModeEnabled = false,
  fastModeSupported = false,
  onFastModeToggle,
  mode,
  onModeToggle,
  onAttach,
  attachPickerOpen,
  onPickPhoto,
  onPickCamera,
  onPickFile,
  onModelSelect,
  modelPickerOpen,
  providerFilters,
  selectedProviderFilter,
  onProviderFilterToggle,
  onProviderFiltersReorder,
  model,
  isOpen,
  onToggle,
  sessionType = 'code',
  researchEnabled = false,
  onResearchToggle,
}: AccordionControlsProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  // Desktop has room — no edge-fade “scroll chrome” or long-press reorder.
  const enableProviderReorder = !isDesktop && providerFilters.length > 1;
  const providerScrollRef = useRef<ScrollView>(null);
  const providerDockOpenRef = useRef(false);
  const providerEditExitTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const providerDraggingRef = useRef(false);
  const providerAutoScrollFrameRef = useRef<number | null>(null);
  const providerAutoScrollPointerXRef = useRef<number | null>(null);
  const providerDragStartScrollXRef = useRef(0);
  const providerDockMetricsRef = useRef({ left: 0, width: 0 });
  const providerScrollMetricsRef = useRef({ x: 0, width: 0, contentWidth: 0 });
  const [providerEditMode, setProviderEditMode] = useState(false);
  const [providerDragging, setProviderDragging] = useState(false);
  const providerDockProgress = useSharedValue(0);
  const providerDragIndex = useSharedValue(-1);
  const providerDropIndex = useSharedValue(-1);
  const providerDragX = useSharedValue(0);
  const providerDragScrollDelta = useSharedValue(0);
  const isChat = sessionType === 'chat';
  const isMako = sessionType === 'mako';

  const clearProviderEditExitTimer = useCallback(() => {
    if (!providerEditExitTimerRef.current) return;
    clearTimeout(providerEditExitTimerRef.current);
    providerEditExitTimerRef.current = null;
  }, []);

  useEffect(() => () => {
    clearProviderEditExitTimer();
  }, [clearProviderEditExitTimer]);

  const measureProviderDock = useCallback(() => {
    requestAnimationFrame(() => {
      providerScrollRef.current?.getNativeScrollRef()?.measureInWindow((x: number, _y: number, width: number) => {
        providerDockMetricsRef.current = { left: x, width };
        providerScrollMetricsRef.current.width = width;
      });
    });
  }, []);

  const stopProviderAutoScroll = useCallback(() => {
    providerAutoScrollPointerXRef.current = null;
    if (providerAutoScrollFrameRef.current !== null) {
      cancelAnimationFrame(providerAutoScrollFrameRef.current);
      providerAutoScrollFrameRef.current = null;
    }
  }, []);

  const estimateProviderContentWidth = useCallback(() => (
    PROVIDER_CONTENT_PADDING_LEFT +
    PROVIDER_CONTENT_PADDING_RIGHT +
    providerFilters.length * PROVIDER_PILL_STEP
  ), [providerFilters.length]);

  const scrollProviderDockTo = useCallback((nextX: number) => {
    const metrics = providerScrollMetricsRef.current;
    const maxX = Math.max(0, metrics.contentWidth - metrics.width);
    const clampedX = clamp(nextX, 0, maxX);

    if (Math.abs(clampedX - metrics.x) < 0.5) return false;

    metrics.x = clampedX;
    providerDragScrollDelta.value = clampedX - providerDragStartScrollXRef.current;
    providerScrollRef.current?.scrollTo({ x: clampedX, animated: false });
    return true;
  }, [providerDragScrollDelta]);

  const stepProviderAutoScroll = useCallback(() => {
    providerAutoScrollFrameRef.current = null;
    if (!providerDraggingRef.current) return;

    const pointerX = providerAutoScrollPointerXRef.current;
    const dockMetrics = providerDockMetricsRef.current;
    const scrollMetrics = providerScrollMetricsRef.current;
    if (pointerX === null) return;

    if (dockMetrics.width <= 0 || scrollMetrics.width <= 0) {
      measureProviderDock();
      providerAutoScrollFrameRef.current = requestAnimationFrame(stepProviderAutoScroll);
      return;
    }

    scrollMetrics.contentWidth = Math.max(
      scrollMetrics.contentWidth,
      estimateProviderContentWidth(),
    );

    if (scrollMetrics.contentWidth <= scrollMetrics.width) return;

    const leftEdge = dockMetrics.left + DOCK_FADE_WIDTH;
    const rightEdge = dockMetrics.left + dockMetrics.width - MODEL_BUTTON_GAP;
    let scrollStep = 0;

    if (pointerX < leftEdge + PROVIDER_AUTO_SCROLL_EDGE_WIDTH) {
      const pressure = clamp(
        (leftEdge + PROVIDER_AUTO_SCROLL_EDGE_WIDTH - pointerX) / PROVIDER_AUTO_SCROLL_EDGE_WIDTH,
        0,
        1,
      );
      scrollStep = -PROVIDER_AUTO_SCROLL_MAX_STEP * pressure;
    } else if (pointerX > rightEdge - PROVIDER_AUTO_SCROLL_EDGE_WIDTH) {
      const pressure = clamp(
        (pointerX - (rightEdge - PROVIDER_AUTO_SCROLL_EDGE_WIDTH)) / PROVIDER_AUTO_SCROLL_EDGE_WIDTH,
        0,
        1,
      );
      scrollStep = PROVIDER_AUTO_SCROLL_MAX_STEP * pressure;
    }

    if (Math.abs(scrollStep) < 0.5) {
      providerAutoScrollFrameRef.current = requestAnimationFrame(stepProviderAutoScroll);
      return;
    }

    scrollProviderDockTo(scrollMetrics.x + scrollStep);
    if (providerDraggingRef.current) {
      providerAutoScrollFrameRef.current = requestAnimationFrame(stepProviderAutoScroll);
    }
  }, [estimateProviderContentWidth, measureProviderDock, scrollProviderDockTo]);

  const scheduleProviderAutoScroll = useCallback(() => {
    if (providerAutoScrollFrameRef.current !== null) return;
    providerAutoScrollFrameRef.current = requestAnimationFrame(stepProviderAutoScroll);
  }, [stepProviderAutoScroll]);

  useEffect(() => () => {
    stopProviderAutoScroll();
  }, [stopProviderAutoScroll]);

  const resetProviderDockDragState = useCallback(() => {
    providerDraggingRef.current = false;
    stopProviderAutoScroll();
    providerDragIndex.value = -1;
    providerDropIndex.value = -1;
    providerDragX.value = 0;
    providerDragScrollDelta.value = 0;
    setProviderDragging(false);
  }, [providerDragIndex, providerDragScrollDelta, providerDropIndex, providerDragX, stopProviderAutoScroll]);

  const handleThinking = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const next = cycleThinkingLevel(thinkingLevel, model);
    onThinkingChange(next);
  };

  const handleMode = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onModeToggle();
  };

  const handleResearch = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onResearchToggle?.();
  };

  const handleAttach = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onAttach();
  };

  const handleFastMode = () => {
    if (!fastModeSupported) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onFastModeToggle?.();
  };

  const handlePermissionMode = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onPermissionModeToggle();
  };

  const handleModel = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onModelSelect();
  };

  const t = theme.colors;
  const fabAccent = t.thinking;
  const thinkingColor = thinkingLevel === 'off'
    ? `${t.mutedForeground}${THINKING_ICON_ALPHA.off}`
    : `${fabAccent}${THINKING_ICON_ALPHA[thinkingLevel]}`;
  const dockFadeColor = theme.scheme === 'dark'
    ? 'rgba(11,17,25,0.92)'
    : 'rgba(255,255,255,0.92)';

  const providerDockOpen = modelPickerOpen && isOpen;

  useEffect(() => {
    providerDockProgress.value = withSpring(providerDockOpen ? 1 : 0, SPRING_CONFIG);
  }, [providerDockOpen, providerDockProgress]);

  const providerDockFadeAnimatedStyle = useAnimatedStyle(() => ({
    opacity: providerDockProgress.value,
  }));

  useEffect(() => {
    if (!providerDockOpen) {
      providerDockOpenRef.current = false;
      setProviderEditMode(false);
      clearProviderEditExitTimer();
      resetProviderDockDragState();
      return;
    }
    if (providerDockOpenRef.current) return;
    providerDockOpenRef.current = true;
    requestAnimationFrame(() => {
      providerScrollRef.current?.scrollToEnd({ animated: false });
      const metrics = providerScrollMetricsRef.current;
      metrics.x = Math.max(0, metrics.contentWidth - metrics.width);
      measureProviderDock();
    });
  }, [clearProviderEditExitTimer, measureProviderDock, providerDockOpen, providerFilters.length, resetProviderDockDragState]);

  const handleProviderScrollLayout = useCallback((event: LayoutChangeEvent) => {
    providerScrollMetricsRef.current.width = event.nativeEvent.layout.width;
    measureProviderDock();
  }, [measureProviderDock]);

  const handleProviderContentSizeChange = useCallback((width: number) => {
    providerScrollMetricsRef.current.contentWidth = Math.max(
      width,
      estimateProviderContentWidth(),
    );
    if (providerDockOpenRef.current && !providerDraggingRef.current) {
      const metrics = providerScrollMetricsRef.current;
      metrics.x = Math.max(0, metrics.contentWidth - metrics.width);
    }
  }, [estimateProviderContentWidth]);

  const handleProviderScroll = useCallback((event: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
    providerScrollMetricsRef.current = {
      x: contentOffset.x,
      width: layoutMeasurement.width,
      contentWidth: Math.max(contentSize.width, estimateProviderContentWidth()),
    };
    if (providerDraggingRef.current) {
      providerDragScrollDelta.value = contentOffset.x - providerDragStartScrollXRef.current;
    }
  }, [estimateProviderContentWidth, providerDragScrollDelta]);

  const handleProviderDragStart = useCallback((_providerId: string, _index: number, absoluteX: number) => {
    clearProviderEditExitTimer();
    providerDraggingRef.current = true;
    providerAutoScrollPointerXRef.current = absoluteX;
    providerScrollMetricsRef.current.contentWidth = Math.max(
      providerScrollMetricsRef.current.contentWidth,
      estimateProviderContentWidth(),
    );
    providerDragStartScrollXRef.current = providerScrollMetricsRef.current.x;
    providerDragScrollDelta.value = 0;
    setProviderDragging(true);
    setProviderEditMode(true);
    measureProviderDock();
    scheduleProviderAutoScroll();
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
  }, [clearProviderEditExitTimer, estimateProviderContentWidth, measureProviderDock, providerDragScrollDelta, scheduleProviderAutoScroll]);

  const handleProviderDragFinalize = useCallback(() => {
    resetProviderDockDragState();
  }, [resetProviderDockDragState]);

  const handleProviderAutoScrollPointer = useCallback((absoluteX: number) => {
    providerAutoScrollPointerXRef.current = absoluteX;
    if (providerDraggingRef.current) {
      scheduleProviderAutoScroll();
    }
  }, [scheduleProviderAutoScroll]);

  const handleProviderDrop = useCallback((providerId: string, toIndex: number) => {
    const fromIndex = providerFilters.findIndex((provider) => provider.id === providerId);
    if (fromIndex < 0) {
      resetProviderDockDragState();
      clearProviderEditExitTimer();
      setProviderEditMode(false);
      return;
    }

    const nextIndex = clamp(toIndex, 0, providerFilters.length - 1);
    resetProviderDockDragState();

    if (nextIndex !== fromIndex && onProviderFiltersReorder) {
      const nextProviders = [...providerFilters];
      const [moved] = nextProviders.splice(fromIndex, 1);
      nextProviders.splice(nextIndex, 0, moved);
      onProviderFiltersReorder(nextProviders.map((provider) => provider.id));
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    }

    clearProviderEditExitTimer();
    setProviderEditMode(false);
  }, [clearProviderEditExitTimer, onProviderFiltersReorder, providerFilters, resetProviderDockDragState]);

  const swipeDown = Gesture.Pan()
    .activeOffsetY(15)
    .failOffsetX([-20, 20])
    .onEnd((event) => {
      if (event.translationY > 40 && isOpen) {
        runOnJS(onToggle)();
      }
    });

  const g = theme.colors.glass;
  const showDesktopFilters = isDesktop && modelPickerOpen && isOpen;
  const desktopFilterCount = providerFilters.length;

  return (
    <View style={styles.container} pointerEvents="box-none">
      {/* Floating accordion pills */}
      <GestureDetector gesture={swipeDown}>
        <Animated.View style={styles.pillColumn}>
          {/* Desktop: filters + bot as a flex row. Filters keep a light stagger
              animation but avoid nested sideContent/scale-to-zero (which hid them). */}
          {isDesktop ? (
            <View style={styles.desktopModelRow} pointerEvents="box-none">
              {showDesktopFilters
                ? providerFilters.map((provider, visualIndex) => {
                    const active = selectedProviderFilter === provider.id;
                    // Stagger from the bot outward (right → left).
                    const staggerIndex = desktopFilterCount - visualIndex - 1;
                    return (
                      <DesktopFilterPill
                        key={provider.id}
                        index={staggerIndex}
                        active={active}
                        label={provider.label}
                        onPress={() => onProviderFilterToggle(provider.id)}
                      >
                        {provider.icon}
                      </DesktopFilterPill>
                    );
                  })
                : null}
              <AccordionPill
                index={5}
                isOpen={isOpen}
                onPress={handleModel}
                active={modelPickerOpen}
                compact
              >
                <Bot
                  size={24}
                  color={modelPickerOpen ? fabAccent : t.mutedForeground}
                  strokeWidth={1.6}
                />
              </AccordionPill>
            </View>
          ) : (
          <AccordionPill
            index={5}
            isOpen={isOpen}
            onPress={handleModel}
            active={modelPickerOpen}
            sideContent={
              <Animated.View
                pointerEvents={providerDockOpen ? "box-none" : "none"}
                style={styles.modelFilterDock}
              >
                <ScrollView
                  ref={providerScrollRef}
                  horizontal
                  bounces={false}
                  alwaysBounceHorizontal={false}
                  directionalLockEnabled
                  nestedScrollEnabled
                  keyboardShouldPersistTaps="always"
                  overScrollMode="never"
                  scrollEnabled={modelPickerOpen && isOpen && !providerDragging}
                  showsHorizontalScrollIndicator={false}
                  scrollEventThrottle={16}
                  onLayout={handleProviderScrollLayout}
                  onContentSizeChange={handleProviderContentSizeChange}
                  onScroll={handleProviderScroll}
                  style={styles.modelFilterScroll}
                  contentContainerStyle={styles.modelFilterContent}
                >
                  {providerFilters.map((provider, visualIndex) => (
                    <ProviderDockPill
                      key={provider.id}
                      provider={provider}
                      index={visualIndex}
                      itemCount={providerFilters.length}
                      isOpen={modelPickerOpen && isOpen}
                      active={selectedProviderFilter === provider.id}
                      editMode={enableProviderReorder && providerEditMode}
                      canReorder={enableProviderReorder}
                      dragIndex={providerDragIndex}
                      dropIndex={providerDropIndex}
                      sharedDragX={providerDragX}
                      dragScrollDelta={providerDragScrollDelta}
                      onPress={() => onProviderFilterToggle(provider.id)}
                      onDragStart={handleProviderDragStart}
                      onDrop={handleProviderDrop}
                      onDragFinalize={handleProviderDragFinalize}
                      onAutoScrollPointer={handleProviderAutoScrollPointer}
                    >
                      {provider.icon}
                    </ProviderDockPill>
                  ))}
                </ScrollView>
                <Animated.View
                  pointerEvents="none"
                  style={[styles.modelFilterFadeLeft, providerDockFadeAnimatedStyle]}
                >
                  <LinearGradient
                    colors={[dockFadeColor, 'transparent']}
                    start={{ x: 0, y: 0.5 }}
                    end={{ x: 1, y: 0.5 }}
                    style={StyleSheet.absoluteFill}
                  />
                </Animated.View>
                <Animated.View
                  pointerEvents="none"
                  style={[styles.modelFilterFadeRight, providerDockFadeAnimatedStyle]}
                >
                  <LinearGradient
                    colors={['transparent', dockFadeColor]}
                    start={{ x: 0, y: 0.5 }}
                    end={{ x: 1, y: 0.5 }}
                    style={StyleSheet.absoluteFill}
                  />
                </Animated.View>
              </Animated.View>
            }
          >
            <Bot
              size={24}
              color={modelPickerOpen ? fabAccent : t.mutedForeground}
              strokeWidth={1.6}
            />
          </AccordionPill>
          )}

          <AccordionPill
            index={4}
            isOpen={isOpen}
            onPress={handleAttach}
            active={attachPickerOpen}
            sideContent={
              <View
                pointerEvents={attachPickerOpen ? "box-none" : "none"}
                style={styles.attachActions}
              >
                <InlineActionPill
                  index={2}
                  itemCount={ATTACH_ACTION_COUNT}
                  isOpen={attachPickerOpen && isOpen}
                  onPress={onPickFile}
                  accessibilityLabel="Attach file"
                >
                  <FileText size={23} color={t.mutedForeground} strokeWidth={1.7} />
                </InlineActionPill>
                <InlineActionPill
                  index={1}
                  itemCount={ATTACH_ACTION_COUNT}
                  isOpen={attachPickerOpen && isOpen}
                  onPress={onPickCamera}
                  accessibilityLabel="Take photo"
                >
                  <Camera size={23} color={t.mutedForeground} strokeWidth={1.7} />
                </InlineActionPill>
                <InlineActionPill
                  index={0}
                  itemCount={ATTACH_ACTION_COUNT}
                  isOpen={attachPickerOpen && isOpen}
                  onPress={onPickPhoto}
                  accessibilityLabel="Choose photo"
                >
                  <ImageIcon size={23} color={t.mutedForeground} strokeWidth={1.7} />
                </InlineActionPill>
              </View>
            }
          >
            <Paperclip
              size={24}
              color={attachPickerOpen ? fabAccent : t.mutedForeground}
              strokeWidth={1.6}
            />
          </AccordionPill>

          {isChat ? (
            <AccordionPill index={3} isOpen={isOpen} onPress={handleResearch}>
              <FlaskConical size={24} color={researchEnabled ? fabAccent : t.mutedForeground} strokeWidth={1.6} />
            </AccordionPill>
          ) : !isMako ? (
            <AccordionPill index={3} isOpen={isOpen} onPress={handleMode}>
              {mode === 'build' ? (
                <Hammer size={24} color={t.mutedForeground} strokeWidth={1.6} />
              ) : (
                <Compass size={24} color={t.mutedForeground} strokeWidth={1.6} />
              )}
            </AccordionPill>
          ) : null}

          <AccordionPill index={2} isOpen={isOpen} onPress={handlePermissionMode}>
            {permissionMode === 'supervised' ? (
              <ShieldCheck size={24} color={t.success} strokeWidth={1.6} />
            ) : (
              <ShieldOff size={24} color={fabAccent} strokeWidth={1.6} />
            )}
          </AccordionPill>

          <AccordionPill
            index={1}
            isOpen={isOpen}
            onPress={handleFastMode}
            disabled={!fastModeSupported}
            active={fastModeSupported && fastModeEnabled}
          >
            <Zap
              size={24}
              color={
                fastModeSupported && fastModeEnabled
                  ? fabAccent
                  : fastModeSupported
                    ? t.mutedForeground
                    : `${t.mutedForeground}66`
              }
              strokeWidth={1.6}
            />
          </AccordionPill>

          <AccordionPill index={0} isOpen={isOpen} onPress={handleThinking}>
            <Brain size={24} color={thinkingColor} strokeWidth={1.6} />
          </AccordionPill>
        </Animated.View>
      </GestureDetector>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    width: '100%',
    alignItems: 'flex-end',
  },
  pillColumn: {
    width: '100%',
    gap: 10,
    alignItems: 'flex-end',
  },
  pillOuter: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    height: 56,
    width: '100%',
    overflow: 'visible',
  },
  pillOuterCompact: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    height: 56,
    width: PROVIDER_PILL_SIZE,
    flexShrink: 0,
    overflow: 'visible',
  },
  desktopModelRow: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    width: '100%',
    height: 56,
    gap: PROVIDER_PILL_GAP,
    overflow: 'visible',
  },
  desktopFilterHit: {
    width: PROVIDER_PILL_SIZE,
    height: PROVIDER_PILL_SIZE,
    flexShrink: 0,
    alignItems: 'center',
    justifyContent: 'center',
  },
  modelFilterDock: {
    flex: 1,
    minWidth: 0,
    height: PROVIDER_DOCK_HEIGHT,
    marginRight: 0,
    overflow: 'hidden',
    position: 'relative',
    justifyContent: 'center',
  },
  modelFilterDockDesktop: {
    flex: 0,
    flexGrow: 0,
    flexShrink: 0,
    overflow: 'visible',
    // Same gap ChatBar uses between model list and crab FAB column.
    marginRight: MODEL_BUTTON_GAP,
  },
  modelFilterRowDesktop: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-start',
    height: PROVIDER_DOCK_HEIGHT,
    width: '100%',
  },
  modelFilterScroll: {
    flex: 1,
  },
  modelFilterContent: {
    minWidth: '100%',
    minHeight: PROVIDER_DOCK_HEIGHT,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    gap: 0,
    paddingLeft: PROVIDER_CONTENT_PADDING_LEFT,
    paddingRight: PROVIDER_CONTENT_PADDING_RIGHT,
  },
  modelFilterFadeLeft: {
    position: 'absolute',
    left: 0,
    top: 0,
    bottom: 0,
    width: DOCK_FADE_WIDTH,
  },
  modelFilterFadeRight: {
    position: 'absolute',
    right: 0,
    top: 0,
    bottom: 0,
    width: MODEL_BUTTON_GAP,
  },
  attachActions: {
    flexDirection: 'row',
    alignItems: 'center',
    flexShrink: 0,
    gap: 10,
    marginRight: 10,
  },
  inlineActionOuter: {
    alignItems: 'center',
    justifyContent: 'center',
  },
  providerDockCell: {
    width: PROVIDER_PILL_STEP,
    height: PROVIDER_DOCK_HEIGHT,
    alignItems: 'center',
    justifyContent: 'center',
    flexShrink: 0,
    overflow: 'visible',
  },
  providerDockPressable: {
    width: '100%',
    height: '100%',
    alignItems: 'center',
    justifyContent: 'center',
  },
  inlinePill: {
    justifyContent: 'center',
    alignItems: 'center',
    borderWidth: StyleSheet.hairlineWidth,
  },
  providerDockPill: {
    width: PROVIDER_PILL_SIZE,
    height: PROVIDER_PILL_SIZE,
    borderRadius: 18,
  },
  providerDockBlur: {
    borderRadius: 18,
    overflow: 'hidden',
  },
  pillBlur: {
    borderRadius: 16,
    overflow: 'hidden',
  },
  pill: {
    width: 56,
    height: 56,
    borderRadius: 18,
    justifyContent: 'center',
    alignItems: 'center',
    borderWidth: StyleSheet.hairlineWidth,
  },
});
