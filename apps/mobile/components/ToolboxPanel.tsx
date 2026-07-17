import { useCallback, useEffect, useState } from 'react';
import {
  View,
  Pressable,
  StyleSheet,
  Platform,
  useWindowDimensions,
} from 'react-native';
import { Gesture, GestureDetector } from 'react-native-gesture-handler';
import { BlurView } from '../platform/blur';
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withSpring,
  withTiming,
  interpolate,
  Easing,
  runOnJS,
} from 'react-native-reanimated';
import { FileText, Search, TerminalSquare, X } from 'lucide-react-native';
import * as Haptics from '../platform/haptics';
import { useThemeContext } from '../hooks/useTheme';
import { useBreakpoint } from '../hooks/useBreakpoint';
import { ToolboxTerminal } from './toolbox/ToolboxTerminal';
import { ToolboxBrowser } from './toolbox/ToolboxBrowser';
import { ReportsContent } from './ReportsViewer';

const SPRING = { damping: 22, stiffness: 280, mass: 0.8 };
const TOOL_TABS = [
  { label: 'Terminal', icon: TerminalSquare },
  { label: 'Browser', icon: Search },
  { label: 'Papers', icon: FileText },
];

interface ToolboxPanelProps {
  visible: boolean;
  onClose: () => void;
  activeTab: number;
  onTabChange: (tab: number) => void;
  /**
   * `dock` — desktop side panel (sibling of chat, no overlay/handle).
   * `overlay` — mobile top sheet with backdrop + drag handle.
   * Default: desktop → dock, mobile → overlay.
   */
  variant?: 'dock' | 'overlay';
}

export function ToolboxPanel({
  visible,
  onClose,
  activeTab,
  onTabChange,
  variant,
}: ToolboxPanelProps) {
  const { theme } = useThemeContext();
  const { isDesktop } = useBreakpoint();
  const t = theme.colors;
  const isDark = theme.scheme === 'dark';
  const { height: windowHeight } = useWindowDimensions();
  const mode = variant ?? (isDesktop ? 'dock' : 'overlay');
  const isDock = mode === 'dock';

  const panelHeight = Math.max(windowHeight, 1);
  const progress = useSharedValue(0);
  const dragOffset = useSharedValue(0);
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    if (isDock) {
      // Dock is layout-driven by the parent; mount when visible.
      setMounted(visible);
      progress.value = visible ? 1 : 0;
      dragOffset.value = 0;
      return;
    }

    if (visible) {
      setMounted(true);
      dragOffset.value = 0;
      progress.value = withSpring(1, SPRING);
    } else {
      progress.value = withTiming(0, {
        duration: 220,
        easing: Easing.out(Easing.cubic),
      });
      const timer = setTimeout(() => {
        dragOffset.value = 0;
        setMounted(false);
      }, 240);
      return () => clearTimeout(timer);
    }
  }, [visible, progress, dragOffset, isDock]);

  const overlayPanelStyle = useAnimatedStyle(() => {
    const upwardDrag = Math.max(-panelHeight, Math.min(dragOffset.value, 0));
    const translateY =
      interpolate(progress.value, [0, 1], [-panelHeight, 0]) + upwardDrag;
    return {
      height: panelHeight,
      borderBottomLeftRadius: 20,
      borderBottomRightRadius: 20,
      transform: [{ translateY }],
      opacity: progress.value,
    };
  });

  const backdropStyle = useAnimatedStyle(() => {
    const backdropOpacity = interpolate(progress.value, [0, 1], [0, 1]);
    return {
      opacity: backdropOpacity,
      pointerEvents: backdropOpacity > 0.05 ? ('auto' as const) : ('none' as const),
    };
  });

  const handleClose = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    onClose();
  }, [onClose]);

  const handleTabChange = useCallback(
    (index: number) => {
      Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
      onTabChange(index);
    },
    [onTabChange],
  );

  const closeHandleGesture = Gesture.Pan()
    .activeOffsetY([-8, 8])
    .failOffsetX([-24, 24])
    .onUpdate((event) => {
      dragOffset.value = Math.min(event.translationY, 0);
    })
    .onEnd((event) => {
      if (event.translationY < -44 || event.velocityY < -500) {
        runOnJS(handleClose)();
        return;
      }
      dragOffset.value = withSpring(0, SPRING);
    });

  if (isDock && !visible) return null;
  if (!isDock && !mounted) return null;

  const header = (
    <View style={[styles.header, { borderBottomColor: t.border }]}>
      <View
        style={[
          styles.tabRail,
          {
            backgroundColor: t.glass.background,
            borderColor: t.glass.border,
          },
        ]}
      >
        {TOOL_TABS.map((tab, index) => {
          const Icon = tab.icon;
          const active = index === activeTab;
          return (
            <Pressable
              key={tab.label}
              accessibilityRole="tab"
              accessibilityLabel={tab.label}
              accessibilityState={{ selected: active }}
              onPress={() => handleTabChange(index)}
              style={[
                styles.tabButton,
                active && { backgroundColor: t.glass.backgroundElevated },
              ]}
            >
              <Icon
                size={18}
                color={active ? t.foreground : t.mutedForeground}
                strokeWidth={active ? 2.1 : 1.8}
              />
            </Pressable>
          );
        })}
      </View>

      {isDock ? (
        <Pressable
          onPress={handleClose}
          accessibilityRole="button"
          accessibilityLabel="Close toolbox"
          style={styles.closeBtn}
        >
          <X size={18} color={t.mutedForeground} strokeWidth={1.8} />
        </Pressable>
      ) : null}
    </View>
  );

  const body = (
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
  );

  const surface = (
    <>
      <BlurView
        intensity={50}
        tint={isDark ? 'systemChromeMaterialDark' : 'systemChromeMaterialLight'}
        style={StyleSheet.absoluteFill}
      />
      <View
        style={[
          StyleSheet.absoluteFill,
          {
            backgroundColor: isDark
              ? 'rgba(11,17,25,0.94)'
              : 'rgba(255,255,255,0.94)',
          },
        ]}
      />
      {header}
      {body}
    </>
  );

  // Desktop dock: fixed-width side rail — no overlay, no half-screen flex share.
  if (isDock) {
    return (
      <View
        style={[
          styles.dockPanel,
          { borderLeftColor: t.border, backgroundColor: t.background },
        ]}
      >
        {surface}
      </View>
    );
  }

  // Mobile overlay sheet.
  return (
    <View style={StyleSheet.absoluteFill} pointerEvents="box-none">
      <Animated.View style={[styles.backdrop, backdropStyle]}>
        <Pressable style={StyleSheet.absoluteFill} onPress={handleClose} />
      </Animated.View>

      <Animated.View style={[styles.panelMobile, overlayPanelStyle]}>
        {surface}
        <GestureDetector gesture={closeHandleGesture}>
          <Animated.View style={styles.handleZoneMobile}>
            <View
              style={[styles.handleMobile, { backgroundColor: t.mutedForeground }]}
            />
          </Animated.View>
        </GestureDetector>
      </Animated.View>
    </View>
  );
}

