import { useState, useEffect, useCallback } from 'react';
import { View, Text, Pressable, StyleSheet, Platform } from 'react-native';
import { Plus, X, TerminalSquare } from 'lucide-react-native';
import type { WebViewProps } from 'react-native-webview';
import * as Haptics from '../../platform/haptics';
import { useThemeContext } from '../../hooks/useTheme';
import { useConnection } from '../../hooks/useConnection';
import { Terminal } from '../desktop/Terminal';
import { getTerminalHtml } from './terminalHtml';

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
}

function NativeTerminal({ visible }: { visible: boolean }) {
  const { theme } = useThemeContext();
  const { client, serverUrl } = useConnection();
  const t = theme.colors;

  const [tabs, setTabs] = useState<NativeTerminalTab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);

  const createTab = useCallback(async () => {
    if (!client || !serverUrl) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      const response = await fetch(`${serverUrl}/api/terminal`, { method: 'POST' });
      const data = await response.json();
      const id = data.id || `term-${Date.now()}`;
      setTabs(prev => [...prev, { id, label: `Terminal ${prev.length + 1}` }]);
      setActiveTab(id);
    } catch {
      const id = `term-${Date.now()}`;
      setTabs(prev => [...prev, { id, label: `Terminal ${prev.length + 1}` }]);
      setActiveTab(id);
    }
  }, [client, serverUrl]);

  const closeTab = useCallback((id: string) => {
    setTabs(prev => {
      const next = prev.filter(tab => tab.id !== id);
      setActiveTab(current => current === id ? (next[0]?.id ?? null) : current);
      return next;
    });
  }, []);

  useEffect(() => {
    if (visible && tabs.length === 0 && client) {
      createTab();
    }
  }, [visible, client, createTab, tabs.length]);

  const wsUrl = serverUrl && activeTab
    ? serverUrl.replace(/^http/, 'ws') + `/api/terminal/${activeTab}/ws`
    : '';

  const html = activeTab
    ? getTerminalHtml(wsUrl, {
        background: t.background,
        foreground: t.foreground,
        cursor: t.userMessage,
      })
    : '';

  return (
    <View style={[styles.container, { backgroundColor: t.background }]}>
      <View style={[styles.tabBar, { borderBottomColor: t.border }]}>
        <TerminalSquare size={16} color={t.mutedForeground} strokeWidth={1.8} />
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
        {activeTab && visible && WebViewComponent ? (
          <WebViewComponent
            key={activeTab}
            source={{ html }}
            style={{ flex: 1, backgroundColor: t.background }}
            originWhitelist={['*']}
            javaScriptEnabled
            domStorageEnabled
          />
        ) : (
          <View style={styles.empty}>
            <TerminalSquare size={32} color={t.mutedForeground} strokeWidth={1.5} />
            <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
              {!WebViewComponent ? 'WebView not available' : !visible ? '' : 'No terminal open'}
            </Text>
          </View>
        )}
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
