import { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import { ActivityIndicator, View, Text, Pressable, StyleSheet, Platform } from 'react-native';
import { Plus, RefreshCw, X } from 'lucide-react-native';
import type { WebViewProps } from 'react-native-webview';
import type { PortEntry, PreviewSettings } from '@krusty/api';
import { trackKrustyPerformanceResource } from '@krusty/state';
import * as Haptics from '../../platform/haptics';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { WorkspacePreview } from '../desktop/WorkspacePreview';
import { recordWebViewDiagnostic } from '../../diagnostics/mobileDiagnostics';

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
  revision?: number;
  recoveryAttempts?: number;
  recoveryBlocked?: boolean;
  lastTerminationAt?: number;
}

const MAX_BROWSER_TABS = 4;
const WEBVIEW_MOUNT_SETTLE_MS = 250;
const WEBVIEW_RECOVERY_COOLDOWN_MS = 1_500;
const MAX_AUTOMATIC_RECOVERIES = 1;

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
  const loadPromiseRef = useRef<Promise<void> | null>(null);
  const tabsRef = useRef(tabs);
  const visibleRef = useRef(visible);
  const activeTabIdRef = useRef(activeTabId);
  const recoveryTimersRef = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const [renderedTabId, setRenderedTabId] = useState<string | null>(null);
  tabsRef.current = tabs;
  visibleRef.current = visible;
  activeTabIdRef.current = activeTabId;

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

  const loadPorts = useCallback((background = false) => {
    if (!client) return Promise.resolve();
    if (loadPromiseRef.current) return loadPromiseRef.current;

    const releaseRequest = trackKrustyPerformanceResource('toolbox_requests');
    if (background) {
      setRefreshing(true);
    } else {
      setLoading(true);
    }

    const request = (async () => {
      try {
        const response = await client.getPorts();
        setPorts((current) => JSON.stringify(current) === JSON.stringify(response.ports)
          ? current
          : response.ports);
        setSettings((current) => JSON.stringify(current) === JSON.stringify(response.settings)
          ? current
          : response.settings);
        setError((current) => current === (response.discovery_error ?? null)
          ? current
          : response.discovery_error ?? null);
        setTabs((current) => {
          let changed = false;
          const next = current.map((tab) => {
            if (tab.port === null) return tab;
            const match = response.ports.find((port) => port.port === tab.port);
            if (!match || match.name === tab.label) return tab;
            changed = true;
            return { ...tab, label: match.name };
          });
          return changed ? next : current;
        });
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to load preview ports.');
      } finally {
        releaseRequest();
        setLoading(false);
        setRefreshing(false);
      }
    })();
    loadPromiseRef.current = request;
    void request.finally(() => {
      if (loadPromiseRef.current === request) loadPromiseRef.current = null;
    });
    return request;
  }, [client]);

  useEffect(() => {
    if (!client || !visible) return;

    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;
    const poll = async (background: boolean) => {
      await loadPorts(background);
      if (cancelled) return;
      const intervalMs = Math.max(
        2,
        browserSession.settings?.auto_refresh_secs ?? settings?.auto_refresh_secs ?? 5,
      ) * 1000;
      timer = setTimeout(() => void poll(true), intervalMs);
    };
    void poll(false);
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [client, loadPorts, visible]);

  // Coalesce frantic tab/open transitions before constructing another native
  // WebView. The selected tab updates immediately; native process ownership
  // moves only after the interaction settles.
  useEffect(() => {
    if (!visible || !activeTabId) {
      setRenderedTabId(null);
      return;
    }
    const timer = setTimeout(() => {
      if (visibleRef.current) setRenderedTabId(activeTabId);
    }, WEBVIEW_MOUNT_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [activeTabId, visible]);

  useEffect(() => {
    if (visible) {
      setTabs((current) => current.map((tab) =>
        tab.recoveryBlocked || (tab.recoveryAttempts ?? 0) > 0
          ? { ...tab, recoveryBlocked: false, recoveryAttempts: 0 }
          : tab));
      return;
    }
    for (const timer of recoveryTimersRef.current.values()) clearTimeout(timer);
    recoveryTimersRef.current.clear();
  }, [visible]);

  useEffect(() => () => {
    for (const timer of recoveryTimersRef.current.values()) clearTimeout(timer);
    recoveryTimersRef.current.clear();
  }, []);

  const retryWebView = useCallback((tabId: string) => {
    recordWebViewDiagnostic('browser', 'reload');
    const timer = recoveryTimersRef.current.get(tabId);
    if (timer) clearTimeout(timer);
    recoveryTimersRef.current.delete(tabId);
    setTabs((current) => current.map((tab) =>
      tab.id === tabId
        ? {
            ...tab,
            revision: (tab.revision ?? 0) + 1,
            recoveryAttempts: 0,
            recoveryBlocked: false,
          }
        : tab));
  }, []);

  const handleWebViewTerminated = useCallback((tabId: string) => {
    if (!visibleRef.current || activeTabIdRef.current !== tabId) return;
    const tab = tabsRef.current.find((candidate) => candidate.id === tabId);
    if (!tab) return;
    const now = Date.now();
    if (now - (tab.lastTerminationAt ?? 0) < 250) return;
    const attempts = tab.recoveryAttempts ?? 0;
    recordWebViewDiagnostic('browser', 'terminate');
    setTabs((current) => current.map((candidate) =>
      candidate.id === tabId
        ? {
            ...candidate,
            recoveryAttempts: attempts + 1,
            recoveryBlocked: true,
            lastTerminationAt: now,
          }
        : candidate));
    if (attempts >= MAX_AUTOMATIC_RECOVERIES) return;

    const existing = recoveryTimersRef.current.get(tabId);
    if (existing) clearTimeout(existing);
    const timer = setTimeout(() => {
      recoveryTimersRef.current.delete(tabId);
      if (!visibleRef.current || activeTabIdRef.current !== tabId) return;
      setTabs((current) => current.map((candidate) =>
        candidate.id === tabId
          ? {
              ...candidate,
              revision: (candidate.revision ?? 0) + 1,
              recoveryBlocked: false,
            }
          : candidate));
    }, WEBVIEW_RECOVERY_COOLDOWN_MS);
    recoveryTimersRef.current.set(tabId, timer);
  }, []);

  const createBlankTab = useCallback(() => {
    setTabs((current) => {
      if (current.length >= MAX_BROWSER_TABS) {
        const active = current[current.length - 1];
        if (active) setActiveTabId(active.id);
        return current;
      }
      const tab: PreviewTab = {
        id: `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
        port: null,
        label: 'New Tab',
        revision: 0,
      };
      setActiveTabId(tab.id);
      return [...current, tab];
    });
    setError(null);
  }, []);

  useEffect(() => {
    if (!visible || tabs.length > 0) return;

    const tab: PreviewTab = {
      id: `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
      port: null,
      label: 'New Tab',
      revision: 0,
    };
    setTabs([tab]);
    setActiveTabId(tab.id);
  }, [tabs.length, visible]);

  const openPort = useCallback((port: PortEntry) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setTabs((prev) => {
      if (activeTab?.port === null) {
        return prev.map((tab) =>
          tab.id === activeTab.id
            ? { ...tab, port: port.port, label: port.name || `Port ${port.port}` }
            : tab,
        );
      }

      // Cap concurrent browser tabs so inactive WebViews stay frozen and budgets
      // do not grow unbounded. Prefer reusing the active tab when full.
      if (prev.length >= MAX_BROWSER_TABS) {
        if (!activeTabId) {
          return prev;
        }
        return prev.map((tab) =>
          tab.id === activeTabId
            ? { ...tab, port: port.port, label: port.name || `Port ${port.port}` }
            : tab,
        );
      }

      const tab: PreviewTab = {
        id: `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
        port: port.port,
        label: port.name || `Port ${port.port}`,
        revision: 0,
      };
      setActiveTabId(tab.id);
      return [...prev, tab];
    });
    if (activeTab?.port === null) {
      setActiveTabId(activeTab.id);
    }
  }, [activeTab, activeTabId]);

  const closeTab = useCallback((tabId: string) => {
    setTabs(prev => {
      const index = prev.findIndex((tab) => tab.id === tabId);
      const next = prev.filter(t => t.id !== tabId);
      if (next.length === 0) {
        const blankTab: PreviewTab = {
          id: `preview-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`,
          port: null,
          label: 'New Tab',
          revision: 0,
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
        {WebViewComponent && visible
          ? tabs
              .filter((tab) => tab.port !== null && tab.id === renderedTabId)
              .map((tab) => {
                // Tab metadata survives closure, but native WebViews are
                // deliberately cold-restored to avoid hidden WebContent CPU.
                const uri = serverUrl
                  ? `${serverUrl.replace(/\/+$/, '')}/api/ports/${tab.port}/proxy`
                  : null;
                if (!uri) {
                  return null;
                }
                return (
                  <View
                    key={`${tab.id}:${tab.revision ?? 0}`}
                    pointerEvents={tab.id === activeTabId ? 'auto' : 'none'}
                    style={styles.webviewHost}
                  >
                    {!tab.recoveryBlocked ? <WebViewComponent
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
                      onContentProcessDidTerminate={() => handleWebViewTerminated(tab.id)}
                      onRenderProcessGone={() => handleWebViewTerminated(tab.id)}
                    /> : (
                      <View style={styles.recoveryState}>
                        <Text style={[styles.emptyText, { color: t.mutedForeground }]}>Preview paused</Text>
                        <Pressable
                          accessibilityRole="button"
                          accessibilityLabel="Reload preview"
                          onPress={() => retryWebView(tab.id)}
                          style={[styles.retryButton, { borderColor: t.border }]}
                        >
                          <RefreshCw size={15} color={t.foreground} strokeWidth={2} />
                          <Text style={[styles.retryText, { color: t.foreground }]}>Reload</Text>
                        </Pressable>
                      </View>
                    )}
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
  recoveryState: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    gap: 12,
  },
  retryButton: {
    minHeight: 40,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    paddingHorizontal: 14,
    borderRadius: 10,
    borderWidth: StyleSheet.hairlineWidth,
  },
  retryText: {
    fontSize: 13,
    fontWeight: '600',
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
