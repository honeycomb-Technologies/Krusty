import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
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
import { AdaptiveMaterial } from '../ui/AdaptiveMaterial';
import { FabGooeyLayer } from './FabGooeyLayer';
import {
  GOOEY_PAD,
  gooeyCanvasHeight,
  gooeyCanvasWidth,
  gooeyFill,
  pillTravelY,
  type GooeyProgresses,
} from './fabGooey';
import * as Haptics from '../../platform/haptics';
import Animated, {
  type SharedValue,
  useSharedValue,
  useAnimatedStyle,
  useAnimatedReaction,
  withSpring,
  withTiming,
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
  ShieldCheck,
  ShieldOff,
  Zap,
  Image as ImageIcon,
  Camera,
  FileText,
} from 'lucide-react-native';
import { useThemeContext } from '../../hooks/useTheme';
import { useBreakpoint } from '../../hooks/useBreakpoint';
import {
  cycleThinkingLevel,
  supportsThinking,
  type ModelInfo,
  type ThinkingLevel,
} from '@mitsuro/api';
import type { PermissionMode } from '@mitsuro/state';

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
  modelInfo?: ModelInfo | null;
  isOpen: boolean;
  onToggle: () => void;
  sessionType?: 'chat' | 'code' | 'hive';
  /** Fires when the last accordion droplet has merged back into the Agent. */
  onCloseComplete?: () => void;
  /** Agent mark — sits in the same 56pt column as the compact pills. */
  agent: ReactNode;
  /** Keep the cluster mounted while close stagger unmounts each GlassView. */
  pillsMounted: boolean;
}

interface ProviderFilterAction {
  id: string;
  label: string;
  icon: ReactNode;
}

const THINKING_ICON_ALPHA: Record<ThinkingLevel, string> = {
  off: '66',
  minimal: '55',
  low: '66',
  medium: 'A6',
  high: 'D9',
  xhigh: 'E8',
  max: '',
  ultra: '',
};

/** Glyph settle only — never spring a GlassView. Transforms stamp iOS glass. */
const LIQUID_OPEN_SPRING = { damping: 24, stiffness: 198, mass: 0.96 };
const MAX_PILL_INDEX = 5;
/** Readable cascade — 20ms read as one pop. */
const OPEN_STAGGER_MS = 58;
const CLOSE_STAGGER_MS = 46;
/** Glyph fade-out before that pill's GlassView unmounts. */
const PILL_SETTLE_MS = 160;
/** Hide glyphs before the glass tile is torn down. */
const GLYPH_FADE_START = 0.42;
const GLYPH_SETTLE_Y = 10;
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

export function pourCloseDurationMs(itemCount: number): number {
  const lastStagger = Math.max(0, itemCount - 1) * CLOSE_STAGGER_MS;
  return lastStagger + PILL_SETTLE_MS;
}

function useDeferredPresence(present: boolean, unmountMs: number): boolean {
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    if (present) {
      setMounted(true);
      return;
    }
    const timer = setTimeout(() => setMounted(false), unmountMs);
    return () => clearTimeout(timer);
  }, [present, unmountMs]);

  return present || mounted;
}

/**
 * Staggered presence. iOS GlassView stamps a copy at every transformed
 * frame, so the glass host never translates or scales.
 *
 * crystallizeOnSettle: a transform-safe stand-in travels first; real glass
 * mounts only after the pour lands, and unmounts before the stand-in leaves.
 */
