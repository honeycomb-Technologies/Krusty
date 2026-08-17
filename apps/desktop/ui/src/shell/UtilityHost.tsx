import { Pressable, StyleSheet, Text, View } from 'react-native';
import {
  Cable,
  CalendarClock,
  FileCode2,
  Globe2,
  MemoryStick,
  TerminalSquare,
  Workflow,
  X,
  type LucideIcon,
} from 'lucide-react-native';
import { DesktopTerminalPane } from '../browser/DesktopTerminalPane';
import { ToolboxChanges } from '@mobile/components/toolbox/ToolboxChanges';
import { ToolboxConnections } from '@mobile/components/toolbox/ToolboxConnections';
import { ReportsContent } from '@mobile/components/ReportsViewer';
import { HiveScheduleView } from '@mobile/components/hive/HiveScheduleView';
import { HiveRunsView } from '@mobile/components/hive/HiveRunsView';
import { HiveMemoryView } from '@mobile/components/hive/HiveMemoryView';
import { useHiveCurrent } from '@mobile/components/hive/hooks/useHiveCurrent';
import { useHiveMemories } from '@mobile/components/hive/hooks/useHiveMemories';
import { useThemeContext } from '@mobile/hooks/useTheme';
import { DesktopBrowserPane } from '../browser/DesktopBrowserPane';
import type { DesktopPlane, DesktopUtilityPane } from './types';
import { DESKTOP } from './desktopTheme';

const TABS: Record<DesktopPlane, Array<{ id: DesktopUtilityPane; label: string; icon: LucideIcon }>> = {
  chat: [
    { id: 'library', label: 'Library', icon: FileCode2 },
    { id: 'connections', label: 'Connections', icon: Cable },
  ],
  code: [
    { id: 'terminal', label: 'Ghostty', icon: TerminalSquare },
    { id: 'changes', label: 'Changes', icon: FileCode2 },
    { id: 'browser', label: 'Browser', icon: Globe2 },
  ],
  hive: [
    { id: 'schedule', label: 'Calendar', icon: CalendarClock },
    { id: 'runs', label: 'Runs', icon: Workflow },
    { id: 'memory', label: 'Memory', icon: MemoryStick },
  ],
};

function Pane({
  active,
  children,
}: {
  active: boolean;
  children: React.ReactNode;
}) {
  return (
    <View
      style={[styles.pane, !active && styles.paneHidden]}
      pointerEvents={active ? 'auto' : 'none'}
    >
      {children}
    </View>
  );
}

export function UtilityHost({
  plane,
  pane,
  onPaneChange,
  onClose,
  projectDirectory,
  onOpenHiveRun,
  onOpenProject,
}: {
  plane: DesktopPlane;
  pane: DesktopUtilityPane;
  onPaneChange: (pane: DesktopUtilityPane) => void;
  onClose: () => void;
  projectDirectory?: string | null;
  onOpenHiveRun?: (id: string) => void;
  onOpenProject?: (path: string, branch?: string | null) => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const tabs = TABS[plane];
  const active = tabs.some((tab) => tab.id === pane) ? pane : tabs[0]?.id ?? 'none';
  const hiveCurrent = useHiveCurrent(plane === 'hive');
  const memories = useHiveMemories(plane === 'hive' && active === 'memory', projectDirectory);
  const projectLabel = projectDirectory
    ? projectDirectory.split('/').filter(Boolean).slice(-2).join('/')
    : 'No project';

  return (
    <View
      style={[
        styles.host,
        {
          backgroundColor: t.background,
          borderLeftColor: t.border,
          width: DESKTOP.utilityWidth,
        },
      ]}
    >
      <View style={[styles.header, { borderBottomColor: t.border }]}>
        <View style={styles.tabs}>
          {tabs.map((tab) => {
            const Icon = tab.icon;
            const selected = tab.id === active;
            return (
              <Pressable
                key={tab.id}
                onPress={() => onPaneChange(tab.id)}
                accessibilityLabel={tab.label}
                style={[
                  styles.tab,
                  selected && {
                    backgroundColor: t.glass.backgroundElevated,
                    borderColor: `${t.userMessage}44`,
                  },
                ]}
              >
                <Icon size={15} color={selected ? t.foreground : t.mutedForeground} />
              </Pressable>
            );
          })}
        </View>
        <Pressable onPress={onClose} style={styles.closeBtn} accessibilityLabel="Close utility pane">
          <X size={16} color={t.mutedForeground} />
        </Pressable>
      </View>

      {plane === 'code' && projectDirectory ? (
        <View style={[styles.projectStrip, { borderBottomColor: t.border }]}>
          <Text style={[styles.projectValue, { color: t.mutedForeground }]} numberOfLines={1}>
            {projectLabel}
          </Text>
        </View>
      ) : null}

      <View style={styles.body}>
        {plane === 'chat' ? (
          <>
            <Pane active={active === 'library'}>
              <ReportsContent visible={active === 'library'} />
            </Pane>
            <Pane active={active === 'connections'}>
              <ToolboxConnections visible={active === 'connections'} />
            </Pane>
          </>
        ) : null}

        {plane === 'code' ? (
          <>
            <Pane active={active === 'terminal'}>
              <DesktopTerminalPane visible={active === 'terminal'} projectDirectory={projectDirectory} />
            </Pane>
            <Pane active={active === 'changes'}>
              {projectDirectory ? (
                <ToolboxChanges visible={active === 'changes'} projectDirectory={projectDirectory} />
              ) : (
                <View style={{ flex: 1 }} />
              )}
            </Pane>
            <Pane active={active === 'browser'}>
              <DesktopBrowserPane visible={active === 'browser'} />
            </Pane>
          </>
        ) : null}

        {plane === 'hive' ? (
          <>
            <Pane active={active === 'schedule'}>
              <HiveScheduleView
                state={hiveCurrent}
                onSelectRun={(id) => onOpenHiveRun?.(id)}
                onOpenProject={onOpenProject}
              />
            </Pane>
            <Pane active={active === 'runs'}>
              <HiveRunsView state={hiveCurrent} onSelectRun={(id) => onOpenHiveRun?.(id)} />
            </Pane>
            <Pane active={active === 'memory'}>
              <HiveMemoryView workspaceDirectory={projectDirectory} state={memories} />
            </Pane>
          </>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  host: {
    borderLeftWidth: StyleSheet.hairlineWidth,
  },
  header: {
    minHeight: 42,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 10,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  tabs: {
    flex: 1,
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: 6,
    paddingVertical: 8,
  },
  tab: {
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: 'transparent',
    borderRadius: 8,
    width: 30,
    height: 30,
    alignItems: 'center',
    justifyContent: 'center',
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
  },
  closeBtn: {
    width: 30,
    height: 30,
    alignItems: 'center',
    justifyContent: 'center',
  },
  projectStrip: {
    minHeight: 30,
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: 12,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
  },
  projectLabel: {
    fontSize: 10,
    fontWeight: '700',
    letterSpacing: 0.4,
    textTransform: 'uppercase',
  },
  projectValue: {
    flex: 1,
    fontSize: 12,
    fontWeight: '600',
  },
  body: {
    flex: 1,
    position: 'relative',
  },
  pane: {
    ...StyleSheet.absoluteFillObject,
  },
  paneHidden: {
    opacity: 0,
  },
});
