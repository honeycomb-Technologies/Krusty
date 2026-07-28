import { useState, useEffect, useCallback, useMemo } from 'react';
import { ActivityIndicator, View, Text, Pressable, StyleSheet, Platform } from 'react-native';
import { Plus, X } from 'lucide-react-native';
import type { WebViewProps } from 'react-native-webview';
import type { PortEntry, PreviewSettings } from '@krusty/api';
import * as Haptics from '../../platform/haptics';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { WorkspacePreview } from '../desktop/WorkspacePreview';

let WebViewComponent: React.ComponentType<WebViewProps> | null = null;
if (Platform.OS !== 'web') {
  try {
    WebViewComponent = require('react-native-webview').default;
  } catch {
    // WebView not available
  }
}

interface PreviewTab {
  id: string;
  port: number | null;
  label: string;
}

// Survive toolbox sheet unmount so reopening Browser does not look like a fresh launch.
const browserSession: {
  ports: PortEntry[];
  settings: PreviewSettings | null;
  tabs: PreviewTab[];
  activeTabId: string | null;
  error: string | null;
} = {
  ports: [],
  settings: null,
  tabs: [],
  activeTabId: null,
  error: null,
};

interface ToolboxBrowserProps {
  visible: boolean;
}

export function ToolboxBrowser({ visible }: ToolboxBrowserProps) {
  if (Platform.OS === 'web') {
    return <WorkspacePreview visible={visible} style={{ flex: 1, height: undefined, borderTopWidth: 0 }} />;
  }

  return <NativeBrowser visible={visible} />;
}

