import { useCallback, useEffect, useState } from 'react';
import {
  View,
  Pressable,
  StyleSheet,
  Dimensions,
  Platform,
} from 'react-native';
import { BlurView } from '../platform/blur';
import Animated, {
  type SharedValue,
  useSharedValue,
  useAnimatedStyle,
  withSpring,
  withTiming,
  interpolate,
  Easing,
} from 'react-native-reanimated';
import { Pin, PinOff, X } from 'lucide-react-native';
import * as Haptics from '../platform/haptics';
import { useThemeContext } from '../hooks/useTheme';
import { SegmentControl } from './ui/SegmentControl';
import { ToolboxTerminal } from './toolbox/ToolboxTerminal';
import { ToolboxBrowser } from './toolbox/ToolboxBrowser';
import { ReportsContent } from './ReportsViewer';

const SCREEN_HEIGHT = Dimensions.get('window').height;
const PANEL_HEIGHT = SCREEN_HEIGHT * 0.88;
const SPLIT_HEIGHT = SCREEN_HEIGHT * 0.42;
const SPRING = { damping: 22, stiffness: 280, mass: 0.8 };
const SEGMENTS = ['Terminal', 'Browser', 'Papers'];

interface ToolboxPanelProps {
  visible: boolean;
  onClose: () => void;
  onTogglePin: () => void;
  isSplit: boolean;
  splitProgress: SharedValue<number>;
  activeTab: number;
  onTabChange: (tab: number) => void;
}

export function ToolboxPanel({
  visible,
  onClose,
  onTogglePin,
  isSplit,
  splitProgress,
  activeTab,
  onTabChange,
}: ToolboxPanelProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const isDark = theme.scheme === 'dark';

  const progress = useSharedValue(0);
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    if (visible) {
      setMounted(true);
      progress.value = withSpring(1, SPRING);
    } else {
      progress.value = withTiming(0, { duration: 200, easing: Easing.out(Easing.cubic) });
      const timer = setTimeout(() => setMounted(false), 220);
      return () => clearTimeout(timer);
    }
  }, [visible, progress]);

  const panelStyle = useAnimatedStyle(() => {
    const height = interpolate(splitProgress.value, [0, 1], [PANEL_HEIGHT, SPLIT_HEIGHT]);
    const radius = interpolate(splitProgress.value, [0, 1], [20, 0]);
    return {
      height,
      borderBottomLeftRadius: radius,
      borderBottomRightRadius: radius,
      transform: [
        { translateY: interpolate(progress.value, [0, 1], [-PANEL_HEIGHT, 0]) },
      ],
      opacity: progress.value,
    };
  });

  const backdropStyle = useAnimatedStyle(() => {
    const backdropOpacity = interpolate(progress.value, [0, 1], [0, 1])
      * interpolate(splitProgress.value, [0, 0.3], [1, 0]);
    return {
      opacity: backdropOpacity,
      pointerEvents: backdropOpacity > 0.05 ? ('auto' as const) : ('none' as const),
    };
  });

  const showPin = activeTab < 2;

  const handlePin = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    onTogglePin();
  }, [onTogglePin]);

  const handleClose = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onClose();
  }, [onClose]);

  const handleTabChange = useCallback((index: number) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onTabChange(index);
  }, [onTabChange]);

  if (!mounted) return null;

  return (
    <>
      <Animated.View style={[styles.backdrop, backdropStyle]}>
        <Pressable style={StyleSheet.absoluteFill} onPress={handleClose} />
      </Animated.View>

      <Animated.View style={[styles.panel, panelStyle]}>
        <BlurView
          intensity={50}
          tint={isDark ? 'systemChromeMaterialDark' : 'systemChromeMaterialLight'}
          style={StyleSheet.absoluteFill}
        />
        <View
          style={[
            StyleSheet.absoluteFill,
            { backgroundColor: isDark ? 'rgba(11,17,25,0.92)' : 'rgba(255,255,255,0.92)' },
          ]}
        />

        <View style={[styles.header, { borderBottomColor: t.border }]}>
          {showPin ? (
            <Pressable onPress={handlePin} style={styles.headerBtn}>
              {isSplit ? (
                <PinOff size={18} color={t.userMessage} strokeWidth={1.8} />
              ) : (
                <Pin size={18} color={t.mutedForeground} strokeWidth={1.8} />
              )}
            </Pressable>
          ) : (
            <View style={styles.headerBtn} />
          )}

          <View style={styles.segmentWrapper}>
            <SegmentControl
              segments={SEGMENTS}
              selected={activeTab}
              onSelect={handleTabChange}
            />
          </View>

          <Pressable onPress={handleClose} style={styles.headerBtn}>
            <X size={20} color={t.mutedForeground} strokeWidth={1.8} />
          </Pressable>
        </View>

        <View style={styles.body}>
          <View style={[styles.tabContent, activeTab !== 0 && styles.hidden]}>
            <ToolboxTerminal visible={activeTab === 0 && visible} />
          </View>
          <View style={[styles.tabContent, activeTab !== 1 && styles.hidden]}>
            <ToolboxBrowser visible={activeTab === 1 && visible} />
          </View>
          <View style={[styles.tabContent, activeTab !== 2 && styles.hidden]}>
            <ReportsContent visible={activeTab === 2 && visible} />
          </View>
        </View>
      </Animated.View>
    </>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: 'rgba(0,0,0,0.5)',
    zIndex: 200,
  },
  panel: {
    position: 'absolute',
    left: 0,
    right: 0,
    top: 0,
    zIndex: 201,
    overflow: 'hidden',
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 12,
    paddingVertical: 10,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: 8,
  },
  headerBtn: {
    width: 36,
    alignItems: 'center',
    justifyContent: 'center',
    padding: 4,
  },
  segmentWrapper: {
    flex: 1,
  },
  body: {
    flex: 1,
  },
  tabContent: {
    ...StyleSheet.absoluteFillObject,
  },
  hidden: Platform.OS === 'web'
    ? { display: 'none' as any }
    : { opacity: 0, pointerEvents: 'none' as const },
});
