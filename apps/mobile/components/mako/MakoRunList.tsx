import { Pressable, StyleSheet, Text, View } from "react-native";
import * as Haptics from "../../platform/haptics";
import { useThemeContext } from "../../hooks/useTheme";
import type { MakoCurrentRunSummary } from "@krusty/api";
import {
  canPauseRun,
  canResumeRun,
  describeRun,
  formatRunMeta,
  getRunResumeLabel,
  getRunDisplayStatus,
} from "./utils";
import { MakoStatusBadge } from "./MakoStatusBadge";

interface MakoRunListProps {
  runs: MakoCurrentRunSummary[];
  emptyLabel: string;
  onSelectRun: (runId: string) => void;
  detailOverride?: (run: MakoCurrentRunSummary) => string;
  activeActionRunId?: string | null;
  onPauseRun?: (runId: string) => void;
  onResumeRun?: (runId: string) => void;
}

export function MakoRunList({
  runs,
  emptyLabel,
  onSelectRun,
  detailOverride,
  activeActionRunId,
  onPauseRun,
  onResumeRun,
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
      {runs.map((run) => {
        const isActionBusy = activeActionRunId === run.session_id;
        const showPause = Boolean(onPauseRun) && canPauseRun(run);
        const showResume = Boolean(onResumeRun) && canResumeRun(run);

        return (
          <View
            key={run.session_id}
            style={[
              styles.rowBlock,
              {
                borderColor: t.border,
              },
            ]}
          >
            <Pressable
              onPress={() => {
                void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                onSelectRun(run.session_id);
              }}
            >
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
                {detailOverride ? detailOverride(run) : describeRun(run)}
              </Text>
            </Pressable>

            {showPause || showResume ? (
              <View style={styles.actions}>
                {showPause ? (
                  <Pressable
                    disabled={isActionBusy}
                    onPress={() => {
                      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                      onPauseRun?.(run.session_id);
                    }}
                    style={[
                      styles.secondaryAction,
                      {
                        opacity: isActionBusy ? 0.6 : 1,
                      },
                    ]}
                  >
                    <Text
                      style={[styles.secondaryLabel, { color: t.mutedForeground }]}
                    >
                      Pause
                    </Text>
                  </Pressable>
                ) : null}

                {showResume ? (
                  <Pressable
                    disabled={isActionBusy}
                    onPress={() => {
                      void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light);
                      onResumeRun?.(run.session_id);
                    }}
                    style={[
                      styles.primaryAction,
                      {
                        opacity: isActionBusy ? 0.6 : 1,
                      },
                    ]}
                  >
                    <Text style={[styles.primaryLabel, { color: t.userMessage }]}>
                      {isActionBusy ? "Working..." : getRunResumeLabel(run)}
                    </Text>
                  </Pressable>
                ) : null}
              </View>
            ) : null}
          </View>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  list: {
    gap: 0,
  },
  rowBlock: {
    borderBottomWidth: StyleSheet.hairlineWidth,
    paddingVertical: 12,
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
    fontSize: 14,
    fontWeight: "600",
  },
  meta: {
    marginTop: 3,
    fontSize: 12,
    fontWeight: "400",
  },
  summary: {
    marginTop: 8,
    fontSize: 13,
    lineHeight: 18,
  },
  actions: {
    marginTop: 8,
    flexDirection: "row",
    gap: 14,
  },
  primaryAction: {
    minHeight: 24,
    justifyContent: "center",
  },
  primaryLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  secondaryAction: {
    minHeight: 24,
    justifyContent: "center",
  },
  secondaryLabel: {
    fontSize: 12,
    fontWeight: "600",
  },
  empty: {
    fontSize: 14,
    paddingHorizontal: 4,
  },
});
