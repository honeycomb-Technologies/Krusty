import { Pressable, StyleSheet, Text, View } from 'react-native';
import { PanelLeftClose } from 'lucide-react-native';
import type { SessionResponse } from '@mitsuro/api';
import { useThemeContext } from '@mobile/hooks/useTheme';
import { DesktopSessionRail } from './DesktopSessionRail';
import type { DesktopPlane } from './types';
import { DESKTOP } from './desktopTheme';

export function ContextRail({
  plane,
  sessions,
  activeSessionId,
  onSelectSession,
  onDeleteSession,
  onNewSession,
  onOpenProject,
  onCollapse,
}: {
  plane: DesktopPlane;
  sessions: SessionResponse[];
  activeSessionId: string | null;
  onSelectSession: (session: SessionResponse) => void;
  onDeleteSession: (id: string) => void;
  onNewSession: () => void;
  onOpenProject: () => void;
  onCollapse: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const title = plane === 'chat' ? 'Chats' : plane === 'code' ? 'Projects' : 'Hive';

  return (
    <View style={[styles.rail, { backgroundColor: t.background, borderRightColor: t.border }]}>
      <View style={[styles.header, { borderBottomColor: t.border }]}>
        <Text style={[styles.title, { color: t.foreground }]}>{title}</Text>
        <Pressable onPress={onCollapse} accessibilityLabel="Collapse sidebar" style={styles.iconBtn}>
          <PanelLeftClose size={16} color={t.mutedForeground} />
        </Pressable>
      </View>
      <DesktopSessionRail
        plane={plane}
        sessions={sessions}
        activeSessionId={activeSessionId}
        onSelectSession={onSelectSession}
        onDeleteSession={onDeleteSession}
        onNewSession={onNewSession}
        onOpenProject={onOpenProject}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  rail: {
    width: DESKTOP.contextRailWidth,
    borderRightWidth: StyleSheet.hairlineWidth,
  },
  header: {
    height: DESKTOP.chromeHeight,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 14,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'space-between',
  },
  title: {
    fontSize: 13,
    fontWeight: '700',
    letterSpacing: -0.2,
  },
  iconBtn: {
    width: 30,
    height: 30,
    alignItems: 'center',
    justifyContent: 'center',
    borderRadius: 8,
  },
});
