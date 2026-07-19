import { useState, useCallback } from 'react';
import { View, Text, Pressable, StyleSheet } from 'react-native';
import {
  Settings, SquarePlus, FolderPlus, Wifi, WifiOff,
  PanelLeftClose, PanelLeftOpen,
} from 'lucide-react-native';
import * as Haptics from '../../platform/haptics';
import { BlurView } from '../../platform/blur';
import { useBreakpoint } from '../../hooks/useBreakpoint';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { SessionList, type SessionListProps } from '../chat/SessionList';
import { PlanTracker } from '../chat/PlanTracker';
import { SettingsModal } from '../SettingsModal';

const SIDEBAR_WIDTH = 280;

interface DesktopShellProps extends SessionListProps {
  onOpenSettings: () => void;
  children: React.ReactNode;
}

export function DesktopShell({
  children,
  onOpenSettings,
  onNewSession,
  onNewSessionWithDir,
  activeTab,
  onSelectMakoView,
  ...sessionListProps
}: DesktopShellProps) {
  const { isDesktop } = useBreakpoint();
  const { theme } = useThemeContext();
  const { status } = useConnection();
  const t = theme.colors;
  const isDark = theme.scheme === 'dark';

  // Default closed — open chat canvas first; user expands the menu when needed.
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [pickerVisible, setPickerVisible] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false);

  // Match the mobile drawer's blur/overlay values
  const blurIntensity = 40;
  const bgOverlay = isDark ? 'rgba(11,17,25,0.88)' : 'rgba(255,255,255,0.88)';
  const blurTint = isDark ? 'systemChromeMaterialDark' : 'systemChromeMaterialLight';

  const handleNewChat = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    onNewSession();
  }, [onNewSession]);

  const toggleSidebar = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setSidebarOpen(prev => !prev);
  }, []);

  if (!isDesktop) {
    return <>{children}</>;
  }

  return (
    <View style={styles.shell}>
      {/* Sidebar — collapsible */}
      {sidebarOpen && (
        <View style={[styles.sidebar, { borderRightColor: t.border }]}>
          <BlurView intensity={blurIntensity} tint={blurTint} style={StyleSheet.absoluteFill} />
          <View style={[StyleSheet.absoluteFill, { backgroundColor: bgOverlay }]} />

          <View style={styles.sidebarContent}>
            {/* Header with collapse button */}
            <View style={styles.sidebarHeader}>
              <Text style={[styles.sidebarTitle, { color: t.foreground }]}>Krusty</Text>
              <Pressable onPress={toggleSidebar} style={styles.iconBtn}>
                <PanelLeftClose size={18} color={t.mutedForeground} strokeWidth={1.8} />
              </Pressable>
            </View>

            <SessionList
              {...sessionListProps}
              activeTab={activeTab}
              onNewSession={onNewSession}
              onNewSessionWithDir={(path) => { setPickerVisible(false); onNewSessionWithDir(path); }}
              showPicker={pickerVisible}
              onPickerDone={() => setPickerVisible(false)}
            />

            <PlanTracker />

            {/* Bottom bar — matches mobile drawer */}
            <View style={[styles.bottomBar, { borderTopColor: t.border }]}>
              <Pressable
                onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); setSettingsOpen(true); }}
                style={styles.iconBtn}
              >
                <Settings size={22} color={t.mutedForeground} strokeWidth={1.8} />
              </Pressable>

              <View style={styles.statusIcon}>
                {status === 'connected'
                  ? <Wifi size={16} color={t.success} strokeWidth={2} />
                  : <WifiOff size={16} color={status === 'connecting' ? t.warning : t.error} strokeWidth={2} />
                }
              </View>

              <View style={{ flex: 1 }} />

              {/* Terminal / browser live in Toolbox — do not duplicate them here. */}

              <Pressable
                onPress={handleNewChat}
                style={styles.iconBtn}
                accessibilityRole="button"
                accessibilityLabel="New conversation"
              >
                <SquarePlus size={22} color={t.mutedForeground} strokeWidth={1.8} />
              </Pressable>
              <Pressable
                onPress={() => setPickerVisible(true)}
                style={styles.iconBtn}
                accessibilityRole="button"
                accessibilityLabel="New code conversation"
              >
                <FolderPlus size={22} color={t.mutedForeground} strokeWidth={1.8} />
              </Pressable>
            </View>
          </View>
        </View>
      )}

      {/* Main area */}
      <View style={styles.main}>
        {/* Sidebar open button when collapsed */}
        {!sidebarOpen && (
          <Pressable
            onPress={toggleSidebar}
            style={[styles.openSidebarBtn, { borderColor: t.border }]}
          >
            <PanelLeftOpen size={18} color={t.mutedForeground} strokeWidth={1.8} />
          </Pressable>
        )}

        {/* Chat content — terminal/browser are Toolbox-only on desktop */}
        <View style={styles.flex}>
          {children}
        </View>
      </View>

      <SettingsModal visible={settingsOpen} onClose={() => setSettingsOpen(false)} />
    </View>
  );
}

const styles = StyleSheet.create({
  shell: {
    flex: 1,
    flexDirection: 'row',
  },
  sidebar: {
    width: SIDEBAR_WIDTH,
    borderRightWidth: StyleSheet.hairlineWidth,
    overflow: 'hidden',
  },
  sidebarContent: {
    flex: 1,
    paddingTop: 16,
  },
  sidebarHeader: {
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
    paddingHorizontal: 20,
    paddingBottom: 16,
  },
  sidebarTitle: {
    fontSize: 20,
    fontWeight: '700',
    letterSpacing: -0.3,
  },
  bottomBar: {
    flexDirection: 'row',
    alignItems: 'center',
    paddingHorizontal: 16,
    paddingVertical: 12,
    borderTopWidth: StyleSheet.hairlineWidth,
    gap: 4,
  },
  iconBtn: {
    padding: 6,
  },
  statusIcon: {
    marginLeft: 4,
  },
  main: {
    flex: 1,
  },
  flex: {
    flex: 1,
  },
  openSidebarBtn: {
    position: 'absolute',
    top: 12,
    left: 12,
    zIndex: 10,
    padding: 8,
    borderRadius: 8,
    borderWidth: StyleSheet.hairlineWidth,
  },
});
