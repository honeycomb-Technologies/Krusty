import { useState, useEffect } from 'react';
import { View, Pressable, Text, StyleSheet } from 'react-native';
import { BlurView } from '../../platform/blur';
import * as Haptics from '../../platform/haptics';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withSpring,
  withDelay,
  withTiming,
  interpolate,
  runOnJS,
} from 'react-native-reanimated';
import { Gesture, GestureDetector } from 'react-native-gesture-handler';
import { Brain, Hammer, Compass, Paperclip, Bot, FlaskConical } from 'lucide-react-native';
import { useThemeContext } from '../../hooks/useTheme';
import { cycleThinkingLevel, type ThinkingLevel } from '@krusty/api';

interface AccordionControlsProps {
  thinkingLevel: ThinkingLevel;
  onThinkingChange: (level: ThinkingLevel) => void;
  mode: 'build' | 'plan';
  onModeToggle: () => void;
  onAttach: () => void;
  onModelSelect: () => void;
  model: string | null;
  isOpen: boolean;
  onToggle: () => void;
  sessionType?: 'chat' | 'code' | 'mako';
  researchEnabled?: boolean;
  onResearchToggle?: () => void;
}

const THINKING_COLORS: Record<ThinkingLevel, string> = {
  off: 'rgba(255,255,255,0.25)',
  low: 'rgba(230,167,0,0.4)',
  medium: 'rgba(230,167,0,0.65)',
  high: 'rgba(230,167,0,0.85)',
  xhigh: 'rgba(255,180,0,1.0)',
};

const SPRING_CONFIG = { damping: 18, stiffness: 350, mass: 0.6 };

function AccordionPill({
  children,
  index,
  isOpen,
  onPress,
  flashLabel,
}: {
  children: React.ReactNode;
  index: number;
  isOpen: boolean;
  onPress: () => void;
  flashLabel?: string | null;
}) {
  const { theme } = useThemeContext();
  const progress = useSharedValue(0);
  const flashOpacity = useSharedValue(0);

  useEffect(() => {
    progress.value = withDelay(
      index * 40,
      withSpring(isOpen ? 1 : 0, SPRING_CONFIG),
    );
  }, [isOpen]);

  useEffect(() => {
    if (flashLabel) {
      flashOpacity.value = withTiming(1, { duration: 150 });
      const timer = setTimeout(() => {
        flashOpacity.value = withTiming(0, { duration: 400 });
      }, 800);
      return () => clearTimeout(timer);
    }
  }, [flashLabel]);

  const animatedStyle = useAnimatedStyle(() => ({
    opacity: progress.value,
    transform: [
      { translateY: interpolate(progress.value, [0, 1], [20, 0]) },
      { scale: interpolate(progress.value, [0, 1], [0.8, 1]) },
    ],
  }));

  const flashStyle = useAnimatedStyle(() => ({
    opacity: flashOpacity.value,
  }));

  const g = theme.colors.glass;

  return (
    <Animated.View style={[styles.pillOuter, animatedStyle]}>
      <Pressable onPress={onPress}>
        <BlurView
          intensity={theme.colors.glassBlur}
          tint={theme.scheme === 'dark' ? 'systemMaterialDark' : 'systemMaterialLight'}
          style={styles.pillBlur}
        >
          <View style={[styles.pill, { borderColor: g.border }]}>
            {children}
          </View>
        </BlurView>
      </Pressable>
      {flashLabel && (
        <Animated.View style={[styles.flashLabel, flashStyle]}>
          <BlurView
            intensity={20}
            tint="dark"
            style={styles.flashBlur}
          >
            <Text style={styles.flashText}>{flashLabel}</Text>
          </BlurView>
        </Animated.View>
      )}
    </Animated.View>
  );
}

export function AccordionControls({
  thinkingLevel,
  onThinkingChange,
  mode,
  onModeToggle,
  onAttach,
  onModelSelect,
  model,
  isOpen,
  onToggle,
  sessionType = 'code',
  researchEnabled = false,
  onResearchToggle,
}: AccordionControlsProps) {
  const { theme } = useThemeContext();
  const [flashThinking, setFlashThinking] = useState<string | null>(null);
  const [flashMode, setFlashMode] = useState<string | null>(null);
  const isChat = sessionType === 'chat';

  const handleThinking = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    const next = cycleThinkingLevel(thinkingLevel, model);
    onThinkingChange(next);
    const labels: Record<ThinkingLevel, string> = {
      off: 'Off', low: 'Low', medium: 'Med', high: 'High', xhigh: 'Max',
    };
    setFlashThinking(labels[next]);
    setTimeout(() => setFlashThinking(null), 1200);
  };

  const handleMode = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onModeToggle();
    setFlashMode(mode === 'build' ? 'Plan' : 'Build');
    setTimeout(() => setFlashMode(null), 1200);
  };

  const handleResearch = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onResearchToggle?.();
    setFlashMode(researchEnabled ? 'Research Off' : 'Research On');
    setTimeout(() => setFlashMode(null), 1200);
  };

  const handleAttach = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onAttach();
  };

  const handleModel = () => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onModelSelect();
  };

  const t = theme.colors;
  const thinkingColor = THINKING_COLORS[thinkingLevel];

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
          <AccordionPill index={3} isOpen={isOpen} onPress={handleModel} flashLabel={null}>
            <Bot size={24} color={t.mutedForeground} strokeWidth={1.6} />
          </AccordionPill>

          <AccordionPill index={2} isOpen={isOpen} onPress={handleAttach} flashLabel={null}>
            <Paperclip size={24} color={t.mutedForeground} strokeWidth={1.6} />
          </AccordionPill>

          {isChat ? (
            <AccordionPill index={1} isOpen={isOpen} onPress={handleResearch} flashLabel={flashMode}>
              <FlaskConical size={24} color={researchEnabled ? t.thinking : t.mutedForeground} strokeWidth={1.6} />
            </AccordionPill>
          ) : (
            <AccordionPill index={1} isOpen={isOpen} onPress={handleMode} flashLabel={flashMode}>
              {mode === 'build' ? (
                <Hammer size={24} color={t.mutedForeground} strokeWidth={1.6} />
              ) : (
                <Compass size={24} color={t.mutedForeground} strokeWidth={1.6} />
              )}
            </AccordionPill>
          )}

          <AccordionPill index={0} isOpen={isOpen} onPress={handleThinking} flashLabel={flashThinking}>
            <Brain size={24} color={thinkingColor} strokeWidth={1.6} />
          </AccordionPill>
        </Animated.View>
      </GestureDetector>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    alignItems: 'center',
  },
  pillColumn: {
    gap: 10,
    alignItems: 'center',
  },
  pillOuter: {
    flexDirection: 'row',
    alignItems: 'center',
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
  flashLabel: {
    position: 'absolute',
    right: 64,
    top: 16,
  },
  flashBlur: {
    borderRadius: 8,
    overflow: 'hidden',
    paddingHorizontal: 10,
    paddingVertical: 4,
  },
  flashText: {
    color: '#fff',
    fontSize: 13,
    fontWeight: '600',
  },
});