const styles = StyleSheet.create({
  backdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: 'rgba(0,0,0,0.5)',
    zIndex: 200,
  },
  panelMobile: {
    position: 'absolute',
    left: 0,
    right: 0,
    top: 0,
    zIndex: 201,
    overflow: 'hidden',
  },
  /**
   * Fixed side rail — never flex-share with chat (no % / flex:1 half-screen).
   * Width must match TOOLBOX_DOCK_WIDTH in chat-screen/styles.
   */
  dockPanel: {
    width: 360,
    flexGrow: 0,
    flexShrink: 0,
    flexBasis: 360,
    alignSelf: 'stretch',
    flexDirection: 'column',
    overflow: 'hidden',
    borderLeftWidth: StyleSheet.hairlineWidth,
  },
  header: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
    paddingHorizontal: 10,
    paddingVertical: 8,
    borderBottomWidth: StyleSheet.hairlineWidth,
    gap: 10,
  },
  tabRail: {
    flexDirection: 'row',
    alignItems: 'center',
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 2,
    gap: 2,
  },
  tabButton: {
    width: 44,
    minHeight: 34,
    borderRadius: 10,
    alignItems: 'center',
    justifyContent: 'center',
  },
  closeBtn: {
    position: 'absolute',
    right: 10,
    width: 32,
    height: 32,
    alignItems: 'center',
    justifyContent: 'center',
  },
  body: {
    flex: 1,
  },
  handleZoneMobile: {
    position: 'absolute',
    left: 0,
    right: 0,
    bottom: 0,
    height: 28,
    alignItems: 'center',
    justifyContent: 'center',
    zIndex: 6,
  },
  handleMobile: {
    width: 44,
    height: 5,
    borderRadius: 999,
    opacity: 0.48,
  },
  tabContent: {
    ...StyleSheet.absoluteFillObject,
  },
  hidden: Platform.OS === 'web'
    ? { display: 'none' as any }
    : { opacity: 0, pointerEvents: 'none' as const },
});
