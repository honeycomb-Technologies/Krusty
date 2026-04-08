import { useCallback, useEffect, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import * as Haptics from "../../platform/haptics";
import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";
import { GlassCard } from "../ui/GlassCard";
import { formatRelativeTime } from "./utils";
import type { ReportSummary } from "@krusty/api";

export function MakoReportsView() {
  const { client, isConnected } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [reports, setReports] = useState<ReportSummary[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isRefreshing, setIsRefreshing] = useState(false);

  const loadReports = useCallback(async () => {
    if (!client || !isConnected) {
      setReports([]);
      setIsLoading(false);
      return;
    }

    try {
      const response = await client.getReports();
      setReports(response.reports);
    } catch {
      setReports([]);
    } finally {
      setIsLoading(false);
      setIsRefreshing(false);
    }
  }, [client, isConnected]);

  useEffect(() => {
    void loadReports();
  }, [loadReports]);

  if (isLoading) {
    return (
      <View style={styles.loading}>
        <ActivityIndicator color={t.userMessage} />
      </View>
    );
  }

  return (
    <ScrollView
      style={styles.scroll}
      contentContainerStyle={styles.content}
      refreshControl={
        <RefreshControl
          refreshing={isRefreshing}
          onRefresh={() => {
            setIsRefreshing(true);
            void loadReports();
          }}
          tintColor={t.userMessage}
        />
      }
      showsVerticalScrollIndicator={false}
    >
      <Text style={[styles.description, { color: t.mutedForeground }]}>
        Reports stay plain on purpose. This is the long-memory surface for what Mako has already written down.
      </Text>

      {reports.length === 0 ? (
        <Text style={[styles.empty, { color: t.mutedForeground }]}>
          No reports yet.
        </Text>
      ) : (
        reports.map((report) => (
          <Pressable
            key={report.id}
            onPress={() => {
              void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            }}
          >
            <GlassCard style={styles.card}>
              <Text style={[styles.title, { color: t.foreground }]} numberOfLines={2}>
                {report.title}
              </Text>
              <Text
                style={[styles.summary, { color: t.mutedForeground }]}
                numberOfLines={3}
              >
                {report.summary || "No summary available."}
              </Text>
              <View style={styles.footer}>
                <Text style={[styles.date, { color: t.mutedForeground }]}>
                  {formatRelativeTime(report.created_at)}
                </Text>
                <Text style={[styles.tags, { color: t.userMessage }]}>
                  {report.tags.slice(0, 3).join(" • ")}
                </Text>
              </View>
            </GlassCard>
          </Pressable>
        ))
      )}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  scroll: {
    flex: 1,
  },
  content: {
    paddingHorizontal: 16,
    paddingBottom: 28,
    gap: 12,
  },
  loading: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
  },
  description: {
    fontSize: 13,
    lineHeight: 18,
  },
  empty: {
    fontSize: 14,
  },
  card: {
    marginBottom: 0,
  },
  title: {
    fontSize: 16,
    fontWeight: "700",
    lineHeight: 22,
  },
  summary: {
    marginTop: 10,
    fontSize: 13,
    lineHeight: 18,
  },
  footer: {
    marginTop: 14,
    flexDirection: "row",
    justifyContent: "space-between",
    gap: 12,
  },
  date: {
    fontSize: 12,
    fontWeight: "500",
  },
  tags: {
    flex: 1,
    textAlign: "right",
    fontSize: 12,
    fontWeight: "600",
  },
});
