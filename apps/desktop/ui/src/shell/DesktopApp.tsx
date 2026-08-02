import { useCallback, useEffect, useState } from 'react';
import { Modal, Pressable, StyleSheet, Text, View } from 'react-native';
import { PanelLeftOpen } from 'lucide-react-native';
import type { SessionResponse } from '@mitsuro/api';
import { SettingsPanel } from '@mobile/components/settings/SettingsPanel';
import { DirectoryPicker } from '@mobile/components/DirectoryPicker';
import { useConnection } from '@mobile/hooks/useConnection';
import {
  useSessionStore,
  useSessionsStore,
  useStores,
  useWorkspaceStore,
} from '@mobile/hooks/useStores';
import { useThemeContext } from '@mobile/hooks/useTheme';
import { useHiveCurrent } from '@mobile/components/hive/hooks/useHiveCurrent';
import { ConversationPlane } from '../planes/ConversationPlane';
import { HivePlane } from '../planes/HivePlane';
import { useDesktopKeyboard } from '../hooks/useDesktopKeyboard';
import { ContextRail } from './ContextRail';
import { PlaneRail } from './PlaneRail';
import { UtilityHost } from './UtilityHost';
import type { DesktopPlane, DesktopUtilityPane } from './types';

const DEFAULT_PANE: Record<DesktopPlane, DesktopUtilityPane> = {
  chat: 'library',
  code: 'terminal',
  hive: 'runs',
};

