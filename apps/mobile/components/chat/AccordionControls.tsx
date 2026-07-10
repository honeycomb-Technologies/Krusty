import { useEffect, useRef } from 'react';
import { View, Pressable, ScrollView, StyleSheet } from 'react-native';
import { BlurView } from '../../platform/blur';
import { LinearGradient } from '../../platform/linear-gradient';
import * as Haptics from '../../platform/haptics';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withSpring,
  withDelay,
  interpolate,
  runOnJS,
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

const THINKING_COLORS: Record<ThinkingLevel, string> = {
  off: 'rgba(255,255,255,0.25)',
  low: 'rgba(230,167,0,0.4)',
  medium: 'rgba(230,167,0,0.65)',
  high: 'rgba(230,167,0,0.85)',
  xhigh: 'rgba(255,180,0,1.0)',
};

const SPRING_CONFIG = { damping: 18, stiffness: 350, mass: 0.6 };
const MAX_PILL_INDEX = 5;
const OPEN_STAGGER_MS = 40;
const CLOSE_STAGGER_MS = 28;
const ATTACH_ACTION_COUNT = 3;
const DOCK_FADE_WIDTH = 34;
const MODEL_BUTTON_GAP = 10;

function AccordionPill({
  children,
  index,
  isOpen,
  onPress,
  active = false,
  sideContent,
  disabled = false,
}: {
  children: React.ReactNode;
  index: number;
  isOpen: boolean;
  onPress: () => void;
  active?: boolean;
  sideContent?: React.ReactNode;
  disabled?: boolean;
}) {
  const { theme } = useThemeContext();
  const progress = useSharedValue(0);

  useEffect(() => {
    const delayMs = isOpen
      ? index * OPEN_STAGGER_MS
      : Math.max(0, MAX_PILL_INDEX - index) * CLOSE_STAGGER_MS;
    progress.value = withDelay(
      delayMs,
      withSpring(isOpen ? 1 : 0, SPRING_CONFIG),
    );
  }, [isOpen]);

  const animatedStyle = useAnimatedStyle(() => ({
    opacity: progress.value,
    transform: [
      { translateY: interpolate(progress.value, [0, 1], [20, 0]) },
      { scale: interpolate(progress.value, [0, 1], [0.8, 1]) },
    ],
  }));

  const g = theme.colors.glass;

  return (
    <Animated.View
      pointerEvents="box-none"
      style={[styles.pillOuter, animatedStyle]}
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

  useEffect(() => {
    const delayMs = isOpen
      ? index * OPEN_STAGGER_MS
      : Math.max(0, itemCount - index - 1) * closeStaggerMs;
    progress.value = withDelay(
      delayMs,
      withSpring(isOpen ? 1 : 0, SPRING_CONFIG),
    );
  }, [isOpen, itemCount, closeStaggerMs]);

  const animatedStyle = useAnimatedStyle(() => ({
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
  model,
  isOpen,
  onToggle,
  sessionType = 'code',
  researchEnabled = false,
  onResearchToggle,
}: AccordionControlsProps) {
  const { theme } = useThemeContext();
  const providerScrollRef = useRef<ScrollView>(null);
  const isChat = sessionType === 'chat';
  const isMako = sessionType === 'mako';

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
  const thinkingColor = THINKING_COLORS[thinkingLevel];
  const dockFadeColor = theme.scheme === 'dark'
    ? 'rgba(11,17,25,0.92)'
    : 'rgba(255,255,255,0.92)';

  useEffect(() => {
    if (!modelPickerOpen || !isOpen) return;
    requestAnimationFrame(() => {
      providerScrollRef.current?.scrollToEnd({ animated: false });
    });
  }, [isOpen, modelPickerOpen, providerFilters.length]);

  const swipeDown = Gesture.Pan()
    .activeOffsetY(15)
    .failOffsetX([-20, 20])
    .onEnd((event) => {
      if (event.translationY > 40 && isOpen) {
        runOnJS(onToggle)();
      }
    });

  return (
    <View style={styles.container} pointerEvents="box-none">
      {/* Floating accordion pills */}
      <GestureDetector gesture={swipeDown}>
        <Animated.View style={styles.pillColumn}>
          <AccordionPill
            index={5}
            isOpen={isOpen}
            onPress={handleModel}
            active={modelPickerOpen}
            sideContent={
              <View
                pointerEvents={modelPickerOpen ? "box-none" : "none"}
                style={styles.modelFilterDock}
              >
                <ScrollView
                  ref={providerScrollRef}
                  horizontal
                  bounces
                  alwaysBounceHorizontal={providerFilters.length > 3}
                  directionalLockEnabled
                  scrollEnabled={modelPickerOpen && isOpen}
                  showsHorizontalScrollIndicator={false}
                  style={styles.modelFilterScroll}
                  contentContainerStyle={styles.modelFilterContent}
                >
                  {[...providerFilters].reverse().map((provider, visualIndex) => (
                    <InlineActionPill
                      key={provider.id}
                      index={providerFilters.length - visualIndex - 1}
                      itemCount={providerFilters.length}
                      isOpen={modelPickerOpen && isOpen}
                      active={selectedProviderFilter === provider.id}
                      size={56}
                      closeStaggerMs={0}
                      accessibilityLabel={`Filter models by ${provider.label}`}
                      onPress={() => onProviderFilterToggle(provider.id)}
                    >
                      {provider.icon}
                    </InlineActionPill>
                  ))}
                </ScrollView>
                <View pointerEvents="none" style={styles.modelFilterFadeLeft}>
                  <LinearGradient
                    colors={[dockFadeColor, 'transparent']}
                    start={{ x: 0, y: 0.5 }}
                    end={{ x: 1, y: 0.5 }}
                    style={StyleSheet.absoluteFill}
                  />
                </View>
                <View pointerEvents="none" style={styles.modelFilterFadeRight}>
                  <LinearGradient
                    colors={['transparent', dockFadeColor]}
                    start={{ x: 0, y: 0.5 }}
                    end={{ x: 1, y: 0.5 }}
                    style={StyleSheet.absoluteFill}
                  />
                </View>
              </View>
            }
          >
            <Bot
              size={24}
              color={modelPickerOpen ? t.thinking : t.mutedForeground}
              strokeWidth={1.6}
            />
          </AccordionPill>

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
              color={attachPickerOpen ? t.thinking : t.mutedForeground}
              strokeWidth={1.6}
            />
          </AccordionPill>

          {isChat ? (
            <AccordionPill index={3} isOpen={isOpen} onPress={handleResearch}>
              <FlaskConical size={24} color={researchEnabled ? t.thinking : t.mutedForeground} strokeWidth={1.6} />
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
              <ShieldOff size={24} color={t.warning} strokeWidth={1.6} />
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
                  ? t.thinking
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
  modelFilterDock: {
    flex: 1,
    minWidth: 0,
    height: 56,
    marginRight: 0,
    overflow: 'hidden',
    position: 'relative',
    justifyContent: 'center',
  },
  modelFilterScroll: {
    flex: 1,
    overflow: 'hidden',
  },
  modelFilterContent: {
    flexGrow: 1,
    minHeight: 56,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'flex-end',
    gap: 8,
    paddingLeft: DOCK_FADE_WIDTH,
    paddingRight: MODEL_BUTTON_GAP,
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
  inlinePill: {
    justifyContent: 'center',
    alignItems: 'center',
    borderWidth: StyleSheet.hairlineWidth,
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
