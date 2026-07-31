import { useCallback, useEffect, useState } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type { GitStatusResponse } from "@krusty/api";

import { useConnection } from "../../hooks/useConnection";
import { useThemeContext } from "../../hooks/useTheme";

interface ToolboxChangesProps {
  visible: boolean;
  projectDirectory?: string | null;
}

export function ToolboxChanges({
  visible,
  projectDirectory,
}: ToolboxChangesProps) {
  const { client } = useConnection();
  const { theme } = useThemeContext();
  const t = theme.colors;
  const [status, setStatus] = useState<GitStatusResponse | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!client || !visible) {
      return;
    }
    setLoading(true);
    setError(null);
    try {
      setStatus(await client.getGitStatus(projectDirectory ?? undefined));
    } catch (nextError) {
      setError(
        nextError instanceof Error
          ? nextError.message
          : "Unable to load repository changes.",
      );
    } finally {
      setLoading(false);
    }
  }, [client, projectDirectory, visible]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return (
    <ScrollView
      contentContainerStyle={styles.content}
      showsVerticalScrollIndicator={false}
    >
      <View style={styles.headingRow}>
        <View style={styles.headingCopy}>
          <Text style={[styles.title, { color: t.foreground }]}>Changes</Text>
          <Text
            numberOfLines={2}
            style={[styles.subtitle, { color: t.mutedForeground }]}
          >
            {status?.repo_root ?? projectDirectory ?? "No active project"}
          </Text>
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Refresh changes"
          onPress={() => void refresh()}
          style={[styles.refreshButton, { borderColor: t.border }]}
        >
          <Text style={[styles.refreshText, { color: t.foreground }]}>
            Refresh
          </Text>
        </Pressable>
      </View>

      {loading && !status ? (
        <ActivityIndicator color={t.mutedForeground} />
      ) : null}
      {error ? <Text style={[styles.error, { color: t.error }]}>{error}</Text> : null}
      {status && !status.in_repo ? (
        <Text style={[styles.empty, { color: t.mutedForeground }]}>
          The active directory is not a Git repository.
        </Text>
      ) : null}
      {status?.in_repo ? (
        <>
          <View
            style={[
              styles.branchCard,
              { borderColor: t.border, backgroundColor: t.card },
            ]}
          >
            <Text style={[styles.branch, { color: t.foreground }]}>
              {status.branch ?? "Detached HEAD"}
            </Text>
            <Text style={[styles.detail, { color: t.mutedForeground }]}>
              {status.ahead} ahead · {status.behind} behind
            </Text>
          </View>

          <View style={styles.grid}>
            {[
              ["Modified", status.modified],
              ["Staged", status.staged],
              ["Untracked", status.untracked],
              ["Conflicts", status.conflicted],
              ["Additions", status.branch_additions],
              ["Deletions", status.branch_deletions],
            ].map(([label, value]) => (
              <View
                key={label}
                style={[
                  styles.metric,
                  { borderColor: t.border, backgroundColor: t.card },
                ]}
              >
                <Text style={[styles.metricValue, { color: t.foreground }]}>
                  {value}
                </Text>
                <Text style={[styles.metricLabel, { color: t.mutedForeground }]}>
                  {label}
                </Text>
              </View>
            ))}
          </View>
        </>
      ) : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create({
  content: {
    padding: 18,
    paddingBottom: 32,
    gap: 16,
  },
  headingRow: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  headingCopy: {
    flex: 1,
  },
  title: {
    fontSize: 19,
    fontWeight: "700",
  },
  subtitle: {
    marginTop: 4,
    fontSize: 12,
    lineHeight: 17,
  },
  refreshButton: {
    minHeight: 36,
    paddingHorizontal: 12,
    borderRadius: 12,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  refreshText: {
    fontSize: 12,
    fontWeight: "600",
  },
  branchCard: {
    padding: 14,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
  },
  branch: {
    fontSize: 15,
    fontWeight: "700",
  },
  detail: {
    marginTop: 4,
    fontSize: 12,
  },
  grid: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 10,
  },
  metric: {
    width: "47%",
    minHeight: 88,
    borderRadius: 14,
    borderWidth: StyleSheet.hairlineWidth,
    padding: 14,
    justifyContent: "space-between",
  },
  metricValue: {
    fontSize: 23,
    fontWeight: "700",
  },
  metricLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  empty: {
    fontSize: 14,
    lineHeight: 20,
  },
  error: {
    fontSize: 13,
    lineHeight: 19,
  },
});
