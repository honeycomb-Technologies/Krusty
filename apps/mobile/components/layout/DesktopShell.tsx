import { useState, useCallback } from 'react';
import { View, Text, Pressable, StyleSheet } from 'react-native';
import { Settings, SquarePlus, FolderPlus, Wifi, WifiOff } from 'lucide-react-native';
import * as Haptics from '../../platform/haptics';
import { BlurView } from '../../platform/blur';
import { useBreakpoint } from '../../hooks/useBreakpoint';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { SessionList, type SessionListProps } from '../chat/SessionList';

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
  ...sessionListProps
}: DesktopShellProps) {
  const { isDesktop } = useBreakpoint();
  const { theme } = useThemeContext();
  const { status } = useConnection();
  const t = theme.colors;
  const isDark = theme.scheme === 'dark';

  const [pickerVisible, setPickerVisible] = useState(false);

  const handleNew = useCallback(() => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium);
    if (activeTab === 0) {
      onNewSession();
    } else {
      setPickerVisible(true);
    }
  }, [activeTab, onNewSession]);

  if (!isDesktop) {
    return <>{children}</>;
  }

  return (
    <View style={styles.shell}>
      {/* Sidebar */}
      <View style={[styles.sidebar, { borderRightColor: t.border }]}>
        <BlurView
          intensity={30}
          tint={isDark ? 'systemChromeMaterialDark' : 'systemChromeMaterialLight'}
          style={StyleSheet.absoluteFill}
        />
        <View style={[StyleSheet.absoluteFill, {
          backgroundColor: isDark ? 'rgba(11,17,25,0.92)' : 'rgba(255,255,255,0.92)',
        }]} />

        <View style={styles.sidebarContent}>
          <View style={styles.sidebarHeader}>
            <Text style={[styles.sidebarTitle, { color: t.foreground }]}>Krusty</Text>
          </View>

          <SessionList
            {...sessionListProps}
            activeTab={activeTab}
            onNewSession={onNewSession}
            onNewSessionWithDir={(path) => { setPickerVisible(false); onNewSessionWithDir(path); }}
            showPicker={pickerVisible}
            onPickerDone={() => setPickerVisible(false)}
          />

          {/* Bottom bar */}
          <View style={[styles.bottomBar, { borderTopColor: t.border }]}>
            <Pressable
              onPress={() => { Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light); onOpenSettings(); }}
              style={styles.iconBtn}
            >
              <Settings size={20} color={t.mutedForeground} strokeWidth={1.8} />
            </Pressable>

            <View style={styles.statusIcon}>
              {status === 'connected'
                ? <Wifi size={16} color="#22c55e" strokeWidth={2} />
                : <WifiOff size={16} color={status === 'connecting' ? '#f59e0b' : '#ef4444'} strokeWidth={2} />
              }
            </View>

            <View style={{ flex: 1 }} />

            <Pressable onPress={handleNew} style={styles.iconBtn}>
              {activeTab === 0
                ? <SquarePlus size={20} color={t.mutedForeground} strokeWidth={1.8} />
                : <FolderPlus size={20} color={t.mutedForeground} strokeWidth={1.8} />}
            </Pressable>
          </View>
        </View>
      </View>

      {/* Main content */}
      <View style={styles.main}>
        {children}
      </View>
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
  },
  iconBtn: {
    padding: 6,
  },
  statusIcon: {
    marginLeft: 6,
  },
  main: {
    flex: 1,
  },
});