function NativeBrowser({ visible }: { visible: boolean }) {
  const { theme } = useThemeContext();
  const { client, serverUrl, serverToken } = useConnection();
  const t = theme.colors;

  const [ports, setPorts] = useState<PortEntry[]>(browserSession.ports);
  const [settings, setSettings] = useState<PreviewSettings | null>(browserSession.settings);
  const [tabs, setTabs] = useState<PreviewTab[]>(browserSession.tabs);
  const [activeTabId, setActiveTabId] = useState<string | null>(browserSession.activeTabId);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [error, setError] = useState<string | null>(browserSession.error);

  const activeTab = tabs.find((tab) => tab.id === activeTabId) ?? null;

  useEffect(() => {
    browserSession.ports = ports;
    browserSession.settings = settings;
    browserSession.tabs = tabs;
    browserSession.activeTabId = activeTabId;
    browserSession.error = error;
  }, [activeTabId, error, ports, settings, tabs]);


  const previewUrl = useMemo(() => {
    if (!serverUrl || activeTab?.port === null || activeTab?.port === undefined) {
      return null;
    }
    return `${serverUrl.replace(/\/+$/, '')}/api/ports/${activeTab.port}/proxy`;
  }, [activeTab?.port, serverUrl]);

  const availablePorts = useMemo(
    () =>
      ports.filter(
        (port) => port.active && port.is_previewable_http,
      ),
    [ports],
  );

  const loadPorts = useCallback(async (background = false) => {
    if (!client) return;

    if (background) {
      setRefreshing(true);
    } else {
      setLoading(true);
    }

    try {
      const response = await client.getPorts();
      setPorts(response.ports);
      setSettings(response.settings);
      setError(response.discovery_error ?? null);
      setTabs((current) =>
        current.map((tab) => {
          if (tab.port === null) return tab;
          const match = response.ports.find((port) => port.port === tab.port);
          return match ? { ...tab, label: match.name } : tab;
        }),
      );
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load preview ports.');
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  }, [client]);

  useEffect(() => {
    if (!client || !visible) return;

    void loadPorts();
    const intervalMs = Math.max(2, settings?.auto_refresh_secs ?? 5) * 1000;
    const interval = setInterval(() => {
      void loadPorts(true);
    }, intervalMs);
    return () => clearInterval(interval);
  }, [client, loadPorts, settings?.auto_refresh_secs, visible]);

  const createBlankTab = useCallback(() => {
    const tab: PreviewTab = {
      id: `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
      port: null,
      label: 'New Tab',
    };
    setTabs((current) => [...current, tab]);
    setActiveTabId(tab.id);
    setError(null);
  }, []);

  useEffect(() => {
    if (!visible || tabs.length > 0) return;

    const tab: PreviewTab = {
      id: `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
      port: null,
      label: 'New Tab',
    };
    setTabs([tab]);
    setActiveTabId(tab.id);
  }, [tabs.length, visible]);

  const openPort = useCallback((port: PortEntry) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setTabs(prev => {
      if (activeTab?.port === null) {
        return prev.map((tab) =>
          tab.id === activeTab.id
            ? { ...tab, port: port.port, label: port.name || `Port ${port.port}` }
            : tab,
        );
      }

      const tab: PreviewTab = {
        id: `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
        port: port.port,
        label: port.name || `Port ${port.port}`,
      };
      setActiveTabId(tab.id);
      return [...prev, tab];
    });
    if (activeTab?.port === null) {
      setActiveTabId(activeTab.id);
    }
  }, [activeTab]);

  const closeTab = useCallback((tabId: string) => {
    setTabs(prev => {
      const index = prev.findIndex((tab) => tab.id === tabId);
      const next = prev.filter(t => t.id !== tabId);
      if (next.length === 0) {
        const blankTab: PreviewTab = {
          id: `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
          port: null,
          label: 'New Tab',
        };
        setActiveTabId(blankTab.id);
        return [blankTab];
      }
      setActiveTabId(current =>
        current === tabId
          ? (next[Math.max(0, index - 1)]?.id ?? next[0]?.id ?? null)
          : current,
      );
      return next;
    });
  }, []);

  return (
    <View style={[styles.container, { backgroundColor: t.background }]}>
      <View style={[styles.tabBar, { borderBottomColor: t.border }]}>
        {tabs.map(tab => (
          <Pressable
            key={tab.id}
            onPress={() => setActiveTabId(tab.id)}
            style={[styles.tab, activeTabId === tab.id && { backgroundColor: `${t.userMessage}18` }]}
          >
            <Text
              style={[styles.tabLabel, { color: activeTabId === tab.id ? t.foreground : t.mutedForeground }]}
              numberOfLines={1}
            >
              {tab.label}
            </Text>
            <Pressable onPress={() => closeTab(tab.id)} hitSlop={8}>
              <X size={12} color={t.mutedForeground} strokeWidth={2} />
            </Pressable>
          </Pressable>
        ))}
        <Pressable onPress={createBlankTab} style={styles.iconBtn}>
          <Plus size={16} color={t.mutedForeground} strokeWidth={2} />
        </Pressable>
        <View style={styles.grow} />
        {refreshing ? <ActivityIndicator color={t.userMessage} size="small" /> : null}
      </View>

      <View style={styles.previewArea}>
        {WebViewComponent
          ? tabs
              .filter((tab) => tab.port !== null)
              .map((tab) => {
                const isActive = visible && tab.id === activeTabId;
                const uri = serverUrl
                  ? `${serverUrl.replace(/\/+$/, '')}/api/ports/${tab.port}/proxy`
                  : null;
                if (!uri) {
                  return null;
                }
                return (
                  <View
                    key={tab.id}
                    pointerEvents={isActive ? 'auto' : 'none'}
                    style={[styles.webviewHost, !isActive && styles.hiddenSurface]}
                  >
                    <WebViewComponent
                      source={{
                        uri,
                        headers: serverToken
                          ? { Authorization: `Bearer ${serverToken}` }
                          : undefined,
                      }}
                      style={{ flex: 1 }}
                      originWhitelist={['*']}
                      javaScriptEnabled
                      domStorageEnabled
                    />
                  </View>
                );
              })
          : null}

        {!previewUrl || !WebViewComponent ? (
          <View style={styles.quickPage}>
            {loading ? (
              <>
                <ActivityIndicator color={t.userMessage} size="small" />
                <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
                  Loading preview ports...
                </Text>
              </>
            ) : availablePorts.length > 0 ? (
              <View style={styles.portStack}>
                {availablePorts.map((port) => (
                  <Pressable
                    key={port.port}
                    onPress={() => openPort(port)}
                    style={[
                      styles.portButton,
                      { backgroundColor: t.card, borderColor: t.border },
                    ]}
                  >
                    <Text style={[styles.portButtonText, { color: t.foreground }]}>
                      :{port.port}
                    </Text>
                  </Pressable>
                ))}
              </View>
            ) : (
              <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
                {!visible
                  ? ''
                  : error
                    ? error
                    : 'No previewable ports are active right now.'}
              </Text>
            )}
          </View>
        ) : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  tabBar: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexWrap: 'wrap',
  },
  grow: {
    flex: 1,
  },
  tab: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderRadius: 6,
  },
  tabLabel: {
    fontSize: 12,
    fontWeight: '500',
    maxWidth: 120,
  },
  iconBtn: {
    padding: 6,
    borderRadius: 8,
  },
  previewArea: {
    flex: 1,
  },
  webviewHost: {
    ...StyleSheet.absoluteFillObject,
  },
  hiddenSurface: {
    opacity: 0,
  },
  quickPage: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    gap: 12,
    padding: 24,
  },
  emptyText: {
    fontSize: 14,
  },
  portStack: {
    width: '100%',
    maxWidth: 220,
    gap: 10,
  },
  portButton: {
    minHeight: 44,
    paddingHorizontal: 18,
    paddingVertical: 11,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 12,
    flexDirection: 'row',
    alignItems: 'center',
    justifyContent: 'center',
  },
  portButtonText: {
    fontSize: 15,
    fontWeight: '700',
  },
});