function usePourMotion({
  isOpen,
  openDelayMs,
  closeDelayMs,
  onCloseSettled,
  progress: progressProp,
  crystallizeOnSettle = false,
}: {
  isOpen: boolean;
  openDelayMs: number;
  closeDelayMs: number;
  onCloseSettled?: () => void;
  progress?: SharedValue<number>;
  crystallizeOnSettle?: boolean;
}): {
  progress: SharedValue<number>;
  materialActive: boolean;
} {
  const fallbackProgress = useSharedValue(0);
  const progress = progressProp ?? fallbackProgress;
  const [materialActive, setMaterialActive] = useState(false);
  const generationRef = useRef(0);
  const onCloseSettledRef = useRef(onCloseSettled);
  onCloseSettledRef.current = onCloseSettled;

  const activateMaterial = useCallback((generation: number) => {
    if (generation !== generationRef.current) return;
    setMaterialActive(true);
  }, []);

  const finishClose = useCallback((generation: number) => {
    if (generation !== generationRef.current) return;
    setMaterialActive(false);
    onCloseSettledRef.current?.();
  }, []);

  useEffect(() => {
    const generation = ++generationRef.current;

    if (isOpen) {
      const appearTimer = setTimeout(() => {
        if (generation !== generationRef.current) return;
        if (!crystallizeOnSettle) setMaterialActive(true);
        progress.value = withSpring(1, LIQUID_OPEN_SPRING, (finished) => {
          if (!finished || !crystallizeOnSettle) return;
          runOnJS(activateMaterial)(generation);
        });
      }, openDelayMs);
      return () => {
        clearTimeout(appearTimer);
      };
    }

    if (crystallizeOnSettle) setMaterialActive(false);

    const retractTimer = setTimeout(() => {
      if (generation !== generationRef.current) return;
      progress.value = withTiming(0, { duration: PILL_SETTLE_MS }, (finished) => {
        if (!finished) return;
        runOnJS(finishClose)(generation);
      });
    }, closeDelayMs);

    return () => {
      clearTimeout(retractTimer);
    };
  }, [
    activateMaterial,
    closeDelayMs,
    crystallizeOnSettle,
    finishClose,
    isOpen,
    openDelayMs,
    progress,
  ]);

  return { progress, materialActive };
}

function FabGlyph({
  progress,
  children,
}: {
  progress: SharedValue<number>;
  children: ReactNode;
}) {
  const glyphStyle = useAnimatedStyle(() => ({
    opacity: interpolate(
      progress.value,
      [0, GLYPH_FADE_START, 1],
      [0, 1, 1],
      Extrapolation.CLAMP,
    ),
    transform: [
      {
        translateY: interpolate(progress.value, [0, 1], [GLYPH_SETTLE_Y, 0]),
      },
    ],
  }));

  return (
    <Animated.View pointerEvents="none" style={glyphStyle}>
      {children}
    </Animated.View>
  );
}