function DesktopBootScreen({
  status,
  isConfigured,
  onOpenSettings,
}: {
  status: string;
  isConfigured: boolean;
  onOpenSettings: () => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  return (
    <View style={[styles.boot, { backgroundColor: t.background }]}>
      <Text style={[styles.bootTitle, { color: t.foreground }]}>Mitsuro Desktop</Text>
      <Text style={[styles.bootBody, { color: t.mutedForeground }]}>
        {isConfigured
          ? status === 'connected'
            ? 'Preparing workspace…'
            : status === 'connecting'
              ? 'Connecting to local server…'
              : 'Waiting for server connection…'
          : 'Server connection is not configured yet.'}
      </Text>
      {!isConfigured ? (
        <Pressable
          onPress={onOpenSettings}
          style={[styles.bootButton, { borderColor: t.border, backgroundColor: t.glass.background }]}
        >
          <Text style={{ color: t.foreground, fontWeight: '700' }}>Open settings</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

export function DesktopApp() {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const { isConnected, isConfigured, status } = useConnection();
  const stores = useStores();
  const [settingsOpen, setSettingsOpen] = useState(false);

  if (!stores) {
    return (
      <>
        <DesktopBootScreen
          status={status}
          isConfigured={isConfigured}
          onOpenSettings={() => setSettingsOpen(true)}
        />
        <Modal visible={settingsOpen} transparent animationType="fade" onRequestClose={() => setSettingsOpen(false)}>
          <View style={styles.settingsBackdrop}>
            <Pressable style={StyleSheet.absoluteFill} onPress={() => setSettingsOpen(false)} />
            <View style={[styles.settingsPanel, { backgroundColor: t.background, borderColor: t.border }]}>
              <SettingsPanel active={settingsOpen} onClose={() => setSettingsOpen(false)} />
            </View>
          </View>
        </Modal>
      </>
    );
  }

  return (
    <DesktopAppReady
      stores={stores}
      settingsOpen={settingsOpen}
      setSettingsOpen={setSettingsOpen}
    />
  );
}

function DesktopAppReady({
  stores,
  settingsOpen,
  setSettingsOpen,
}: {
  stores: NonNullable<ReturnType<typeof useStores>>;
  settingsOpen: boolean;
  setSettingsOpen: (open: boolean) => void;
}) {
  const { theme } = useThemeContext();
  const t = theme.colors;
  const { client, isConnected } = useConnection();
  const [plane, setPlane] = useState<DesktopPlane>(() => {
    if (typeof window === 'undefined') return 'chat';
    const saved = window.localStorage.getItem('mitsuro.desktop.plane');
    return saved === 'code' || saved === 'hive' || saved === 'chat' ? saved : 'chat';
  });
  const [contextOpen, setContextOpen] = useState(true);
  const [utilityOpen, setUtilityOpen] = useState(false);
  const [utilityPane, setUtilityPane] = useState<DesktopUtilityPane>(DEFAULT_PANE[plane] ?? DEFAULT_PANE.chat);
  const [pickerOpen, setPickerOpen] = useState(false);

  const sessions = useSessionsStore((state) => state.sessions) as SessionResponse[];
  const sessionId = useSessionStore((state) => state.sessionId, plane) ?? null;
  const workspaceDirectory = useWorkspaceStore((state) => state.directory, plane) ?? null;
  const hiveCurrent = useHiveCurrent(plane === 'hive' || true);
  const attentionCount = hiveCurrent.current?.approvals?.length ?? 0;

  useEffect(() => {
    setUtilityPane(DEFAULT_PANE[plane]);
    // Keep utility closed by default; user opens it intentionally.
    // Avoid auto-opening empty Ghostty/Changes ornaments on bare Code plane.
    setUtilityOpen(false);
    if (typeof window !== 'undefined') {
      window.localStorage.setItem('mitsuro.desktop.plane', plane);
    }
  }, [plane]);

  useEffect(() => {
    if (!stores) return;
    void stores.sessions.getState().loadSessions();
  }, [stores, isConnected]);

  const bootstrapSession = useCallback(
    async (session: SessionResponse) => {
      if (!stores) return;
      const targetPlane = (session.session_type as DesktopPlane) || 'chat';
      const target = stores.modes[targetPlane];
      const directory = session.project_dir ?? session.working_dir ?? null;
      target.session.getState().initSession(
        session.id,
        session.title || '',
        session.permission_mode,
        session.session_type,
      );
      target.workspace.getState().initFromSession(
        session.id,
        directory,
        (session.workspace_mode ?? (directory ? 'selected' : 'neutral')) as any,
        session.target_branch ?? null,
      );
      await target.session.getState().loadSession(session.id);
      setPlane(targetPlane);
      await stores.sessions.getState().loadSessions();
    },
    [stores],
  );

  const loadSession = useCallback(
    async (session: SessionResponse) => {
      await bootstrapSession(session);
    },
    [bootstrapSession],
  );

  const handleNewSession = useCallback(async () => {
    if (!stores || !client) return;
    if (plane === 'code') {
      setPickerOpen(true);
      return;
    }
    const current = stores.modes[plane].session.getState();
    if (current.sessionId) current.detachSession();
    const session = await client.createSession(
      undefined,
      undefined,
      undefined,
      'neutral',
      plane,
      current.permissionMode,
    );
    await bootstrapSession(session);
  }, [bootstrapSession, client, plane, stores]);

  const handleNewSessionWithDir = useCallback(
    async (path: string) => {
      if (!stores || !client) return;
      if (!path) {
        setPickerOpen(true);
        return;
      }
      const current = stores.modes.code.session.getState();
      if (current.sessionId) current.detachSession();
      const session = await client.createSession(
        undefined,
        path,
        undefined,
        'selected',
        'code',
        current.permissionMode,
      );
      await bootstrapSession(session);
      setPickerOpen(false);
    },
    [bootstrapSession, client, stores],
  );

  const handleDeleteSession = useCallback(
    async (id: string) => {
      if (!stores) return;
      await stores.sessions.getState().deleteSession(id);
      if (sessionId === id) {
        stores.modes[plane].session.getState().clearSession();
      }
    },
    [plane, sessionId, stores],
  );

  useDesktopKeyboard({
    onPlane: setPlane,
    onToggleContext: () => setContextOpen((value) => !value),
    onToggleUtility: () => setUtilityOpen((value) => !value),
    onOpenSettings: () => setSettingsOpen(true),
    onNewSession: () => {
      void handleNewSession();
    },
  });

  return (
    <View style={[styles.root, { backgroundColor: t.background }]}> 
      <PlaneRail
        plane={plane}
        onPlaneChange={setPlane}
        onOpenSettings={() => setSettingsOpen(true)}
        attentionCount={attentionCount}
      />

      {contextOpen ? (
        <ContextRail
          plane={plane}
          sessions={sessions}
          activeSessionId={sessionId}
          onSelectSession={(session) => void loadSession(session)}
          onDeleteSession={(id) => void handleDeleteSession(id)}
          onNewSession={() => void handleNewSession()}
          onOpenProject={() => setPickerOpen(true)}
          onCollapse={() => setContextOpen(false)}
        />
      ) : (
        <Pressable
          onPress={() => setContextOpen(true)}
          style={[styles.expandContext, { borderColor: t.border, backgroundColor: t.glass.background }]}
          accessibilityLabel="Open context rail"
        >
          <PanelLeftOpen size={16} color={t.mutedForeground} />
        </Pressable>
      )}

      <View style={styles.main}>

        <View style={styles.workspace}>
          <View style={styles.canvas}>
            {plane === 'hive' ? (
              <HivePlane
                onOpenProject={(path, branch) => {
                  setPlane('code');
                  if (stores) {
                    stores.modes.code.workspace.getState().setWorkspace(path, null, 'selected', branch ?? null);
                  }
                }}
              />
            ) : (
              <ConversationPlane
                plane={plane}
                utilityOpen={utilityOpen}
                onToggleUtility={() => setUtilityOpen((value) => !value)}
                onOpenSettings={() => setSettingsOpen(true)}
                onOpenProject={() => setPickerOpen(true)}
              />
            )}
          </View>

          {utilityOpen ? (
            <UtilityHost
              plane={plane}
              pane={utilityPane}
              onPaneChange={setUtilityPane}
              onClose={() => setUtilityOpen(false)}
              projectDirectory={workspaceDirectory}
              onOpenHiveRun={(id) => {
                setPlane('hive');
                void stores?.modes.hive.session.getState().loadSession(id);
              }}
              onOpenProject={(path, branch) => {
                setPlane('code');
                stores?.modes.code.workspace.getState().setWorkspace(path, null, 'selected', branch ?? null);
              }}
            />
          ) : null}
        </View>
      </View>

      <Modal visible={settingsOpen} transparent animationType="fade" onRequestClose={() => setSettingsOpen(false)}>
        <View style={styles.settingsBackdrop}>
          <Pressable style={StyleSheet.absoluteFill} onPress={() => setSettingsOpen(false)} />
          <View style={[styles.settingsPanel, { backgroundColor: t.background, borderColor: t.border }]}> 
            <SettingsPanel active={settingsOpen} onClose={() => setSettingsOpen(false)} />
          </View>
        </View>
      </Modal>

      <DirectoryPicker
        visible={pickerOpen}
        onClose={() => setPickerOpen(false)}
        onSelect={(path) => void handleNewSessionWithDir(path)}
      />
    </View>
  );
}

const styles = StyleSheet.create({
  root: {
    flex: 1,
    flexDirection: 'row',
  },
  boot: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    gap: 10,
    paddingHorizontal: 24,
  },
  bootTitle: {
    fontSize: 22,
    fontWeight: '800',
    letterSpacing: -0.3,
  },
  bootBody: {
    fontSize: 13,
    lineHeight: 18,
    textAlign: 'center',
    maxWidth: 360,
  },
  bootButton: {
    marginTop: 8,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 10,
    paddingHorizontal: 14,
    paddingVertical: 10,
  },
  main: {
    flex: 1,
    minWidth: 0,
  },
  workspace: {
    flex: 1,
    flexDirection: 'row',
    minHeight: 0,
  },
  canvas: {
    flex: 1,
    minWidth: 0,
  },
  expandContext: {
    position: 'absolute',
    left: 60,
    top: 10,
    zIndex: 20,
    width: 34,
    height: 34,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: 'center',
    justifyContent: 'center',
  },
  settingsBackdrop: {
    flex: 1,
    backgroundColor: 'rgba(0,0,0,0.45)',
    justifyContent: 'center',
    alignItems: 'center',
    padding: 24,
  },
  settingsPanel: {
    width: '96%',
    maxWidth: 920,
    height: '92%',
    maxHeight: 820,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 14,
    overflow: 'hidden',
  },
});
