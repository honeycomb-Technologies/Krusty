import { type ReactNode, useCallback, useEffect, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  View,
} from "react-native";
import { ChevronDown, ChevronRight } from "lucide-react-native";
import type { McpServerResponse, SkillInfo } from "@krusty/api";

import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";

interface ToolboxConnectionsProps {
  visible: boolean;
  onOpenSettings?: () => void;
}

type SectionKey = "mcp" | "skills";

export function ToolboxConnections({ visible }: ToolboxConnectionsProps) {
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [servers, setServers] = useState<McpServerResponse[]>([]);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [expanded, setExpanded] = useState<Record<SectionKey, boolean>>({
    mcp: true,
    skills: false,
  });
  const [loading, setLoading] = useState(false);
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!client || !visible) {
      return;
    }
    setLoading(true);
    setMessage(null);
    try {
      const [nextServers, nextSkills] = await Promise.all([
        client.getMcpServers(),
        client.getSkills(),
      ]);
      setServers(nextServers);
      setSkills(nextSkills);
    } catch (error) {
      setMessage(error instanceof Error ? error.message : "Unable to load connections.");
    } finally {
      setLoading(false);
    }
  }, [client, visible]);

  useEffect(() => {
    void load();
  }, [load]);

  const connectedCount = servers.filter((server) => server.connected).length;
  const enabledSkillCount = skills.filter((skill) => skill.enabled).length;

  const toggleSection = (section: SectionKey) => {
    setExpanded((current) => ({ ...current, [section]: !current[section] }));
  };

  const toggleMcp = useCallback(
    async (server: McpServerResponse) => {
      if (!client) return;
      const key = `mcp:${server.name}`;
      setBusyKey(key);
      setMessage(null);
      try {
        const updated = server.connected
          ? await client.disconnectMcpServer(server.name)
          : await client.connectMcpServer(server.name);
        setServers((current) =>
          current.map((entry) => (entry.name === updated.name ? updated : entry)),
        );
      } catch (error) {
        setMessage(
          error instanceof Error
            ? error.message
            : `Unable to update ${server.name}.`,
        );
      } finally {
        setBusyKey(null);
      }
    },
    [client],
  );

  const toggleSkill = useCallback(
    async (skill: SkillInfo) => {
      if (!client) return;
      const key = `skill:${skill.name}`;
      setBusyKey(key);
      setMessage(null);
      try {
        const updated = await client.updateSkillPolicy(skill.name, {
          enabled: !skill.enabled,
        });
        setSkills((current) =>
          current.map((entry) => (entry.name === updated.name ? updated : entry)),
        );
      } catch (error) {
        setMessage(
          error instanceof Error
            ? error.message
            : `Unable to update ${skill.name}.`,
        );
      } finally {
        setBusyKey(null);
      }
    },
    [client],
  );

  return (
    <ScrollView
      contentContainerStyle={styles.content}
      showsVerticalScrollIndicator={false}
    >
      <View style={styles.heading}>
        <Text style={[styles.title, { color: t.foreground }]}>Connections</Text>
        {loading ? <ActivityIndicator size="small" color={t.mutedForeground} /> : null}
      </View>

      {message ? <Text style={[styles.message, { color: t.error }]}>{message}</Text> : null}

      <ConnectionSection
        title="MCP servers"
        summary={`${connectedCount} of ${servers.length} on`}
        expanded={expanded.mcp}
        onToggle={() => toggleSection("mcp")}
      >
        {servers.length === 0 ? (
          <EmptyRow label="No MCP servers configured" />
        ) : (
          servers.map((server) => {
            const key = `mcp:${server.name}`;
            return (
              <ToggleRow
                key={server.name}
                title={server.name}
                detail={
                  server.error ??
                  `${server.tool_count} tool${server.tool_count === 1 ? "" : "s"}`
                }
                value={server.connected}
                disabled={busyKey === key}
                onChange={() => void toggleMcp(server)}
              />
            );
          })
        )}
      </ConnectionSection>

      <ConnectionSection
        title="Skills"
        summary={`${enabledSkillCount} of ${skills.length} on`}
        expanded={expanded.skills}
        onToggle={() => toggleSection("skills")}
      >
        {skills.length === 0 ? (
          <EmptyRow label="No skills installed" />
        ) : (
          skills.map((skill) => {
            const key = `skill:${skill.name}`;
            return (
              <ToggleRow
                key={skill.name}
                title={skill.name}
                detail={skill.description}
                value={skill.enabled}
                disabled={busyKey === key}
                onChange={() => void toggleSkill(skill)}
              />
            );
          })
        )}
      </ConnectionSection>
    </ScrollView>
  );

  function ConnectionSection({
    title,
    summary,
    expanded: isExpanded,
    onToggle,
    children,
  }: {
    title: string;
    summary: string;
    expanded: boolean;
    onToggle: () => void;
    children: ReactNode;
  }) {
    return (
      <View style={[styles.section, { borderColor: t.border }]}>
        <Pressable
          accessibilityRole="button"
          accessibilityState={{ expanded: isExpanded }}
          onPress={onToggle}
          style={styles.sectionHeader}
        >
          {isExpanded ? (
            <ChevronDown size={17} color={t.mutedForeground} />
          ) : (
            <ChevronRight size={17} color={t.mutedForeground} />
          )}
          <Text style={[styles.sectionTitle, { color: t.foreground }]}>{title}</Text>
          <Text style={[styles.sectionSummary, { color: t.mutedForeground }]}>
            {summary}
          </Text>
        </Pressable>
        {isExpanded ? children : null}
      </View>
    );
  }

  function ToggleRow({
    title,
    detail,
    value,
    disabled,
    onChange,
  }: {
    title: string;
    detail: string;
    value: boolean;
    disabled: boolean;
    onChange: () => void;
  }) {
    return (
      <View style={[styles.itemRow, { borderTopColor: t.border }]}>
        <View style={styles.itemCopy}>
          <Text style={[styles.itemTitle, { color: t.foreground }]}>{title}</Text>
          <Text
            numberOfLines={2}
            style={[styles.itemDetail, { color: t.mutedForeground }]}
          >
            {detail}
          </Text>
        </View>
        <Switch
          accessibilityLabel={`${value ? "Disable" : "Enable"} ${title}`}
          value={value}
          disabled={disabled}
          onValueChange={onChange}
          trackColor={{ false: t.muted, true: `${t.userMessage}88` }}
          thumbColor={value ? t.userMessage : t.mutedForeground}
        />
      </View>
    );
  }

  function EmptyRow({ label }: { label: string }) {
    return (
      <Text
        style={[styles.empty, { borderTopColor: t.border, color: t.mutedForeground }]}
      >
        {label}
      </Text>
    );
  }
}

const styles = StyleSheet.create({
  content: {
    padding: 18,
    paddingBottom: 32,
    gap: 12,
  },
  heading: {
    minHeight: 30,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  title: {
    fontSize: 19,
    fontWeight: "700",
  },
  message: {
    fontSize: 12,
    lineHeight: 18,
  },
  section: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 14,
    overflow: "hidden",
  },
  sectionHeader: {
    minHeight: 54,
    flexDirection: "row",
    alignItems: "center",
    gap: 9,
    paddingHorizontal: 13,
  },
  sectionTitle: {
    flex: 1,
    fontSize: 14,
    fontWeight: "700",
  },
  sectionSummary: {
    fontSize: 12,
    fontWeight: "600",
  },
  itemRow: {
    minHeight: 62,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingHorizontal: 14,
    paddingVertical: 10,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  itemCopy: {
    flex: 1,
    minWidth: 0,
  },
  itemTitle: {
    fontSize: 13,
    fontWeight: "600",
  },
  itemDetail: {
    marginTop: 3,
    fontSize: 11,
    lineHeight: 16,
  },
  empty: {
    paddingHorizontal: 14,
    paddingVertical: 16,
    borderTopWidth: StyleSheet.hairlineWidth,
    fontSize: 12,
  },
});