/** Desktop provider filter — keeps staggered spring entrance without scale-to-zero. */
function DesktopFilterPill({
  index,
  itemCount,
  isOpen,
  active,
  label,
  onPress,
  children,
}: {
  index: number;
  itemCount: number;
  isOpen: boolean;
  active: boolean;
  label: string;
  onPress: () => void;
  children: ReactNode;
}) {
  const { theme } = useThemeContext();
  const { progress, materialActive } = usePourMotion({
    isOpen,
    openDelayMs: index * OPEN_STAGGER_MS,
    closeDelayMs: Math.max(0, itemCount - index - 1) * CLOSE_STAGGER_MS,
  });

  const g = theme.colors.glass;

  return (
    <View style={styles.desktopFilterHit}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Filter models by ${label}`}
        onPress={() => {
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          onPress();
        }}
        style={[styles.desktopFilterHit, styles.materialHost, styles.clipHost]}
      >
        <AdaptiveMaterial
          active={materialActive}
          borderRadius={18}
          tone="regular"
          interactive
        />
        <View
          style={[
            styles.providerDockPill,
            {
              backgroundColor: active
                ? theme.colors.thinking + '18'
                : 'transparent',
              borderColor: active
                ? theme.colors.thinking + '80'
                : g.borderLight,
              borderWidth: StyleSheet.hairlineWidth,
              alignItems: 'center',
              justifyContent: 'center',
            },
          ]}
        >
          <FabGlyph progress={progress}>{children}</FabGlyph>
        </View>
      </Pressable>
    </View>
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
  maxIndex = MAX_PILL_INDEX,
  accessibilityLabel,
  accessibilityHint,
  onCloseSettled,
  progress: progressProp,
}: {
  children: ReactNode;
  index: number;
  isOpen: boolean;
  onPress: () => void;
  active?: boolean;
  sideContent?: ReactNode;
  disabled?: boolean;
  /** When true, only as wide as the pill (no full-width row stretch). */
  compact?: boolean;
  maxIndex?: number;
  accessibilityLabel: string;
  accessibilityHint?: string;
  onCloseSettled?: () => void;
  progress?: SharedValue<number>;
}) {
  const { theme } = useThemeContext();
  const { progress, materialActive } = usePourMotion({
    isOpen,
    openDelayMs: index * OPEN_STAGGER_MS,
    closeDelayMs: Math.max(0, maxIndex - index) * CLOSE_STAGGER_MS,
    onCloseSettled,
    progress: progressProp,
    crystallizeOnSettle: true,
  });

  const travelStyle = useAnimatedStyle(() => ({
    transform: [
      {
        translateY: interpolate(
          progress.value,
          [0, 1],
          [pillTravelY(index), 0],
        ),
      },
    ],
  }));

  const g = theme.colors.glass;

  return (
    <View
      style={[
        compact ? styles.pillOuterCompact : styles.pillOuter,
        styles.pointerBoxNone,
      ]}
    >
      {sideContent}
      <View
        pointerEvents="none"
        style={[styles.materialHost, styles.clipHost, styles.pillSlot]}
      >
        <AdaptiveMaterial
          active={materialActive}
          borderRadius={18}
          tone="regular"
          interactive
        />
      </View>
      <Animated.View pointerEvents="box-none" style={[styles.pillTraveler, travelStyle]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={accessibilityLabel}
          accessibilityHint={accessibilityHint}
          accessibilityState={{ disabled: !isOpen || disabled, selected: active }}
          disabled={!isOpen || disabled}
          onPress={onPress}
          style={styles.pill}
        >
          <View
            style={[
              styles.pillFace,
              {
                backgroundColor: active ? theme.colors.thinking + '18' : 'transparent',
                borderColor: active ? theme.colors.thinking + '80' : g.borderLight,
              },
            ]}
          >
            <FabGlyph progress={progress}>{children}</FabGlyph>
          </View>
        </Pressable>
      </Animated.View>
    </View>
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
  children: ReactNode;
  active?: boolean;
  size?: number;
  accessibilityLabel?: string;
  closeStaggerMs?: number;
}) {
  const { theme } = useThemeContext();
  const { progress, materialActive } = usePourMotion({
    isOpen,
    openDelayMs: index * OPEN_STAGGER_MS,
    closeDelayMs: Math.max(0, itemCount - index - 1) * closeStaggerMs,
  });

  const g = theme.colors.glass;

  return (
    <View
      style={[
        styles.inlineActionOuter,
        styles.pointerBoxNone,
        { width: size, height: size },
      ]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={accessibilityLabel}
        disabled={!isOpen}
        onPress={() => {
          Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
          onPress();
        }}
        style={[
          styles.materialHost,
          styles.clipHost,
          { borderRadius: size >= 56 ? 18 : 14 },
        ]}
      >
        <AdaptiveMaterial
          active={materialActive}
          borderRadius={size >= 56 ? 18 : 14}
          tone="regular"
          interactive
        />
        <View
          style={[
            styles.inlinePill,
            {
              width: size,
              height: size,
              borderRadius: size >= 56 ? 18 : 14,
              backgroundColor: active ? theme.colors.thinking + '18' : 'transparent',
              borderColor: active ? theme.colors.thinking + '80' : g.borderLight,
            },
          ]}
        >
          <FabGlyph progress={progress}>{children}</FabGlyph>
        </View>
      </Pressable>
    </View>
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
  children: ReactNode;
}) {
  const { theme } = useThemeContext();
  const animationIndex = Math.max(0, itemCount - index - 1);
  const { progress: revealProgress, materialActive } = usePourMotion({
    isOpen,
    openDelayMs: animationIndex * OPEN_STAGGER_MS,
    closeDelayMs: index * CLOSE_STAGGER_MS,
  });
  const editProgress = useSharedValue(0);
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
    const zIndex = isActiveDrag ? 20 : editProgress.value ? 4 : 0;
    const lift = interpolate(editProgress.value, [0, 1], [0, -2]);
    const dragLift = interpolate(dragging.value, [0, 1], [0, -4]);
    const tilt = interpolate(
      editProgress.value,
      [0, 1],
      [0, index % 2 === 0 ? -1.2 : 1.2],
      Extrapolation.CLAMP,
    );

    return {
      zIndex,
      transform: [
        {
          translateX: isActiveDrag ? sharedDragX.value : reorderX.value,
        },
        { translateY: lift + dragLift },
        { scale: 1 + editProgress.value * 0.02 + dragging.value * 0.05 },
        { rotate: `${tilt}deg` },
      ],
    };
  });

  const g = theme.colors.glass;

  const pill = (
    <Animated.View
      style={[
        styles.providerDockCell,
        { pointerEvents: isOpen ? "auto" : "none" },
        animatedStyle,
      ]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Filter models by ${provider.label}`}
        disabled={!isOpen}
        style={[styles.providerDockPressable, styles.materialHost, styles.clipHost]}
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
        <AdaptiveMaterial
          active={materialActive}
          borderRadius={18}
          tone="regular"
          interactive
        />
        <View
          style={[
            styles.inlinePill,
            styles.providerDockPill,
            {
              backgroundColor: active ? theme.colors.thinking + '18' : 'transparent',
              borderColor: editMode || active ? theme.colors.thinking + '70' : g.borderLight,
            },
          ]}
        >
          <FabGlyph progress={revealProgress}>{children}</FabGlyph>
        </View>
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
  modelInfo = null,
  isOpen,
  onToggle,
  sessionType = 'code',
  onCloseComplete,
  agent,
  pillsMounted,
}: AccordionControlsProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const gooey0 = useSharedValue(0);
  const gooey1 = useSharedValue(0);
  const gooey2 = useSharedValue(0);
  const gooey3 = useSharedValue(0);
  const gooey4 = useSharedValue(0);
  const gooey5 = useSharedValue(0);
  const pillProgresses: GooeyProgresses = [
    gooey0,
    gooey1,
    gooey2,
    gooey3,
    gooey4,
    gooey5,
  ];
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
  const providerDragIndex = useSharedValue(-1);
  const providerDropIndex = useSharedValue(-1);
  const providerDragX = useSharedValue(0);
  const providerDragScrollDelta = useSharedValue(0);
  const hasWorkMode = sessionType === 'code';
  const modelManagedByHive = sessionType === 'hive';
  const maxPillIndex = hasWorkMode ? 5 : 4;
  const modelPillIndex = maxPillIndex;
  const attachPillIndex = hasWorkMode ? 4 : 3;
  const thinkingSupported = supportsThinking(modelInfo ?? model);

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
    if (!thinkingSupported) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const next = cycleThinkingLevel(thinkingLevel, modelInfo ?? model);
    onThinkingChange(next);
  };

  const handleMode = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onModeToggle();
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
    if (modelManagedByHive) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onModelSelect();
  };

  const t = theme.colors;
  const fabAccent = t.thinking;
  const thinkingColor = thinkingLevel === 'off'
    ? `${fabAccent}${THINKING_ICON_ALPHA.off}`
    : `${fabAccent}${THINKING_ICON_ALPHA[thinkingLevel]}`;
  const providerDockOpen = !modelManagedByHive && modelPickerOpen && isOpen;

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

  const desktopFiltersOpen = isDesktop && !modelManagedByHive &&
    modelPickerOpen && isOpen;
  const desktopFilterCount = providerFilters.length;
  const attachActionsOpen = attachPickerOpen && isOpen;
  const providerDockMounted = useDeferredPresence(
    providerDockOpen,
    pourCloseDurationMs(providerFilters.length),
  );
  const attachActionsMounted = useDeferredPresence(
    attachActionsOpen,
    pourCloseDurationMs(ATTACH_ACTION_COUNT),
  );
  const desktopFiltersMounted = useDeferredPresence(
    desktopFiltersOpen,
    pourCloseDurationMs(desktopFilterCount),
  );
  const onCloseCompleteRef = useRef(onCloseComplete);
  onCloseCompleteRef.current = onCloseComplete;

  const handleLastPillSettled = useCallback(() => {
    if (isOpen) return;
    onCloseCompleteRef.current?.();
  }, [isOpen]);

  const mergePillCount = maxPillIndex + 1;
  const gooeyPillCount = mergePillCount;
  const sideColumnHeight = mergePillCount * 56 + Math.max(0, mergePillCount - 1) * 10;

  return (
    <View style={[styles.container, styles.pointerBoxNone]}>
      <View style={[styles.chromeRow, styles.pointerBoxNone]}>
        {pillsMounted ? (
          <View
            style={[
              styles.sideColumn,
              styles.pointerBoxNone,
              { height: sideColumnHeight },
            ]}
          >
            <View style={[styles.sideRow, styles.pointerBoxNone]}>
              {isDesktop ? (
                <View style={[styles.desktopModelRow, styles.pointerBoxNone]}>
                  {desktopFiltersMounted
                    ? providerFilters.map((provider, visualIndex) => {
                        const active = selectedProviderFilter === provider.id;
                        const staggerIndex = desktopFilterCount - visualIndex - 1;
                        return (
                          <DesktopFilterPill
                            key={provider.id}
                            index={staggerIndex}
                            itemCount={desktopFilterCount}
                            isOpen={desktopFiltersOpen}
                            active={active}
                            label={provider.label}
                            onPress={() => onProviderFilterToggle(provider.id)}
                          >
                            {provider.icon}
                          </DesktopFilterPill>
                        );
                      })
                    : null}
                </View>
              ) : (
                <Animated.View
                  style={[
                    styles.modelFilterDock,
                    {
                      pointerEvents: providerDockOpen ? "box-none" : "none",
                    },
                  ]}
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
                    scrollEnabled={providerDockOpen && !providerDragging}
                    showsHorizontalScrollIndicator={false}
                    scrollEventThrottle={16}
                    onLayout={handleProviderScrollLayout}
                    onContentSizeChange={handleProviderContentSizeChange}
                    onScroll={handleProviderScroll}
                    style={styles.modelFilterScroll}
                    contentContainerStyle={styles.modelFilterContent}
                  >
                    {providerDockMounted
                      ? providerFilters.map((provider, visualIndex) => (
                      <ProviderDockPill
                        key={provider.id}
                        provider={provider}
                        index={visualIndex}
                        itemCount={providerFilters.length}
                        isOpen={providerDockOpen}
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
                        ))
                      : null}
                  </ScrollView>
                </Animated.View>
              )}
            </View>
            <View style={[styles.sideRow, styles.pointerBoxNone]}>
              <View
                style={[
                  styles.attachActions,
                  {
                    pointerEvents: attachPickerOpen ? "box-none" : "none",
                  },
                ]}
              >
                {attachActionsMounted ? (
                  <>
                    <InlineActionPill
                      index={2}
                      itemCount={ATTACH_ACTION_COUNT}
                      isOpen={attachActionsOpen}
                      onPress={onPickFile}
                      accessibilityLabel="Attach file"
                    >
                      <FileText size={23} color={fabAccent} strokeWidth={1.7} />
                    </InlineActionPill>
                    <InlineActionPill
                      index={1}
                      itemCount={ATTACH_ACTION_COUNT}
                      isOpen={attachActionsOpen}
                      onPress={onPickCamera}
                      accessibilityLabel="Take photo"
                    >
                      <Camera size={23} color={fabAccent} strokeWidth={1.7} />
                    </InlineActionPill>
                    <InlineActionPill
                      index={0}
                      itemCount={ATTACH_ACTION_COUNT}
                      isOpen={attachActionsOpen}
                      onPress={onPickPhoto}
                      accessibilityLabel="Choose photo"
                    >
                      <ImageIcon size={23} color={fabAccent} strokeWidth={1.7} />
                    </InlineActionPill>
                  </>
                ) : null}
              </View>
            </View>
          </View>
        ) : null}
        <View pointerEvents="box-none" style={styles.mergeColumn}>
          <View
            pointerEvents="none"
            style={[
              styles.gooeyLayer,
              {
                width: gooeyCanvasWidth(),
                height: gooeyCanvasHeight(gooeyPillCount),
                right: -GOOEY_PAD,
                bottom: -GOOEY_PAD,
              },
            ]}
          >
            <FabGooeyLayer
              progresses={pillProgresses}
              pillCount={gooeyPillCount}
              fill={gooeyFill(theme.scheme)}
            />
          </View>
          <GestureDetector gesture={swipeDown}>
            <View style={styles.mergePills}>
              {pillsMounted ? (
                <>
          <AccordionPill
            index={modelPillIndex}
            isOpen={isOpen}
            onPress={handleModel}
            active={!modelManagedByHive && modelPickerOpen}
            disabled={modelManagedByHive}
            compact
            maxIndex={maxPillIndex}
            progress={pillProgresses[modelPillIndex]}
            accessibilityLabel={modelManagedByHive
              ? "Hive-managed model"
              : "Choose model"}
            accessibilityHint={modelManagedByHive
              ? "This conversation uses its configured Hive model"
              : undefined}
          >
            <Bot
              size={24}
              color={fabAccent}
              strokeWidth={1.6}
            />
          </AccordionPill>

          <AccordionPill
            index={attachPillIndex}
            isOpen={isOpen}
            onPress={handleAttach}
            active={attachPickerOpen}
            compact
            maxIndex={maxPillIndex}
            progress={pillProgresses[attachPillIndex]}
            accessibilityLabel="Add attachment"
          >
            <Paperclip
              size={24}
              color={fabAccent}
              strokeWidth={1.6}
            />
          </AccordionPill>

          {hasWorkMode ? (
            <AccordionPill
              index={3}
              isOpen={isOpen}
              onPress={handleMode}
              compact
              maxIndex={maxPillIndex}
              progress={pillProgresses[3]}
              accessibilityLabel={mode === 'build' ? 'Build mode' : 'Plan mode'}
              accessibilityHint="Switch between Build and Plan"
            >
              {mode === 'build' ? (
                <Hammer size={24} color={fabAccent} strokeWidth={1.6} />
              ) : (
                <Compass size={24} color={fabAccent} strokeWidth={1.6} />
              )}
            </AccordionPill>
          ) : null}

          <AccordionPill
            index={2}
            isOpen={isOpen}
            onPress={handlePermissionMode}
            compact
            maxIndex={maxPillIndex}
            progress={pillProgresses[2]}
            accessibilityLabel={permissionMode === 'supervised' ? 'Supervised permissions' : 'Autonomous permissions'}
            accessibilityHint="Switch permission mode"
          >
            {permissionMode === 'supervised' ? (
              <ShieldCheck size={24} color={fabAccent} strokeWidth={1.6} />
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
            compact
            maxIndex={maxPillIndex}
            progress={pillProgresses[1]}
            accessibilityLabel={fastModeSupported ? (fastModeEnabled ? 'Fast mode on' : 'Fast mode off') : 'Fast mode unavailable for this model'}
            accessibilityHint={fastModeSupported ? 'Toggle provider speed mode' : undefined}
          >
            <Zap
              size={24}
              color={fastModeSupported ? fabAccent : `${fabAccent}66`}
              strokeWidth={1.6}
            />
          </AccordionPill>

          <AccordionPill
            index={0}
            isOpen={isOpen}
            onPress={handleThinking}
            disabled={!thinkingSupported}
            compact
            maxIndex={maxPillIndex}
            progress={pillProgresses[0]}
            onCloseSettled={handleLastPillSettled}
            accessibilityLabel={thinkingSupported ? `Thinking ${thinkingLevel}` : 'Thinking unavailable for this model'}
            accessibilityHint={thinkingSupported ? 'Cycle thinking level' : undefined}
          >
            <Brain
              size={24}
              color={thinkingSupported ? thinkingColor : `${fabAccent}66`}
              strokeWidth={1.6}
            />
          </AccordionPill>
                </>
              ) : null}
              {agent}
            </View>
          </GestureDetector>
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  materialHost: {
    position: 'relative',
  },
  clipHost: {
    overflow: 'hidden',
    borderRadius: 18,
  },
  pointerBoxNone: {
    pointerEvents: 'box-none',
  },
  container: {
    width: '100%',
    alignItems: 'flex-end',
  },
  chromeRow: {
    width: '100%',
    flexDirection: 'row',
    alignItems: 'flex-end',
    justifyContent: 'flex-end',
  },
  sideColumn: {
    flex: 1,
    minWidth: 0,
    gap: 10,
    marginBottom: 66,
    justifyContent: 'flex-start',
  },
  sideRow: {
    height: 56,
    width: '100%',
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
  },
  mergeColumn: {
    width: 56,
    position: 'relative',
    overflow: 'visible',
  },
  gooeyLayer: {
    position: 'absolute',
    zIndex: 0,
  },
  mergePills: {
    width: 56,
    gap: 10,
    alignItems: 'center',
    overflow: 'visible',
    zIndex: 1,
  },
  pillOuter: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    height: 56,
    width: '100%',
    position: 'relative',
    overflow: 'visible',
  },
  pillOuterCompact: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    height: 56,
    width: PROVIDER_PILL_SIZE,
    flexShrink: 0,
    position: 'relative',
    overflow: 'visible',
  },
  pillSlot: {
    position: 'absolute',
    right: 0,
    top: 0,
    width: 56,
    height: 56,
  },
  pillTraveler: {
    width: 56,
    height: 56,
  },
  pillFace: {
    width: 56,
    height: 56,
    borderRadius: 18,
    justifyContent: 'center',
    alignItems: 'center',
    borderWidth: StyleSheet.hairlineWidth,
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
    width: PROVIDER_PILL_SIZE,
    height: PROVIDER_PILL_SIZE,
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
  pill: {
    width: 56,
    height: 56,
    borderRadius: 18,
    justifyContent: 'center',
    alignItems: 'center',
    borderWidth: StyleSheet.hairlineWidth,
  },
});
