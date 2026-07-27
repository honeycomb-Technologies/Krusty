import { useEffect, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type {
  McpServerResponse,
  ProviderStatus,
  SkillInfo,
} from "@krusty/api";

import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";

interface ToolboxConnectionsProps {
  visible: boolean;
  onOpenSettings?: () => void;
}

export function ToolboxConnections({
  visible,
  onOpenSettings,
}: ToolboxConnectionsProps) {
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [providers, setProviders] = useState<ProviderStatus[]>([]);
  const [servers, setServers] = useState<McpServerResponse[]>([]);
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!client || !visible) {
      return;
    }
    let active = true;
    setLoading(true);
    void Promise.all([
      client.getCredentials().catch(() => []),
      client.getMcpServers().catch(() => []),
      client.getSkills().catch(() => []),
    ]).then(([nextProviders, nextServers, nextSkills]) => {
      if (!active) {
        return;
      }
      setProviders(nextProviders);
      setServers(nextServers);
      setSkills(nextSkills);
      setLoading(false);
    });
    return () => {
      active = false;
    };
  }, [client, visible]);

  const configuredProviders = providers.filter(
    (provider) => provider.configured || provider.has_oauth,
  );
  const connectedServers = servers.filter(
    (server) => server.status === "connected",
  );

  return (
    <ScrollView
      contentContainerStyle={styles.content}
      showsVerticalScrollIndicator={false}
    >
      <Text style={[styles.title, { color: t.foreground }]}>Connections</Text>
      <Text style={[styles.subtitle, { color: t.mutedForeground }]}>
        Providers, MCP servers, and skills available to Chat.
      </Text>

      {loading ? <ActivityIndicator color={t.mutedForeground} /> : null}

      {[
        {
          title: "Providers",
          summary: `${configuredProviders.length} configured`,
          values: configuredProviders.map((provider) => provider.name),
        },
        {
          title: "MCP",
          summary: `${connectedServers.length} connected`,
          values: connectedServers.map((server) => server.name),
        },
        {
          title: "Skills",
          summary: `${skills.length} installed`,
          values: skills.map((skill) => skill.name),
        },
      ].map((section) => (
        <View
          key={section.title}
          style={[
            styles.card,
            { borderColor: t.border, backgroundColor: t.card },
          ]}
        >
          <View style={styles.cardHeader}>
            <Text style={[styles.cardTitle, { color: t.foreground }]}>
              {section.title}
            </Text>
            <Text style={[styles.cardSummary, { color: t.mutedForeground }]}>
              {section.summary}
            </Text>
          </View>
          <Text style={[styles.values, { color: t.mutedForeground }]}>
            {section.values.slice(0, 6).join(" · ") || "None available"}
          </Text>
        </View>
      ))}

      {onOpenSettings ? (
        <Pressable
          accessibilityRole="button"
          onPress={onOpenSettings}
          style={[styles.settingsButton, { borderColor: t.border }]}
        >
          <Text style={[styles.settingsText, { color: t.foreground }]}>
            Manage connections
          </Text>
        </Pressable>
      ) : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: {
    padding: 18,
    paddingBottom: 32,
    gap: 12,
  },
  title: {
    fontSize: 19,
    fontWeight: "700",
  },
  subtitle: {
    marginTop: -6,
    marginBottom: 4,
    fontSize: 13,
    lineHeight: 19,
  },
  card: {
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: 14,
    padding: 14,
    gap: 8,
  },
  cardHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  cardTitle: {
    fontSize: 14,
    fontWeight: "700",
  },
  cardSummary: {
    fontSize: 12,
    fontWeight: "600",
  },
  values: {
    fontSize: 12,
    lineHeight: 18,
  },
  settingsButton: {
    minHeight: 46,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
    marginTop: 4,
  },
  settingsText: {
    fontSize: 13,
    fontWeight: "700",
  },
});
