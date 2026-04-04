import { useState, useEffect, useCallback } from "react";
import { View, Text, Pressable, StyleSheet, Platform } from "react-native";
import { Globe, X, ExternalLink } from "lucide-react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import { useConnection } from "../../hooks/useConnection";

interface PortEntry {
  port: number;
  type: string;
  status: string;
}

interface PreviewTab {
  port: number;
  label: string;
}

interface WorkspacePreviewProps {
  visible: boolean;
}

export function WorkspacePreview({ visible }: WorkspacePreviewProps) {
  const { theme } = useThemeContext();
  const { serverUrl } = useConnection();
  const t = theme.colors;

  const [ports, setPorts] = useState<PortEntry[]>([]);
  const [tabs, setTabs] = useState<PreviewTab[]>([]);
  const [activePort, setActivePort] = useState<number | null>(null);

  if (Platform.OS !== "web" || !visible) return null;

  // Poll for ports
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
    const existing = tabs.find((t) => t.port === port);
    if (!existing) {
      setTabs((prev) => [...prev, { port, label: `localhost:${port}` }]);
    }
    setActivePort(port);
  }, [tabs]);

  const closePort = useCallback((port: number) => {
    setTabs((prev) => {
      const next = prev.filter((t) => t.port !== port);
      if (activePort === port) setActivePort(next[0]?.port ?? null);
      return next;
    });
  }, [activePort]);

  return (
    <View style={[styles.container, { backgroundColor: t.background, borderTopColor: t.border }]}>
      {/* Tab bar */}
      <View style={[styles.tabBar, { borderBottomColor: t.border }]}>
        <Globe size={16} color={t.mutedForeground} strokeWidth={1.8} />

        {tabs.map((tab) => (
          <Pressable
            key={tab.port}
            onPress={() => setActivePort(tab.port)}
            style={[styles.tab, activePort === tab.port && { backgroundColor: `${t.userMessage}18` }]}
          >
            <Text style={[styles.tabLabel, { color: activePort === tab.port ? t.foreground : t.mutedForeground }]} numberOfLines={1}>
              {tab.label}
            </Text>
            <Pressable onPress={() => closePort(tab.port)} hitSlop={8}>
              <X size={12} color={t.mutedForeground} strokeWidth={2} />
            </Pressable>
          </Pressable>
        ))}

        {/* Detected ports */}
        {ports.filter((p) => p.status === "ok" && !tabs.find((t) => t.port === p.port)).map((p) => (
          <Pressable key={p.port} onPress={() => openPort(p.port)} style={styles.portChip}>
            <Text style={[styles.portChipText, { color: t.mutedForeground }]}>:{p.port}</Text>
            <ExternalLink size={10} color={t.mutedForeground} strokeWidth={2} />
          </Pressable>
        ))}
      </View>

      {/* Preview iframe — web only */}
      <View style={styles.previewArea}>
        {activePort && Platform.OS === "web" ? (
          <div style={{ width: "100%", height: "100%" }}>
            <iframe
              src={`http://localhost:${activePort}`}
              style={{ width: "100%", height: "100%", border: "none" }}
              title={`Preview :${activePort}`}
            />
          </div>
        ) : (
          <View style={styles.empty}>
            <Text style={[styles.emptyText, { color: t.mutedForeground }]}>
              {ports.length > 0 ? "Select a port to preview" : "No dev servers detected"}
            </Text>
          </View>
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    height: 350,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  tabBar: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    paddingHorizontal: 12,
    paddingVertical: 6,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexWrap: "wrap",
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
    maxWidth: 120,
  },
  portChip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 4,
    paddingHorizontal: 8,
    paddingVertical: 4,
    borderRadius: 4,
    borderWidth: StyleSheet.hairlineWidth,
    borderColor: "rgba(255,255,255,0.1)",
  },
  portChipText: {
    fontSize: 11,
    fontWeight: "500",
  },
  previewArea: {
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
