import { useState, useEffect, useCallback, useRef } from 'react';
import { View, Text, Pressable, StyleSheet, Platform } from 'react-native';
import { Plus, RefreshCw, X, TerminalSquare } from 'lucide-react-native';
import type { WebViewProps } from 'react-native-webview';
import * as Haptics from '../../platform/haptics';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { Terminal } from '../desktop/Terminal';
import { buildTerminalWebSocketUrl } from '../terminalUrl';
import { getTerminalHtml } from './terminalHtml';
import { recordWebViewDiagnostic } from '../../diagnostics/mobileDiagnostics';

let WebViewComponent: React.ComponentType<WebViewProps> | null = null;
if (Platform.OS !== 'web') {
  try {
    WebViewComponent = require('react-native-webview').default;
  } catch {
    // WebView not available
  }
}

interface ToolboxTerminalProps {
  visible: boolean;
}

export function ToolboxTerminal({ visible }: ToolboxTerminalProps) {
  if (Platform.OS === 'web') {
    return <Terminal visible={visible} style={{ flex: 1, height: undefined, borderTopWidth: 0 }} />;
  }

  return <NativeTerminal visible={visible} />;
}

interface NativeTerminalTab {
  id: string;
  label: string;
  html: string;
  revision?: number;
  recoveryAttempts?: number;
  recoveryBlocked?: boolean;
  lastTerminationAt?: number;
}

const MAX_TERMINAL_TABS = 4;
const WEBVIEW_MOUNT_SETTLE_MS = 250;
const WEBVIEW_RECOVERY_COOLDOWN_MS = 1_500;
const MAX_AUTOMATIC_RECOVERIES = 1;

// Survive toolbox sheet unmount so reopening Terminal keeps existing sessions.
const terminalSession: {
  tabs: NativeTerminalTab[];
  activeTab: string | null;
} = {
  tabs: [],
  activeTab: null,
};

