import { useState, useEffect, useRef, useCallback } from "react";
import { View, Text, Pressable, StyleSheet, Platform } from "react-native";
import { Plus, X, TerminalSquare } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";

interface TerminalTab {
  id: string;
  label: string;
}

interface TerminalProps {
  visible: boolean;
}

export function Terminal({ visible }: TerminalProps) {
  const { theme } = useThemeContext();
  const { client, serverUrl } = useConnection();
  const t = theme.colors;

  const [tabs, setTabs] = useState<TerminalTab[]>([]);
  const [activeTab, setActiveTab] = useState<string | null>(null);
  const termInstancesRef = useRef<Map<string, any>>(new Map());
  const wsRef = useRef<Map<string, WebSocket>>(new Map());

  // Only render on web
  if (Platform.OS !== "web" || !visible) return null;

  const createTab = useCallback(async () => {
    if (!client || !serverUrl) return;
    Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
    try {
      const response = await fetch(`${serverUrl}/api/terminal`, { method: "POST" });
      const data = await response.json();
      const id = data.id || `term-${Date.now()}`;
      const tab: TerminalTab = { id, label: `Terminal ${tabs.length + 1}` };
      setTabs((prev) => [...prev, tab]);
      setActiveTab(id);
    } catch {
      const id = `term-${Date.now()}`;
      setTabs((prev) => [...prev, { id, label: `Terminal ${tabs.length + 1}` }]);
      setActiveTab(id);
    }
  }, [client, serverUrl, tabs.length]);

  const closeTab = useCallback((id: string) => {
    const ws = wsRef.current.get(id);
    if (ws) { ws.close(); wsRef.current.delete(id); }
    const inst = termInstancesRef.current.get(id);
    if (inst) { inst.dispose(); termInstancesRef.current.delete(id); }
    setTabs((prev) => {
      const next = prev.filter((tab) => tab.id !== id);
      if (activeTab === id) setActiveTab(next[0]?.id ?? null);
      return next;
    });
  }, [activeTab]);

  // Auto-create first tab
  useEffect(() => {
    if (visible && tabs.length === 0 && client) {
      createTab();
    }
  }, [visible, client, createTab]);

  return (
    <View style={[styles.container, { backgroundColor: t.background, borderTopColor: t.border }]}>
      {/* Tab bar */}
      <View style={[styles.tabBar, { borderBottomColor: t.border }]}>
        <TerminalSquare size={16} color={t.mutedForeground} strokeWidth={1.8} />
        {tabs.map((tab) => (
          <Pressable
            key={tab.id}
            onPress={() => setActiveTab(tab.id)}
            style={[styles.tab, activeTab === tab.id && { backgroundColor: `${t.userMessage}18` }]}
          >
            <Text style={[styles.tabLabel, { color: activeTab === tab.id ? t.foreground : t.mutedForeground }]} numberOfLines={1}>
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

      {/* Terminal viewport */}
      <View style={styles.terminalArea}>
        {activeTab ? (
          <XtermView
            key={activeTab}
            terminalId={activeTab}
            serverUrl={serverUrl}
            instancesRef={termInstancesRef}
            wsRef={wsRef}
          />
        ) : (
          <View style={styles.empty}>
            <Text style={[styles.emptyText, { color: t.mutedForeground }]}>No terminal open</Text>
          </View>
        )}
      </View>
    </View>
  );
}

// Lazy xterm.js integration — only loaded on web
function XtermView({
  terminalId,
  serverUrl,
  instancesRef,
  wsRef,
}: {
  terminalId: string;
  serverUrl: string | null;
  instancesRef: React.RefObject<Map<string, any>>;
  wsRef: React.RefObject<Map<string, WebSocket>>;
}) {
  const divRef = useRef<HTMLDivElement | null>(null);
  const { theme } = useThemeContext();
  const cleanupRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (Platform.OS !== "web" || !divRef.current || !serverUrl) return;

    let disposed = false;

    (async () => {
      const { Terminal } = await import("xterm");
      const { FitAddon } = await import("@xterm/addon-fit");
      if (disposed) return;

      const terminal = new Terminal({
        cursorBlink: true,
        fontSize: 14,
        fontFamily: "JetBrainsMono, Menlo, Monaco, monospace",
        theme: {
          background: theme.colors.background,
          foreground: theme.colors.foreground,
          cursor: theme.colors.userMessage,
        },
      });

      const fitAddon = new FitAddon();
      terminal.loadAddon(fitAddon);
      terminal.open(divRef.current!);
      fitAddon.fit();

      instancesRef.current!.set(terminalId, terminal);

      // WebSocket connection
      const wsUrl = serverUrl.replace(/^http/, "ws") + `/api/terminal/${terminalId}/ws`;
      const ws = new WebSocket(wsUrl);
      wsRef.current!.set(terminalId, ws);

      ws.onmessage = (event) => {
        terminal.write(typeof event.data === "string" ? event.data : new Uint8Array(event.data));
      };
      ws.onclose = () => terminal.write("\r\n[Connection closed]\r\n");

      terminal.onData((data: string) => {
        if (ws.readyState === WebSocket.OPEN) ws.send(data);
      });

      terminal.onResize(({ cols, rows }: { cols: number; rows: number }) => {
        if (ws.readyState === WebSocket.OPEN) {
          ws.send(JSON.stringify({ type: "resize", cols, rows }));
        }
      });

      const observer = new ResizeObserver(() => fitAddon.fit());
      observer.observe(divRef.current!);

      cleanupRef.current = () => {
        observer.disconnect();
        ws.close();
        terminal.dispose();
        instancesRef.current!.delete(terminalId);
        wsRef.current!.delete(terminalId);
      };
    })();

    return () => {
      disposed = true;
      cleanupRef.current?.();
      cleanupRef.current = null;
    };
  }, [terminalId, serverUrl, theme]);

  return (
    <View style={styles.xtermContainer}>
      {Platform.OS === "web" && (
        <div ref={divRef as any} style={{ width: "100%", height: "100%" }} />
      )}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    height: 300,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  tabBar: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  tab: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    paddingHorizontal: 10,
    paddingVertical: 5,
    borderRadius: 6,
  },
  tabLabel: {
    fontSize: 12,
    fontWeight: "500",
    maxWidth: 100,
  },
  addBtn: {
    padding: 4,
  },
  terminalArea: {
    flex: 1,
  },
  xtermContainer: {
    flex: 1,
  },
  empty: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
  },
  emptyText: {
    fontSize: 14,
  },
});
