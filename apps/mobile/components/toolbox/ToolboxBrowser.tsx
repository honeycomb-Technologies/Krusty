import { useState, useEffect, useCallback } from 'react';
import { View, Text, Pressable, StyleSheet, Platform } from 'react-native';
import { Globe, X, ExternalLink } from 'lucide-react-native';
import type { WebViewProps } from 'react-native-webview';
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

interface PortEntry {
  port: number;
  type: string;
  status: string;
}

interface PreviewTab {
  port: number;
  label: string;
}

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
  const { serverUrl } = useConnection();
  const t = theme.colors;

  const [ports, setPorts] = useState<PortEntry[]>([]);
  const [tabs, setTabs] = useState<PreviewTab[]>([]);
  const [activePort, setActivePort] = useState<number | null>(null);

  useEffect(() => {
    if (!serverUrl || !visible) return;
    let mounted = true;

    const poll = async () => {
      try {
        const res = await fetch(`${serverUrl}/api/workspace/ports`);
        const data = await res.json();
        if (mounted && data.ports) setPorts(data.ports);
      } catch { /* silent */ }
    };

    poll();
    const interval = setInterval(poll, 5000);
    return () => { mounted = false; clearInterval(interval); };
  }, [serverUrl, visible]);

  const openPort = useCallback((port: number) => {
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    setTabs(prev => {
      if (prev.some(t => t.port === port)) return prev;
      return [...prev, { port, label: `localhost:${port}` }];
    });
    setActivePort(port);
  }, []);

  const closePort = useCallback((port: number) => {
    setTabs(prev => {
      const next = prev.filter(t => t.port !== port);
      setActivePort(current => current === port ? (next[0]?.port ?? null) : current);
      return next;
    });
  }, []);

  return (
    <View style={[styles.container, { backgroundColor: t.background }]}>
      <View style={[styles.tabBar, { borderBottomColor: t.border }]}>
        <Globe size={16} color={t.mutedForeground} strokeWidth={1.8} />
        {tabs.map(tab => (
          <Pressable
            key={tab.port}
            onPress={() => setActivePort(tab.port)}
            style={[styles.tab, activePort === tab.port && { backgroundColor: `${t.userMessage}18` }]}
          >
            <Text
              style={[styles.tabLabel, { color: activePort === tab.port ? t.foreground : t.mutedForeground }]}
              numberOfLines={1}
            >
              {tab.label}
            </Text>
            <Pressable onPress={() => closePort(tab.port)} hitSlop={8}>
              <X size={12} color={t.mutedForeground} strokeWidth={2} />
            </Pressable>
          </Pressable>
        ))}
        {ports
          .filter(p => p.status === 'ok' && !tabs.some(t => t.port === p.port))
          .map(p => (
            <Pressable key={p.port} onPress={() => openPort(p.port)} style={styles.portChip}>
              <Text style={[styles.portChipText, { color: t.mutedForeground }]}>:{p.port}</Text>
              <ExternalLink size={10} color={t.mutedForeground} strokeWidth={2} />
            </Pressable>
          ))}
      </View>

      <View style={styles.previewArea}>
        {activePort && visible && WebViewComponent ? (
          <WebViewComponent
            key={activePort}
            source={{ uri: `http://localhost:${activePort}` }}
            style={{ flex: 1 }}
            originWhitelist={['*']}
            javaScriptEnabled
            domStorageEnabled
          />
        ) : (
          <View style={styles.empty}>
            <Globe size={32} color={t.mutedForeground} strokeWidth={1.5} />
            <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
              {!visible ? '' : ports.length > 0 ? 'Select a port to preview' : 'No dev servers detected'}
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
    flexWrap: 'wrap',
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
  portChip: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 4,
    paddingHorizontal: 8,
    paddingVertical: 4,
    borderRadius: 4,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: 'rgba(255,255,255,0.1)',
  },
  portChipText: {
    fontSize: 11,
    fontWeight: '500',
  },
  previewArea: {
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