function NativeTerminal({ visible }: { visible: boolean }) {
  const { theme } = useThemeContext();
  const { serverUrl, serverToken } = useConnection();
  const t = theme.colors;

  const [tabs, setTabs] = useState<NativeTerminalTab[]>(terminalSession.tabs);
  const [activeTab, setActiveTab] = useState<string | null>(terminalSession.activeTab);
  const [renderedTabId, setRenderedTabId] = useState<string | null>(null);
  const tabsRef = useRef(tabs);
  const visibleRef = useRef(visible);
  const activeTabRef = useRef(activeTab);
  const recoveryTimersRef = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  tabsRef.current = tabs;
  visibleRef.current = visible;
  activeTabRef.current = activeTab;

  useEffect(() => {
    terminalSession.tabs = tabs;
    terminalSession.activeTab = activeTab;
  }, [activeTab, tabs]);

  const createTab = useCallback(() => {
    if (!serverUrl) return;
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setTabs((prev) => {
      if (prev.length >= MAX_TERMINAL_TABS) {
        // Cap concurrent terminal processes. Reuse newest tab shell.
        const active = prev[prev.length - 1];
        if (active) setActiveTab(active.id);
        return prev;
      }
      const id = createTerminalId();
      const html = getTerminalHtml(buildTerminalWebSocketUrl(serverUrl, serverToken), {
        background: t.background,
        foreground: t.foreground,
        cursor: t.userMessage,
      });
      setActiveTab(id);
      return [...prev, { id, label: `Terminal ${prev.length + 1}`, html, revision: 0 }];
    });
  }, [serverToken, serverUrl, t.background, t.foreground, t.userMessage]);

  const closeTab = useCallback((id: string) => {
    setTabs(prev => {
      const next = prev.filter(tab => tab.id !== id);
      setActiveTab(current => current === id ? (next[0]?.id ?? null) : current);
      return next;
    });
  }, []);

  useEffect(() => {
    if (visible && tabs.length === 0 && serverUrl) {
      createTab();
    }
  }, [visible, createTab, serverUrl, tabs.length]);

  // Keep rapid tab/open taps in JS until the selection settles; constructing a
  // WKWebView, loading xterm and opening a PTY must remain a bounded operation.
  useEffect(() => {
    if (!visible || !activeTab) {
      setRenderedTabId(null);
      return;
    }
    const timer = setTimeout(() => {
      if (visibleRef.current) setRenderedTabId(activeTab);
    }, WEBVIEW_MOUNT_SETTLE_MS);
    return () => clearTimeout(timer);
  }, [activeTab, visible]);

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
    recordWebViewDiagnostic('terminal', 'reload');
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
    if (!visibleRef.current || activeTabRef.current !== tabId) return;
    const tab = tabsRef.current.find((candidate) => candidate.id === tabId);
    if (!tab) return;
    const now = Date.now();
    if (now - (tab.lastTerminationAt ?? 0) < 250) return;
    const attempts = tab.recoveryAttempts ?? 0;
    recordWebViewDiagnostic('terminal', 'terminate');
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
      if (!visibleRef.current || activeTabRef.current !== tabId) return;
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

  return (
    <View style={[styles.container, { backgroundColor: t.background }]}>
      <View style={[styles.tabBar, { borderBottomColor: t.border }]}>
        {tabs.map(tab => (
          <Pressable
            key={tab.id}
            onPress={() => setActiveTab(tab.id)}
            style={[styles.tab, activeTab === tab.id && { backgroundColor: `${t.userMessage}18` }]}
          >
            <Text
              style={[styles.tabLabel, { color: activeTab === tab.id ? t.foreground : t.mutedForeground }]}
              numberOfLines={1}
            >
              {tab.label}
            </Text>
            <Pressable onPress={() => closeTab(tab.id)} hitSlop={8}>
              <X size={12} color={t.mutedForeground} strokeWidth={2} />
            </Pressable>
          </Pressable>
        ))}
        <Pressable onPress={createTab} style={styles.addBtn}>
          <Plus size={16} color={t.mutedForeground} strokeWidth={2} />
        </Pressable>
      </View>

      <View style={styles.terminalArea}>
        {WebViewComponent && visible
          ? tabs.map((tab) => {
              // Tab metadata survives closure. The WebView/websocket/PTY is a
              // deliberate cold restore so hidden terminals consume no CPU.
              if (tab.id !== renderedTabId) {
                return null;
              }
              return (
                <View
                  key={`${tab.id}:${tab.revision ?? 0}`}
                  pointerEvents={tab.id === activeTab ? 'auto' : 'none'}
                  style={styles.webviewHost}
                >
                  {!tab.recoveryBlocked ? <WebViewComponent
                    source={{ html: tab.html }}
                    style={{ flex: 1, backgroundColor: t.background }}
                    originWhitelist={['*']}
                    javaScriptEnabled
                    domStorageEnabled
                    onContentProcessDidTerminate={() => handleWebViewTerminated(tab.id)}
                    onRenderProcessGone={() => handleWebViewTerminated(tab.id)}
                  /> : (
                    <View style={styles.recoveryState}>
                      <Text style={[styles.emptyText, { color: t.mutedForeground }]}>Terminal paused</Text>
                      <Pressable
                        accessibilityRole="button"
                        accessibilityLabel="Reload terminal"
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

        {!activeTab || !visible || !WebViewComponent ? (
          <View style={styles.empty}>
            <TerminalSquare size={32} color={t.mutedForeground} strokeWidth={1.5} />
            <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
              {!WebViewComponent
                ? 'WebView not available'
                : !serverUrl
                  ? 'Connect to a server to open a terminal.'
                  : !visible
                    ? ''
                    : 'No terminal open'}
            </Text>
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
    maxWidth: 100,
  },
  addBtn: {
    padding: 4,
  },
  terminalArea: {
    flex: 1,
  },
  webviewHost: {
    ...StyleSheet.absoluteFillObject,
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
  hiddenSurface: {
    opacity: 0,
  },
  empty: {
    flex: 1,
    justifyContent: 'center',
    alignItems: 'center',
    gap: 12,
  },
  emptyText: {
    fontSize: 14,
  },
});

function createTerminalId(): string {
  if (typeof crypto !== 'undefined' && 'randomUUID' in crypto) {
    return crypto.randomUUID();
  }

  return `term-${Date.now()}-${Math.random().toString(36).slice(2, 10)}`;
}
