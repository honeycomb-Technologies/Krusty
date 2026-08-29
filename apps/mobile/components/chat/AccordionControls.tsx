import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from 'react';
import {
  View,
  Pressable,
  Platform,
  ScrollView,
  StyleSheet,
  type GestureResponderEvent,
  type LayoutChangeEvent,
  type NativeScrollEvent,
  type NativeSyntheticEvent,
} from 'react-native';
import {
  AdaptiveMaterial,
  useAdaptiveMaterialMotionSafe,
} from '../ui/AdaptiveMaterial';
import { FabGooeyLayer } from './FabGooeyLayer';
import {
  FAB_GAP,
  FAB_MATERIAL_CROSSFADE_MS,
  FAB_PILL,
  FAB_POUR_CLOSE_MS,
  FAB_POUR_GLYPH_REVEAL_END,
  FAB_POUR_GLYPH_SETTLE_Y,
  FAB_POUR_OPEN_SPRING,
  FAB_POUR_OPEN_STAGGER_MS,
  FAB_STEP,
  GOOEY_PAD,
  MAX_GOOEY_PILLS,
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
  modelPopoverProgress: SharedValue<number>;
  modelPopoverCoverOpacity: SharedValue<number>;
  onModelPopoverMaterialActiveChange: (active: boolean) => void;
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
  /** Keep the cluster mounted until the last traveling surface rejoins Agent. */
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

/** Shared surface/glyph spring. Native glass exists only in fixed endpoints. */
const MAX_PILL_INDEX = 5;
/** Readable cascade — 20ms read as one pop. */
const OPEN_STAGGER_MS = FAB_POUR_OPEN_STAGGER_MS;
const CLOSE_STAGGER_MS = 46;
/** Keep the scroll rail still until the last opening spring has visibly settled. */
const PILL_OPEN_SETTLE_MS = 440;
/** Glyph fade-out before that traveling pill unmounts. */
const ATTACH_ACTION_COUNT = 3;
const DOCK_FADE_WIDTH = 34;
const MODEL_BUTTON_GAP = 10;
const PROVIDER_PILL_SIZE = 56;
// Keep every branch on the same 66pt center-to-center rhythm so its visible
// controls and the shared Skia bridge occupy the exact same path.
const PROVIDER_PILL_GAP = FAB_GAP;
const PROVIDER_PILL_STEP = PROVIDER_PILL_SIZE + PROVIDER_PILL_GAP;
const PROVIDER_CONTENT_PADDING_LEFT = DOCK_FADE_WIDTH - PROVIDER_PILL_GAP / 2;
const PROVIDER_CONTENT_PADDING_RIGHT = MODEL_BUTTON_GAP - PROVIDER_PILL_GAP / 2;
const PROVIDER_DOCK_HEIGHT = 72;
const PROVIDER_AUTO_SCROLL_EDGE_WIDTH = 52;
const PROVIDER_AUTO_SCROLL_MAX_STEP = 18;
const PROVIDER_REORDER_SPRING_CONFIG = { damping: 24, stiffness: 420, mass: 0.55 };
const PROVIDER_REORDER_LONG_PRESS_MS = 460;
const NATIVE_MATERIAL_COMMIT_MS = Platform.OS === 'ios' ? 20 : 0;
const usePourMotionEffect = Platform.OS === 'ios' ? useLayoutEffect : useEffect;

export function pourCloseDurationMs(itemCount: number): number {
  const lastStagger = Math.max(0, itemCount - 1) * CLOSE_STAGGER_MS;
  return lastStagger + FAB_POUR_CLOSE_MS + NATIVE_MATERIAL_COMMIT_MS;
}

function pourOpenDurationMs(itemCount: number): number {
  const lastStagger = Math.max(0, itemCount - 1) * OPEN_STAGGER_MS;
  return lastStagger + PILL_OPEN_SETTLE_MS;
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

function useGooeyProgresses(): GooeyProgresses {
  return [
    useSharedValue(0),
    useSharedValue(0),
    useSharedValue(0),
    useSharedValue(0),
    useSharedValue(0),
    useSharedValue(0),
  ];
}

/**
 * Staggered presence for every branch of the FAB. The moving surface is always
 * the transform-safe graphite/Skia silhouette. Native glass mounts only in a
 * fixed destination sibling after the spring settles; an opaque graphite
 * cover then yields without ever animating the GlassView itself.
 */
function usePourMotion({
  isOpen,
  openDelayMs,
  closeDelayMs,
  onCloseSettled,
  progress: progressProp,
  coverOpacity: coverOpacityProp,
  materialAllowed = true,
  onMaterialActiveChange,
}: {
  isOpen: boolean;
  openDelayMs: number;
  closeDelayMs: number;
  onCloseSettled?: () => void;
  progress?: SharedValue<number>;
  coverOpacity?: SharedValue<number>;
  materialAllowed?: boolean;
  onMaterialActiveChange?: (active: boolean) => void;
}): {
  progress: SharedValue<number>;
  materialActive: boolean;
  coverStyle: ReturnType<typeof useAnimatedStyle>;
} {
  const fallbackProgress = useSharedValue(0);
  const fallbackCoverOpacity = useSharedValue(1);
  const progress = progressProp ?? fallbackProgress;
  const coverOpacity = coverOpacityProp ?? fallbackCoverOpacity;
  const [materialActive, setMaterialActive] = useState(false);
  const materialActiveRef = useRef(false);
  const generationRef = useRef(0);
  const onCloseSettledRef = useRef(onCloseSettled);
  const onMaterialActiveChangeRef = useRef(onMaterialActiveChange);
  const isOpenRef = useRef(isOpen);
  const materialAllowedRef = useRef(materialAllowed);
  onCloseSettledRef.current = onCloseSettled;
  onMaterialActiveChangeRef.current = onMaterialActiveChange;
  isOpenRef.current = isOpen;
  materialAllowedRef.current = materialAllowed;
  materialActiveRef.current = materialActive;

  const activateMaterial = useCallback((generation: number) => {
    if (generation !== generationRef.current) return;
    if (!isOpenRef.current || !materialAllowedRef.current) return;
    setMaterialActive(true);
  }, []);

  const finishClose = useCallback((generation: number) => {
    if (generation !== generationRef.current) return;
    onCloseSettledRef.current?.();
  }, []);

  usePourMotionEffect(() => {
    const generation = ++generationRef.current;
    coverOpacity.value = 1;
    setMaterialActive(false);

    if (isOpen) {
      const appearTimer = setTimeout(() => {
        if (generation !== generationRef.current) return;
        progress.value = withSpring(1, FAB_POUR_OPEN_SPRING, (finished) => {
          if (!finished) return;
          runOnJS(activateMaterial)(generation);
        });
      }, openDelayMs);
      return () => {
        clearTimeout(appearTimer);
      };
    }

    let retractTimer: ReturnType<typeof setTimeout> | null = null;
    let materialCommitFrame = 0;
    const beginRetraction = () => {
      retractTimer = setTimeout(() => {
        if (generation !== generationRef.current) return;
        progress.value = withTiming(0, { duration: FAB_POUR_CLOSE_MS }, (finished) => {
          if (!finished) return;
          runOnJS(finishClose)(generation);
        });
      }, closeDelayMs);
    };

    if (materialActiveRef.current) {
      // `isOpen` already rendered false, so the fixed GlassView is gone. Wait
      // one committed frame with graphite restored before moving the traveler.
      materialCommitFrame = requestAnimationFrame(beginRetraction);
    } else {
      beginRetraction();
    }

    return () => {
      if (retractTimer) clearTimeout(retractTimer);
      if (materialCommitFrame) cancelAnimationFrame(materialCommitFrame);
    };
  }, [
    closeDelayMs,
    coverOpacity,
    finishClose,
    isOpen,
    openDelayMs,
    progress,
    activateMaterial,
  ]);

  useEffect(() => {
    if (!isOpen || !materialAllowed) {
      coverOpacity.value = 1;
      setMaterialActive(false);
      return;
    }

    // If the startup motion gate clears after this spring has already landed,
    // promote the fixed endpoint without replaying the pour.
    if (progress.value >= 0.999) setMaterialActive(true);
  }, [coverOpacity, isOpen, materialAllowed, progress]);

  usePourMotionEffect(() => {
    onMaterialActiveChangeRef.current?.(
      isOpen && materialAllowed && materialActive,
    );
  }, [isOpen, materialActive, materialAllowed]);

  useEffect(() => {
    if (!isOpen || !materialAllowed || !materialActive) return;

    // Two committed paints guarantee the fixed sibling (and, for the model
    // panel, its parent state handoff) exists before graphite starts yielding.
    let fadeFrame = 0;
    const commitFrame = requestAnimationFrame(() => {
      fadeFrame = requestAnimationFrame(() => {
        coverOpacity.value = withTiming(0, {
          duration: FAB_MATERIAL_CROSSFADE_MS,
        });
      });
    });

    return () => {
      cancelAnimationFrame(commitFrame);
      if (fadeFrame) cancelAnimationFrame(fadeFrame);
    };
  }, [coverOpacity, isOpen, materialActive, materialAllowed]);

  const coverStyle = useAnimatedStyle(() => ({
    opacity: coverOpacity.value,
  }));

  return { progress, materialActive, coverStyle };
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
      [0, FAB_POUR_GLYPH_REVEAL_END, 1],
      [0, 1, 1],
      Extrapolation.CLAMP,
    ),
    transform: [
      {
        translateY: interpolate(
          progress.value,
          [0, 1],
          [FAB_POUR_GLYPH_SETTLE_Y, 0],
        ),
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
  progress: progressProp,
}: {
  index: number;
  itemCount: number;
  isOpen: boolean;
  active: boolean;
  label: string;
  onPress: () => void;
  children: ReactNode;
  progress?: SharedValue<number>;
}) {
  const { theme } = useThemeContext();
  const materialMotionSafe = useAdaptiveMaterialMotionSafe();
  const { progress, materialActive, coverStyle } = usePourMotion({
    isOpen,
    openDelayMs: index * OPEN_STAGGER_MS,
    closeDelayMs: Math.max(0, itemCount - index - 1) * CLOSE_STAGGER_MS,
    progress: progressProp,
    materialAllowed: Platform.OS === 'ios' && materialMotionSafe,
  });

  const travelStyle = useAnimatedStyle(() => ({
    transform: [{
      translateX: interpolate(
        progress.value,
        [0, 1],
        [(index + 1) * PROVIDER_PILL_STEP, 0],
      ),
    }],
  }));

  const g = theme.colors.glass;

  return (
    <View style={styles.desktopFilterHit}>
      <AdaptiveMaterial
        active={isOpen && materialActive}
        borderRadius={18}
        tone="regular"
        fallbackColor={gooeyFill(theme.scheme)}
        liquidGlassOnly
        respectMotionGate
      />
      <Animated.View style={[styles.desktopFilterHit, travelStyle]}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Filter models by ${label}`}
          onPress={() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            onPress();
          }}
          style={styles.desktopFilterHit}
        >
          <View
            style={[
              styles.providerDockPill,
              styles.fabSurface,
              {
                backgroundColor: Platform.OS === 'ios'
                  ? 'transparent'
                  : gooeyFill(theme.scheme),
                borderColor: active
                  ? theme.colors.thinking + '80'
                  : g.borderLight,
                alignItems: 'center',
                justifyContent: 'center',
              },
            ]}
          >
            {Platform.OS === 'ios' ? (
              <Animated.View
                pointerEvents="none"
                style={[
                  styles.graphiteCover,
                  { borderRadius: 18, backgroundColor: gooeyFill(theme.scheme) },
                  coverStyle,
                ]}
              />
            ) : null}
            <FabGlyph progress={progress}>{children}</FabGlyph>
          </View>
        </Pressable>
      </Animated.View>
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
  const materialMotionSafe = useAdaptiveMaterialMotionSafe();
  const { progress, materialActive, coverStyle } = usePourMotion({
    isOpen,
    openDelayMs: index * OPEN_STAGGER_MS,
    closeDelayMs: Math.max(0, maxIndex - index) * CLOSE_STAGGER_MS,
    onCloseSettled,
    progress: progressProp,
    materialAllowed: Platform.OS === 'ios' && materialMotionSafe,
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
      <AdaptiveMaterial
        active={isOpen && materialActive}
        borderRadius={18}
        tone="regular"
        fallbackColor={gooeyFill(theme.scheme)}
        liquidGlassOnly
        respectMotionGate
      />
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
                backgroundColor: Platform.OS === 'ios'
                  ? 'transparent'
                  : gooeyFill(theme.scheme),
                borderColor: active ? theme.colors.thinking + '80' : g.borderLight,
              },
            ]}
          >
            {Platform.OS === 'ios' ? (
              <Animated.View
                pointerEvents="none"
                style={[
                  styles.graphiteCover,
                  { borderRadius: 18, backgroundColor: gooeyFill(theme.scheme) },
                  coverStyle,
                ]}
              />
            ) : null}
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
  progress: progressProp,
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
  progress?: SharedValue<number>;
}) {
  const { theme } = useThemeContext();
  const materialMotionSafe = useAdaptiveMaterialMotionSafe();
  const { progress, materialActive, coverStyle } = usePourMotion({
    isOpen,
    openDelayMs: index * OPEN_STAGGER_MS,
    closeDelayMs: Math.max(0, itemCount - index - 1) * closeStaggerMs,
    progress: progressProp,
    materialAllowed: Platform.OS === 'ios' && materialMotionSafe,
  });

  const travelStyle = useAnimatedStyle(() => ({
    transform: [{
      translateX: interpolate(
        progress.value,
        [0, 1],
        [(index + 1) * FAB_STEP, 0],
      ),
    }],
  }));

  const g = theme.colors.glass;

  return (
    <View
      style={[
        styles.inlineActionOuter,
        styles.pointerBoxNone,
        { width: size, height: size },
      ]}
    >
      <AdaptiveMaterial
        active={isOpen && materialActive}
        borderRadius={size >= 56 ? 18 : 14}
        tone="regular"
        fallbackColor={gooeyFill(theme.scheme)}
        liquidGlassOnly
        respectMotionGate
      />
      <Animated.View style={travelStyle}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={accessibilityLabel}
          disabled={!isOpen}
          onPress={() => {
            Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            onPress();
          }}
        >
          <View
            style={[
              styles.inlinePill,
              styles.fabSurface,
              {
                width: size,
                height: size,
                borderRadius: size >= 56 ? 18 : 14,
                backgroundColor: Platform.OS === 'ios'
                  ? 'transparent'
                  : gooeyFill(theme.scheme),
                borderColor: active ? theme.colors.thinking + '80' : g.borderLight,
              },
            ]}
          >
            {Platform.OS === 'ios' ? (
              <Animated.View
                pointerEvents="none"
                style={[
                  styles.graphiteCover,
                  {
                    borderRadius: size >= 56 ? 18 : 14,
                    backgroundColor: gooeyFill(theme.scheme),
                  },
                  coverStyle,
                ]}
              />
            ) : null}
            <FabGlyph progress={progress}>{children}</FabGlyph>
          </View>
        </Pressable>
      </Animated.View>
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
  progress: progressProp,
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
  progress?: SharedValue<number>;
}) {
  const { theme } = useThemeContext();
  const animationIndex = Math.max(0, itemCount - index - 1);
  const { progress: revealProgress } = usePourMotion({
    isOpen,
    openDelayMs: animationIndex * OPEN_STAGGER_MS,
    closeDelayMs: index * CLOSE_STAGGER_MS,
    progress: progressProp,
    // These pills live in a horizontally scrolling/reorderable rail, so there
    // is no truly fixed native destination. Keep them graphite at all times.
    materialAllowed: false,
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
    const revealTravel = interpolate(
      revealProgress.value,
      [0, 1],
      [(animationIndex + 1) * PROVIDER_PILL_STEP, 0],
    );
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
          translateX: revealTravel + (
            isActiveDrag ? sharedDragX.value : reorderX.value
          ),
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
        <View
          style={[
            styles.inlinePill,
            styles.providerDockPill,
            styles.fabSurface,
            {
              backgroundColor: gooeyFill(theme.scheme),
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
  modelPopoverProgress,
  modelPopoverCoverOpacity,
  onModelPopoverMaterialActiveChange,
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
  const materialMotionSafe = useAdaptiveMaterialMotionSafe();
  const pillProgresses = useGooeyProgresses();
  const providerProgresses = useGooeyProgresses();
  const attachProgresses = useGooeyProgresses();
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
  const providerPourFrameRef = useRef<number | null>(null);
  const providerPourSettleTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [providerEditMode, setProviderEditMode] = useState(false);
  const [providerDragging, setProviderDragging] = useState(false);
  const [providerPourOpen, setProviderPourOpen] = useState(false);
  const [providerPourSettled, setProviderPourSettled] = useState(false);
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

  const alignProviderDockToEnd = useCallback(() => {
    const metrics = providerScrollMetricsRef.current;
    metrics.contentWidth = Math.max(
      metrics.contentWidth,
      estimateProviderContentWidth(),
    );
    metrics.x = Math.max(0, metrics.contentWidth - metrics.width);
    providerScrollRef.current?.scrollToEnd({ animated: false });
  }, [estimateProviderContentWidth]);

  const clearProviderPourSchedule = useCallback(() => {
    if (providerPourFrameRef.current !== null) {
      cancelAnimationFrame(providerPourFrameRef.current);
      providerPourFrameRef.current = null;
    }
    if (providerPourSettleTimerRef.current !== null) {
      clearTimeout(providerPourSettleTimerRef.current);
      providerPourSettleTimerRef.current = null;
    }
  }, []);

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
    clearProviderPourSchedule();
  }, [clearProviderPourSchedule, stopProviderAutoScroll]);

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

  // The large list begins on the nearest provider beat. Its fixed shell and
  // content use the same spring/reveal language without waiting behind the
  // complete provider cascade or growing from a disconnected dark seed.
  usePourMotion({
    isOpen: providerPourOpen,
    openDelayMs: 0,
    closeDelayMs: 0,
    progress: modelPopoverProgress,
    coverOpacity: modelPopoverCoverOpacity,
    materialAllowed: Platform.OS === 'ios' && materialMotionSafe,
    onMaterialActiveChange: onModelPopoverMaterialActiveChange,
  });

  useEffect(() => {
    clearProviderPourSchedule();
    setProviderPourSettled(false);

    if (!providerDockOpen) {
      providerDockOpenRef.current = false;
      setProviderEditMode(false);
      clearProviderEditExitTimer();
      resetProviderDockDragState();
      // A scrolled provider rail must rejoin the model trigger from its aligned
      // end position. Give ScrollView one frame to apply the no-animation snap
      // before the shared close progress begins.
      alignProviderDockToEnd();
      if (!providerPourOpen) return clearProviderPourSchedule;
      providerPourFrameRef.current = requestAnimationFrame(() => {
        providerPourFrameRef.current = null;
        setProviderPourOpen(false);
      });
      return clearProviderPourSchedule;
    }

    providerDockOpenRef.current = true;
    if (providerPourOpen) {
      alignProviderDockToEnd();
      providerPourSettleTimerRef.current = setTimeout(() => {
        providerPourSettleTimerRef.current = null;
        setProviderPourSettled(true);
      }, pourOpenDurationMs(providerFilters.length));
      return clearProviderPourSchedule;
    }

    providerPourFrameRef.current = requestAnimationFrame(() => {
      alignProviderDockToEnd();
      measureProviderDock();
      providerPourFrameRef.current = requestAnimationFrame(() => {
        providerPourFrameRef.current = null;
        setProviderPourOpen(true);
        providerPourSettleTimerRef.current = setTimeout(() => {
          providerPourSettleTimerRef.current = null;
          setProviderPourSettled(true);
        }, pourOpenDurationMs(providerFilters.length));
      });
    });
    return clearProviderPourSchedule;
  }, [
    alignProviderDockToEnd,
    clearProviderEditExitTimer,
    clearProviderPourSchedule,
    measureProviderDock,
    providerDockOpen,
    providerFilters.length,
    providerPourOpen,
    resetProviderDockDragState,
  ]);

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
      providerScrollRef.current?.scrollToEnd({ animated: false });
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
    if (!providerPourSettled) return;
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
  }, [clearProviderEditExitTimer, estimateProviderContentWidth, measureProviderDock, providerDragScrollDelta, providerPourSettled, scheduleProviderAutoScroll]);

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

  const desktopFiltersOpen = isDesktop && providerDockOpen;
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
  const providerBranchCount = Math.min(providerFilters.length, MAX_GOOEY_PILLS);
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
              <View
                pointerEvents="none"
                style={[
                  styles.branchGooeyViewport,
                  {
                    height: gooeyCanvasHeight(providerBranchCount, 'horizontal'),
                  },
                ]}
              >
                <View
                  style={[
                    styles.branchGooeyLayer,
                    {
                      width: gooeyCanvasWidth(providerBranchCount, 'horizontal'),
                      height: gooeyCanvasHeight(providerBranchCount, 'horizontal'),
                    },
                  ]}
                >
                  <FabGooeyLayer
                    progresses={providerProgresses}
                    pillCount={providerBranchCount}
                    fill={gooeyFill(theme.scheme)}
                    orientation="horizontal"
                  />
                </View>
              </View>
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
                            isOpen={providerPourOpen}
                            active={active}
                            label={provider.label}
                            onPress={() => onProviderFilterToggle(provider.id)}
                            progress={staggerIndex < MAX_GOOEY_PILLS
                              ? providerProgresses[staggerIndex]
                              : undefined}
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
                      pointerEvents: providerPourOpen ? "box-none" : "none",
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
                    scrollEnabled={providerDockOpen && providerPourSettled && !providerDragging}
                    showsHorizontalScrollIndicator={false}
                    scrollEventThrottle={16}
                    onLayout={handleProviderScrollLayout}
                    onContentSizeChange={handleProviderContentSizeChange}
                    onScroll={handleProviderScroll}
                    style={styles.modelFilterScroll}
                    contentContainerStyle={styles.modelFilterContent}
                  >
                    {providerDockMounted
                      ? providerFilters.map((provider, visualIndex) => {
                        const branchIndex = providerFilters.length - visualIndex - 1;
                        return (
                      <ProviderDockPill
                        key={provider.id}
                        provider={provider}
                        index={visualIndex}
                        itemCount={providerFilters.length}
                        isOpen={providerPourOpen}
                        active={selectedProviderFilter === provider.id}
                        editMode={enableProviderReorder && providerPourSettled && providerEditMode}
                        canReorder={enableProviderReorder && providerPourSettled}
                        dragIndex={providerDragIndex}
                        dropIndex={providerDropIndex}
                        sharedDragX={providerDragX}
                        dragScrollDelta={providerDragScrollDelta}
                        onPress={() => onProviderFilterToggle(provider.id)}
                        onDragStart={handleProviderDragStart}
                        onDrop={handleProviderDrop}
                        onDragFinalize={handleProviderDragFinalize}
                        onAutoScrollPointer={handleProviderAutoScrollPointer}
                        progress={branchIndex < MAX_GOOEY_PILLS
                          ? providerProgresses[branchIndex]
                          : undefined}
                      >
                        {provider.icon}
                      </ProviderDockPill>
                        );
                      })
                      : null}
                  </ScrollView>
                </Animated.View>
              )}
            </View>
            <View style={[styles.sideRow, styles.pointerBoxNone]}>
              <View
                pointerEvents="none"
                style={[
                  styles.branchGooeyViewport,
                  {
                    height: gooeyCanvasHeight(ATTACH_ACTION_COUNT, 'horizontal'),
                  },
                ]}
              >
                <View
                  style={[
                    styles.branchGooeyLayer,
                    {
                      width: gooeyCanvasWidth(ATTACH_ACTION_COUNT, 'horizontal'),
                      height: gooeyCanvasHeight(ATTACH_ACTION_COUNT, 'horizontal'),
                    },
                  ]}
                >
                  <FabGooeyLayer
                    progresses={attachProgresses}
                    pillCount={ATTACH_ACTION_COUNT}
                    fill={gooeyFill(theme.scheme)}
                    orientation="horizontal"
                  />
                </View>
              </View>
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
                      progress={attachProgresses[2]}
                    >
                      <FileText size={23} color={fabAccent} strokeWidth={1.7} />
                    </InlineActionPill>
                    <InlineActionPill
                      index={1}
                      itemCount={ATTACH_ACTION_COUNT}
                      isOpen={attachActionsOpen}
                      onPress={onPickCamera}
                      accessibilityLabel="Take photo"
                      progress={attachProgresses[1]}
                    >
                      <Camera size={23} color={fabAccent} strokeWidth={1.7} />
                    </InlineActionPill>
                    <InlineActionPill
                      index={0}
                      itemCount={ATTACH_ACTION_COUNT}
                      isOpen={attachActionsOpen}
                      onPress={onPickPhoto}
                      accessibilityLabel="Choose photo"
                      progress={attachProgresses[0]}
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
    position: 'relative',
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    overflow: 'visible',
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
  branchGooeyViewport: {
    position: 'absolute',
    left: 0,
    right: -(GOOEY_PAD + FAB_PILL),
    top: -GOOEY_PAD,
    overflow: 'hidden',
    zIndex: 0,
  },
  branchGooeyLayer: {
    position: 'absolute',
    right: 0,
    top: 0,
  },
  fabSurface: {
    borderWidth: StyleSheet.hairlineWidth,
  },
  graphiteCover: {
    position: 'absolute',
    top: 0,
    right: 0,
    bottom: 0,
    left: 0,
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
    paddingRight: MODEL_BUTTON_GAP,
    overflow: 'visible',
    zIndex: 1,
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
    zIndex: 1,
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
    zIndex: 1,
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
