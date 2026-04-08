import { Pressable, StyleSheet, Text, View } from "react-native";
import * as Haptics from "../../platform/haptics";
import { GlassCard } from "../ui/GlassCard";
import { useThemeContext } from "../../hooks/useTheme";
import type { MakoCurrentRunSummary } from "@krusty/api";
import {
  describeRun,
  formatRunMeta,
  getRunDisplayStatus,
} from "./utils";
import { MakoStatusBadge } from "./MakoStatusBadge";

interface MakoRunListProps {
  runs: MakoCurrentRunSummary[];
  emptyLabel: string;
  onSelectRun: (runId: string) => void;
}

export function MakoRunList({
  runs,
  emptyLabel,
  onSelectRun,
}: MakoRunListProps) {
  const { theme } = useThemeContext();
  const t = theme.colors;

  if (runs.length === 0) {
    return (
      <Text style={[styles.empty, { color: t.mutedForeground }]}>{emptyLabel}</Text>
    );
  }

  return (
    <View style={styles.list}>
      {runs.map((run) => (
        <Pressable
          key={run.session_id}
          onPress={() => {
            void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
            onSelectRun(run.session_id);
          }}
        >
          <GlassCard style={styles.card}>
            <View style={styles.row}>
              <View style={styles.copy}>
                <Text
                  style={[styles.title, { color: t.foreground }]}
                  numberOfLines={1}
                >
                  {run.title || "Untitled run"}
                </Text>
                <Text
                  style={[styles.meta, { color: t.mutedForeground }]}
                  numberOfLines={1}
                >
                  {formatRunMeta(run)}
                </Text>
              </View>
              <MakoStatusBadge status={getRunDisplayStatus(run)} />
            </View>

            <Text
              style={[styles.summary, { color: t.mutedForeground }]}
              numberOfLines={2}
            >
              {describeRun(run)}
            </Text>
          </GlassCard>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  list: {
    gap: 10,
  },
  card: {
    marginBottom: 0,
  },
  row: {
    flexDirection: "row",
    alignItems: "flex-start",
    gap: 12,
  },
  copy: {
    flex: 1,
    minWidth: 0,
  },
  title: {
    fontSize: 16,
    fontWeight: "700",
  },
  meta: {
    marginTop: 4,
    fontSize: 12,
    fontWeight: "500",
  },
  summary: {
    marginTop: 12,
    fontSize: 13,
    lineHeight: 18,
  },
  empty: {
    fontSize: 14,
    paddingHorizontal: 4,
  },
});
